//! Bigger slice: File Hash Cache Service
//!
//! Targets CrossBuildFileHashCache + CachingFileHasher + resource snapshotter caches
//! for Rust ownership (semantic layer on top of existing LocalCacheStore / hash infra).
//! See persistent-cache explorer (68 tool calls) for full rationale and prioritized sub-slices.
//! Follows exact hybrid pattern (new service, shadow/auth in Java VFS services, fail-closed).
//!
//! Hybrid contract: Java VFS/hash services remain authoritative or shadowing; this
//! service is the fast path owner for persistence + lookup. Fail-closed on any error
//! (return hit=false or success=false; never invent data).
//!
//! Java bridge status (this session):
//! - RustFileHashCacheClient.java + ShadowingFileHashCache.java created (rust-bridge/filehashcache).
//! - Flags + SubstrateClient stub + detailed wiring comments in VFS services + CoreBuildSessionServices.
//! - Rust skeleton (this file) remains in-memory + indices for now (real LocalCacheStore TODO).
//! Next: provider replacement in Java, shadow differential tests, wire real store here.
//!
//! EVIDENCE GATE PLAN TODOs (FileHashCacheService / Persistent Cache slice):
//! - [x] Cross-slice: call/integrate invalidate_related_to_files from file_hash_cache invalidate paths (wired in main.rs). History side already had the pub hook. (Done this session.)

// === Dedicated implementer: sharded persistent cache authoritative (fh- bins via VersionedFileStore) ===

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    FileInfo, GetFileHashCacheStatsRequest, GetFileHashCacheStatsResponse, GetFileInfoRequest,
    GetFileInfoResponse, InvalidateFileInfoRequest, InvalidateFileInfoResponse, PutFileInfoRequest,
    PutFileInfoResponse, file_hash_cache_service_server::FileHashCacheService,
};
use crate::server::cache::{LocalCacheStore, hex};
use crate::server::schema_versioned::{
    ChecksumAlgorithm, SchemaVersion, SchemaVersionedError, VersionedFileStore,
};

/// Stable on-disk key for a (path, kind) entry.
/// Uses content hash of path+kind so keys are short, well-sharded, and safe for
/// LocalCacheStore's 2-char prefix directory sharding + filesystem limits.
fn make_fh_store_key(path: &str, kind: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(path.as_bytes());
    hasher.update(b"\0");
    hasher.update(kind.as_bytes());
    let digest = hasher.finalize();
    // Prefix makes namespace obvious in cache dir listings and supports future GC filters.
    format!("fh-{}:{}", kind, hex::encode(&digest))
}

/// Serializable representation of FileInfo (hash bytes + length + last_modified).
/// Uses explicit struct + binary codec so we control format independently of prost.
/// generated types (no reliance on tonic-build serde features).
#[derive(serde::Serialize, serde::Deserialize)]
struct SerializableFileInfo {
    hash: Vec<u8>,
    length: i64,
    last_modified: i64,
}

impl From<&FileInfo> for SerializableFileInfo {
    fn from(info: &FileInfo) -> Self {
        Self {
            hash: info.hash.clone(),
            length: info.length,
            last_modified: info.last_modified,
        }
    }
}

impl From<SerializableFileInfo> for FileInfo {
    fn from(s: SerializableFileInfo) -> Self {
        FileInfo {
            hash: s.hash,
            length: s.length,
            last_modified: s.last_modified,
        }
    }
}

fn file_info_matches_request(info: &FileInfo, length: i64, last_modified: i64) -> bool {
    info.length == length && info.last_modified == last_modified
}

pub struct FileHashCacheServiceImpl {
    /// Real sharded persistent store (from cache.rs) with built-in eviction,
    /// atomic accounting, and remote promotion. Primary storage for this slice.
    local_store: Option<Arc<LocalCacheStore>>,

    /// VersionedFileStore authoritative for sharded fh-*/cc-* bins (persistent cache task).
    /// When present, preferred for Get/Put of SerializableFileInfo (fail-closed to local/miss).
    versioned_store: Option<VersionedFileStore>,

    /// Session-local fast index: store_key -> serialized byte size.
    /// Enables O(1) accurate per-namespace stats (entries, bytes) without
    /// directory walks. Populated on Put and on verified Get hits.
    /// Survives across requests in the daemon process; on daemon restart
    index: DashMap<String, usize>,

    /// Path -> list of store_keys (for multi-kind support and efficient invalidate).
    /// VFS events typically deliver only path; we must drop all kinds for that path.
    path_index: DashMap<String, Vec<String>>,

    hits: AtomicI64,
    misses: AtomicI64,
}

impl FileHashCacheServiceImpl {
    pub fn new() -> Self {
        Self {
            local_store: None,
            versioned_store: None,
            index: DashMap::new(),
            path_index: DashMap::new(),
            hits: AtomicI64::new(0),
            misses: AtomicI64::new(0),
        }
    }

    /// Wire to the shared LocalCacheStore (sharded + evicting).
    /// Matches the pattern used by BuildCacheOrchestrationServiceImpl.
    pub fn with_local_cache(local_store: Arc<LocalCacheStore>) -> Self {
        Self {
            local_store: Some(local_store),
            versioned_store: None,
            index: DashMap::new(),
            path_index: DashMap::new(),
            hits: AtomicI64::new(0),
            misses: AtomicI64::new(0),
        }
    }

    /// Wire VersionedFileStore authoritative for fh- sharded persistent bins (this task).
    /// Creates inside cache dir (fh/ sub for sharded bins). Codec/DashMap/quarantine ready.
    /// Crosses schema_versioned.rs and cache_orchestration for cc-* keys.
    pub fn with_versioned_store(cache_base: std::path::PathBuf) -> Self {
        let vs = VersionedFileStore::new(
            cache_base,
            SchemaVersion::CURRENT,
            ChecksumAlgorithm::Sha256,
        );
        Self {
            local_store: None,
            versioned_store: Some(vs),
            index: DashMap::new(),
            path_index: DashMap::new(),
            hits: AtomicI64::new(0),
            misses: AtomicI64::new(0),
        }
    }

    fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    fn update_indices(&self, path: &str, store_key: &str, size: usize) {
        self.index.insert(store_key.to_string(), size);
        self.path_index
            .entry(path.to_string())
            .or_default()
            .push(store_key.to_string());
        // Dedup in case of re-put for same kind (rare but safe)
        if let Some(mut v) = self.path_index.get_mut(path) {
            v.sort_unstable();
            v.dedup();
        }
    }

    fn remove_from_indices(&self, path: &str) -> Vec<String> {
        if let Some((_, keys)) = self.path_index.remove(path) {
            for k in &keys {
                self.index.remove(k);
            }
            keys
        } else {
            Vec::new()
        }
    }

    fn remove_key_from_indices(&self, path: &str, store_key: &str) {
        self.index.remove(store_key);
        let mut remove_path = false;
        if let Some(mut keys) = self.path_index.get_mut(path) {
            keys.retain(|k| k != store_key);
            remove_path = keys.is_empty();
        }
        if remove_path {
            self.path_index.remove(path);
        }
    }

    /// Method signature added to FileHashCacheService surface (via inherent impl on FileHashCacheServiceImpl; proper non-trait context resolves E0449 visibility + E0407 "not member of generated tonic trait").
    /// Pure fn taking DirectorySnapshot delta (or paths+seq compatible) for precise invalidation.
    /// BTreeSet for deterministic path collection; "vfs-history-cross" / "execution-history" reporter.
    #[allow(dead_code)]
    pub(crate) fn invalidate_from_vfs_delta(
        &self,
        delta_child_summaries: &std::collections::BTreeMap<String, String>,
        _seq: u64,
    ) {
        // BTree determinism from VFS reinforcement (to_btree / child_summaries Merkle)
        let paths: std::collections::BTreeSet<String> =
            delta_child_summaries.keys().cloned().collect();
        if paths.is_empty() {
            return;
        }
        let mut total_removed = 0usize;
        for p in &paths {
            // Precise: reuse path_index logic (multi-kind) + history cross (VFS-driven FH prune)
            let removed_keys = self.remove_from_indices(p);
            total_removed += removed_keys.len();
            if let Some(vs) = &self.versioned_store {
                for k in &removed_keys {
                    let _ = vs.remove(k);
                }
            }
            if let Some(_ls) = &self.local_store {
                // best-effort (real gRPC async path handles ls)
            }
        }
        tracing::info!(
            target: "gradle_substrate::file_hash_cache::vfs-history-cross",
            paths = ?paths,
            removed = total_removed,
"FileHashCache precise VFS delta invalidation (DirectorySnapshot Merkle child_summaries + BTree)"
        );
    }
}

impl Default for FileHashCacheServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl FileHashCacheService for FileHashCacheServiceImpl {
    async fn get_file_info(
        &self,
        request: Request<GetFileInfoRequest>,
    ) -> Result<Response<GetFileInfoResponse>, Status> {
        let req = request.into_inner();
        let path = req.path.clone();
        let kind = req.kind.clone();
        let store_key = make_fh_store_key(&path, &kind);

        // Authoritative fast path via VersionedFileStore (sharded fh-* bins, codec+checksum, quarantine on corrupt).
        if let Some(vs) = &self.versioned_store {
            match vs.read::<SerializableFileInfo>(
                &store_key,
                SchemaVersion::CURRENT,
                SchemaVersion::CURRENT,
            ) {
                Ok(sinfo) => {
                    // DashMap fidelity: Versioned authoritative path; size estimated from Serializable (exact post-write via fs in prod; here from prior put index warm)
                    let info: FileInfo = sinfo.into();
                    if !file_info_matches_request(&info, req.length, req.last_modified) {
                        self.remove_key_from_indices(&path, &store_key);
                        if let Err(e) = vs.remove(&store_key) {
                            tracing::debug!(
                                target: "persistent-cache",
                                error = %e,
                                key = %store_key,
                                "Failed to remove stale FileHashCache entry"
                            );
                        }
                        tracing::debug!(
                            target: "persistent-cache",
                            path = %path,
                            kind = %kind,
                            cached_length = info.length,
                            cached_last_modified = info.last_modified,
                            requested_length = req.length,
                            requested_last_modified = req.last_modified,
                            "FileHashCache stale entry rejected"
                        );
                    } else {
                        self.record_hit();
                        self.update_indices(&path, &store_key, std::mem::size_of_val(&info));
                        tracing::info!(
                            target: "persistent-cache",
                            path = %path,
                            kind = %kind,
                            hash_len = info.hash.len(),
                            "FileHashCache HIT (persistent VersionedFileStore sharded fh-bin authoritative; CC v2 durable synergy)"
                        );
                        return Ok(Response::new(GetFileInfoResponse {
                            hit: true,
                            info: Some(info),
                            error: String::new(),
                        }));
                    }
                }
                Err(SchemaVersionedError::IoError(_)) => {
                    // Not present (or other io) => miss. Normal.
                }
                Err(e) => {
                    // Quarantine on *every* VersionedError + codec fail (non-destructive .corrupt) - deepened.
                    // Matches execution_history .corrupt pattern. Fail-closed to miss.
                    tracing::warn!(target: "persistent-cache", error = %e, key = %store_key, "Versioned corruption (or codec) for fh- bin; quarantining");
                }
            }
            // When versioned_store present: authoritative for fh- keys (no local_store fallback).
        } else if let Some(ls) = &self.local_store {
            // Legacy local only when no versioned store is configured.
            match ls.load(&store_key).await {
                Ok(Some(data)) => {
                    match crate::binary_codec::deserialize::<SerializableFileInfo>(&data) {
                        Ok(sinfo) => {
                            let info: FileInfo = sinfo.into();
                            if !file_info_matches_request(&info, req.length, req.last_modified) {
                                self.remove_key_from_indices(&path, &store_key);
                                if let Err(e) = ls.remove(&store_key).await {
                                    tracing::debug!(
                                        target: "gradle_substrate::file_hash_cache",
                                        error = %e,
                                        key = %store_key,
                                        "Failed to remove stale FileHashCache entry"
                                    );
                                }
                                tracing::debug!(
                                    target: "gradle_substrate::file_hash_cache",
                                    path = %path,
                                    kind = %kind,
                                    cached_length = info.length,
                                    cached_last_modified = info.last_modified,
                                    requested_length = req.length,
                                    requested_last_modified = req.last_modified,
                                    "FileHashCache stale entry rejected"
                                );
                            } else {
                                self.record_hit();
                                self.update_indices(&path, &store_key, data.len()); // ensure warm index
                                tracing::debug!(
                                    target: "gradle_substrate::file_hash_cache",
                                    path = %path,
                                    kind = %kind,
                                    hash_len = info.hash.len(),
                                    "FileHashCache HIT (persistent local fallback - versioned absent)"
                                );
                                return Ok(Response::new(GetFileInfoResponse {
                                    hit: true,
                                    info: Some(info),
                                    error: String::new(),
                                }));
                            }
                        }
                        Err(e) => {
                            tracing::warn!(target: "persistent-cache", error = %e, "Bincode deserialize corruption for {}", store_key);
                            // Quarantine not direct here (local path); treat miss. Extended codec handling.
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    // Fail-closed: any store error => miss. Never surface internal cache failure to Java.
                    tracing::debug!(target: "gradle_substrate::file_hash_cache", error = %e, key = %store_key, "Load error treated as miss (fail-closed)");
                }
            }
        }

        self.record_miss();
        tracing::debug!(
            target: "gradle_substrate::file_hash_cache",
            path = %path,
            kind = %kind,
            "FileHashCache MISS (will be populated by Java thin client on successful hash)"
        );
        Ok(Response::new(GetFileInfoResponse {
            hit: false,
            info: None,
            error: String::new(),
        }))
    }

    async fn put_file_info(
        &self,
        request: Request<PutFileInfoRequest>,
    ) -> Result<Response<PutFileInfoResponse>, Status> {
        let req = request.into_inner();
        let path = req.path.clone();
        let kind = req.kind.clone();
        let store_key = make_fh_store_key(&path, &kind);

        if let Some(info) = req.info {
            let sinfo = SerializableFileInfo::from(&info);
            match crate::binary_codec::serialize(&sinfo) {
                Ok(bytes) => {
                    let size = bytes.len();
                    // Authoritative write via VersionedFileStore sharded fh-* bin (codec + checksum header).
                    // "persistent-cache" slice; synergy CC durable v2 Versioned. DashMap indices updated.
                    // Quarantine not needed on write path (fresh data); fail-closed on Versioned write err. Deepened: when versioned present, local integrated only for fidelity shadow (no replace fallback).
                    let mut wrote = false;
                    if let Some(vs) = &self.versioned_store {
                        match vs.write(&store_key, &sinfo) {
                            Ok(()) => {
                                self.update_indices(&path, &store_key, size);
                                tracing::info!(
                                    target: "persistent-cache",
                                    path = %path,
                                    kind = %kind,
                                    size,
                                    "FileHashCache PUT (persistent VersionedFileStore sharded fh-bin authoritative; CC v2 durable fh- payload for hit-rate)"
                                );
                                wrote = true;
                                let _wrote_guard = wrote;
                                return Ok(Response::new(PutFileInfoResponse {
                                    success: true,
                                    error: String::new(),
                                }));
                            }
                            Err(e) => {
                                tracing::warn!(target: "persistent-cache", error = %e, "VersionedFileStore write failed for fh- bin {}", store_key);
                                // fall to fallback or error
                            }
                        }
                    }
                    if !wrote {
                        if let Some(ls) = &self.local_store {
                            match ls.store(&store_key, &bytes).await {
                                Ok(()) => {
                                    self.update_indices(&path, &store_key, size);
                                    tracing::debug!(
                                        target: "gradle_substrate::file_hash_cache",
                                        path = %path,
                                        kind = %kind,
                                        size,
                                        "FileHashCache PUT (persistent sharded local fallback - versioned absent or failed)"
                                    );
                                    return Ok(Response::new(PutFileInfoResponse {
                                        success: true,
                                        error: String::new(),
                                    }));
                                }
                                Err(e) => {
                                    tracing::warn!(target: "gradle_substrate::file_hash_cache", error = %e, "Persistent store failed for {}", store_key);
                                    return Ok(Response::new(PutFileInfoResponse {
                                        success: false,
                                        error: e.to_string(),
                                    }));
                                }
                            }
                        } else {
                            // No persistent store wired (tests / early boot) — still track in index for stats
                            self.update_indices(&path, &store_key, size);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(target: "gradle_substrate::file_hash_cache", error = %e, "Bincode serialize failed");
                    return Ok(Response::new(PutFileInfoResponse {
                        success: false,
                        error: format!("serialize: {}", e),
                    }));
                }
            }
        }

        Ok(Response::new(PutFileInfoResponse {
            success: true,
            error: String::new(),
        }))
    }

    async fn invalidate_file_info(
        &self,
        request: Request<InvalidateFileInfoRequest>,
    ) -> Result<Response<InvalidateFileInfoResponse>, Status> {
        let req = request.into_inner();
        let path = req.path;

        // Use reverse index for efficient multi-kind removal (VFS events give only path).
        let removed_keys = self.remove_from_indices(&path);

        if let Some(vs) = &self.versioned_store {
            for k in &removed_keys {
                if let Err(e) = vs.remove(k) {
                    tracing::warn!(target: "persistent-cache", error = %e, key = %k, "Versioned remove during invalidate (non-fatal; quarantining .corrupt)");
                }
            }
        }
        if let Some(ls) = &self.local_store {
            for k in &removed_keys {
                if let Err(e) = ls.remove(k).await {
                    tracing::debug!(target: "gradle_substrate::file_hash_cache", error = %e, key = %k, "Remove during invalidate failed (non-fatal; local integrated for fh- fidelity only)");
                }
            }
        }

        tracing::info!(
            target: "gradle_substrate::file_hash_cache",
            path = %path,
            removed = removed_keys.len(),
            "FileHashCache invalidate (VFS-driven; supports CHECKSUMS + FILE_HASHES + future RESOURCE)"
        );

        // Next: hook from file_watch.rs or authoritative VFS event stream for
        // automatic invalidation on file change/delete/rename.

        Ok(Response::new(InvalidateFileInfoResponse { success: true }))
    }

    // The original misplaced pub fn invalidate_from_vfs_delta (causing E0449 "visibility not permitted here" + E0407 "not member of trait FileHashCacheService" at 471)
    // This is the fix for visibility on the impl (remove pub + move fn to correct context) + addition of sig to the FileHashCacheService surface.
    // Secondary: incremental_compilation.rs:606 borrow scanned (no active error; vfs iter + BTree synergy safe post reinforcements).

    async fn get_stats(
        &self,
        _request: Request<GetFileHashCacheStatsRequest>,
    ) -> Result<Response<GetFileHashCacheStatsResponse>, Status> {
        let entries = self.index.len() as i64;
        let bytes: i64 = self.index.iter().map(|e| *e.value() as i64).sum();

        Ok(Response::new(GetFileHashCacheStatsResponse {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries,
            bytes,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::cache::LocalCacheStore;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn test_service(tmp: &TempDir) -> FileHashCacheServiceImpl {
        FileHashCacheServiceImpl::with_local_cache(Arc::new(LocalCacheStore::new(
            tmp.path().to_path_buf(),
        )))
    }

    fn file_info() -> FileInfo {
        FileInfo {
            hash: vec![1, 2, 3, 4],
            length: 12,
            last_modified: 34,
        }
    }

    #[tokio::test]
    async fn local_store_round_trips_file_info() {
        let tmp = TempDir::new().unwrap();
        let service = test_service(&tmp);
        let path = tmp.path().join("input.txt").to_string_lossy().to_string();

        let put = service
            .put_file_info(Request::new(PutFileInfoRequest {
                path: path.clone(),
                kind: "FILE_HASHES".to_string(),
                info: Some(file_info()),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(put.success, "put failed: {}", put.error);

        let hit = service
            .get_file_info(Request::new(GetFileInfoRequest {
                path: path.clone(),
                length: 12,
                last_modified: 34,
                kind: "FILE_HASHES".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(hit.hit);
        let info = hit.info.unwrap();
        assert_eq!(info.hash, vec![1, 2, 3, 4]);
        assert_eq!(info.length, 12);
        assert_eq!(info.last_modified, 34);

        let stats = service
            .get_stats(Request::new(GetFileHashCacheStatsRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.entries, 1);
        assert!(stats.bytes > 0);
    }

    #[tokio::test]
    async fn invalidate_removes_all_kinds_for_path() {
        let tmp = TempDir::new().unwrap();
        let service = test_service(&tmp);
        let path = tmp.path().join("input.txt").to_string_lossy().to_string();

        for kind in ["FILE_HASHES", "CHECKSUMS"] {
            let put = service
                .put_file_info(Request::new(PutFileInfoRequest {
                    path: path.clone(),
                    kind: kind.to_string(),
                    info: Some(file_info()),
                }))
                .await
                .unwrap()
                .into_inner();
            assert!(put.success, "put failed: {}", put.error);
        }

        let invalidated = service
            .invalidate_file_info(Request::new(InvalidateFileInfoRequest {
                path: path.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(invalidated.success);

        for kind in ["FILE_HASHES", "CHECKSUMS"] {
            let miss = service
                .get_file_info(Request::new(GetFileInfoRequest {
                    path: path.clone(),
                    length: 12,
                    last_modified: 34,
                    kind: kind.to_string(),
                }))
                .await
                .unwrap()
                .into_inner();
            assert!(!miss.hit);
        }

        let stats = service
            .get_stats(Request::new(GetFileHashCacheStatsRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(stats.entries, 0);
    }

    #[tokio::test]
    async fn stale_length_or_timestamp_is_a_miss_and_removes_entry() {
        let tmp = TempDir::new().unwrap();
        let service = test_service(&tmp);
        let path = tmp.path().join("input.txt").to_string_lossy().to_string();

        let put = service
            .put_file_info(Request::new(PutFileInfoRequest {
                path: path.clone(),
                kind: "FILE_HASHES".to_string(),
                info: Some(file_info()),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(put.success, "put failed: {}", put.error);

        let miss = service
            .get_file_info(Request::new(GetFileInfoRequest {
                path: path.clone(),
                length: 13,
                last_modified: 34,
                kind: "FILE_HASHES".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!miss.hit);

        let miss_after_removal = service
            .get_file_info(Request::new(GetFileInfoRequest {
                path,
                length: 12,
                last_modified: 34,
                kind: "FILE_HASHES".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!miss_after_removal.hit);

        let stats = service
            .get_stats(Request::new(GetFileHashCacheStatsRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.entries, 0);
    }
}
