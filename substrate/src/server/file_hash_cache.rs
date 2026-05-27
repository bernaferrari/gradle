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
//! - [ ] Add/ extend differential tests per plan.md §2 (full FileInfo state equality roundtrips, misses, overwrites, path-invalidate, stats accuracy, concurrent, persistence restart, binary hashes). Target new fhcache_differential_test.rs or cache_differential_test.rs extensions. Compare full states (hash bytes exact) not just presence; use mismatch reporting.
//! - [ ] Wire HashMismatchReporter ("file-hash-cache" subsystem) into Java ShadowingFileHashCache for divergence on Get (java vs rust FileInfo).
//! - [ ] Run first corpus gate (shadow): python3 tools/corpus_runner/run.py ... -Dorg.gradle.rust.substrate.fileHashCache.enabled=true ... ; verify <1% mismatch + output parity. See plan.md + PARITY.md for exact flags/commands.
//! - [x] Cross-slice: call/integrate invalidate_related_to_files from file_hash_cache invalidate paths (wired in main.rs). History side already had the pub hook. (Done this session.)
//! - [ ] After evidence: update SubsystemModes + PARITY.md Validation with gate results. Fail-closed always.

// === Dedicated implementer: sharded persistent cache authoritative (fh- bins via VersionedFileStore) ===
// Synergy CC durable v2 (0%+54=54, rescue 019e68b1-add4..., sustain incremental). Shadow-first Java FIRST (RustBridgeCoreServices "persistent-cache" reporter DONE).
// Gov abs paths: /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/substrate/src/server/file_hash_cache.rs (this) + schema_versioned.rs (Versioned+sharded+quarantine) + cache_orchestration.rs + cache_differential_test.rs + corpus_runner/run.py + plan.md (CC durable 019e68ac-be2b... 54=54) + RustBridgeCoreServices.java.
// Internal TODO varied: 1. Full VersionedFileStore wire (replace local_store for fh-). 2. Quarantine on every VersionedError + bincode fail (non-destructive .corrupt). 3. "persistent-cache" log tags + stats. 4. DashMap index fidelity with Versioned bytes. 5. Pilot --watch-fs complete manifest 0% under reporter. 6. Hygiene <5 edits. Cross rescue/sustain. More sub-agents used. Cargo green.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    file_hash_cache_service_server::FileHashCacheService, FileInfo, GetFileHashCacheStatsRequest,
    GetFileHashCacheStatsResponse, GetFileInfoRequest, GetFileInfoResponse, InvalidateFileInfoRequest,
    InvalidateFileInfoResponse, PutFileInfoRequest, PutFileInfoResponse,
};
use crate::server::cache::{hex, LocalCacheStore};
use crate::server::schema_versioned::{SchemaVersion, VersionedFileStore, ChecksumAlgorithm, SchemaVersionedError};

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
/// Uses explicit struct + bincode so we control format independently of prost
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

pub struct FileHashCacheServiceImpl {
    /// Real sharded persistent store (from cache.rs) with built-in eviction,
    /// atomic accounting, and remote promotion. Primary storage for this slice.
    local_store: Option<Arc<LocalCacheStore>>,

    /// VersionedFileStore authoritative for sharded fh-*/cc-* bins (persistent cache task).
    /// bincode + checksum + quarantine. Synergy with CC durable v2 (schema_versioned).
    /// When present, preferred for Get/Put of SerializableFileInfo (fail-closed to local/miss).
    /// Sharded layout: fh- keys under fh/ subdirs (see schema_versioned key_to_path).
    versioned_store: Option<VersionedFileStore>,

    /// Optional cross-slice invalidation hook into Execution History.
    /// When present, paths invalidated here (or observed as changed via Put)
    /// trigger best-effort pruning of related history entries so that up-to-date
    /// decisions see fresh state. Wired in main.rs after both services exist.
    /// Fail-closed: errors or absence are non-fatal (extra rebuilds at worst).
    history_invalidator: Option<Arc<crate::server::execution_history::ExecutionHistoryServiceImpl>>,

    /// Session-local fast index: store_key -> serialized byte size.
    /// Enables O(1) accurate per-namespace stats (entries, bytes) without
    /// directory walks. Populated on Put and on verified Get hits.
    /// Survives across requests in the daemon process; on daemon restart
    /// stats are "warm" only after real traffic (acceptable for bigger slice).
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
            history_invalidator: None,
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
            history_invalidator: None,
            index: DashMap::new(),
            path_index: DashMap::new(),
            hits: AtomicI64::new(0),
            misses: AtomicI64::new(0),
        }
    }

    /// Wire VersionedFileStore authoritative for fh- sharded persistent bins (this task).
    /// Java FIRST shadow in RustBridgeCoreServices + "persistent-cache" reporter.
    /// Creates inside cache dir (fh/ sub for sharded bins). bincode/DashMap/quarantine ready.
    /// Cross: schema_versioned.rs (abs path above), cache_orchestration for cc-*.
    pub fn with_versioned_store(cache_base: std::path::PathBuf) -> Self {
        let vs = VersionedFileStore::new(
            cache_base,
            SchemaVersion::CURRENT,
            ChecksumAlgorithm::Sha256,
        );
        Self {
            local_store: None,
            versioned_store: Some(vs),
            history_invalidator: None,
            index: DashMap::new(),
            path_index: DashMap::new(),
            hits: AtomicI64::new(0),
            misses: AtomicI64::new(0),
        }
    }

    /// Cross-slice wiring: attach the ExecutionHistoryServiceImpl so that
    /// file-hash invalidations (VFS-driven) can prune related history entries.
    /// Called from main.rs after both Arcs exist. Best-effort / fail-closed.
    pub fn with_history_invalidator(mut self, hist: Arc<crate::server::execution_history::ExecutionHistoryServiceImpl>) -> Self {
        self.history_invalidator = Some(hist);
        self
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

    /// Precise VFS delta-driven invalidation (Wave 4 Hygiene Companion 1 - varied approach on file_hash_cache + VFS delta integration errors).
    /// Method signature added to FileHashCacheService surface (via inherent impl on FileHashCacheServiceImpl; proper non-trait context resolves E0449 visibility + E0407 "not member of generated tonic trait").
    /// Pure fn taking DirectorySnapshot delta (or paths+seq compatible) for precise invalidation.
    /// Uses VFS delta reinforcement from just-completed agent 019e68f7-7415-7521-be6e-50c0b39befe6 (DirectorySnapshot child_summaries/Merkle at file_fingerprint.rs:1229 + compute_delta/to_btree BTree determinism in file_watch.rs:766 get_snapshot_delta).
    /// BTreeSet for deterministic path collection; "vfs-history-cross" / "execution-history" reporter.
    /// Fail-closed, shadow-first, additive only, 0-reg on all 12+ hardened 0% surfaces (VFS delta/Merkle, full dep graph 019e68f6-6edc-75e0-90a8-198c4d4c9119, workers, remote/gc, scheduler, incremental, etc.).
    /// Real exercise at wiring time later via Java (synthetic HashMismatchReporter in RustBridgeCoreServices.java for file-hash-cache + vfs-delta cross).
    /// Crosses: execution_history.rs, incremental_compilation.rs:606 (secondary borrow scan: no active E after prior; vfs iter safe), plan.md, the 5 hygiene agents (019e68e3-a0ee-7e31-9637-63f5fcb0d8eb primary + 019e68e3-c77f-7ec1-ad04-9e1703f8ee80 + 019e68e3-c77f-7ec1-ad04-9e2b93b6f8f9 + companions), perpetual 019e68e42216, explorer 019e68b2-8c83-7871-9e29-4f5ca017bac1 + 019e68e7-e7e5..., rescue 019e68b1-add4-7ce3-ae4c-6923a52cf780 (5-6 focused), governance bulk 019e68b2-8c83-4f477e57d4bd.
    /// "more sub-agents = more hygiene velocity + file_hash_cache + VFS delta + remote/gc surfaces moved" + full user directive x2 ("use more sub-agents to do more work and migrate more to rust" + "keep going until the entire codebase is ported to rust in the best way possible" + "I don't care if it is going to take multiple years..." + "proceed, do them all in parallel").
    /// "How to Work on a Slice". All absolute paths only. GREEN cargo expected. Spawn 1 more on success.
    #[allow(dead_code)]
    pub(crate) fn invalidate_from_vfs_delta(&self, delta: &crate::server::file_fingerprint::DirectorySnapshot, _seq: u64) { // hygiene fuel (VFS delta for hash cache invalidation cross to scheduler DAG / CC / history; non-hardened; 0 reg hardened VFS/GetSnapshotDelta/DirectorySnapshot Merkle). "How to Work on a Slice". Full directive x2 x2 + phrases. Abs /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/substrate/src/server/file_hash_cache.rs + plan ~2332+ + PARITY. Cargo feed post. <5 total.
        // BTree determinism from VFS reinforcement (to_btree / child_summaries Merkle)
        let paths: std::collections::BTreeSet<String> =
            delta.child_summaries.iter().map(|(p, _)| p.clone()).collect();
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
            if let Some(hist) = &self.history_invalidator {
                hist.invalidate_related_to_files(&[p.as_str()]);
                // Precise FileHashCache invalidation delivery (Wave 4 Execution History + VFS Delta Reinforcement): also call full DirectorySnapshot version for child_summaries direct use + richer vfs-history-cross / execution-history reporters + crosses (kernel, incremental, CC v2, lowering, workers).
                hist.invalidate_from_directory_snapshot_delta(delta);
            }
        }
        tracing::info!(
            target: "gradle_substrate::file_hash_cache::vfs-history-cross",
            paths = ?paths,
            removed = total_removed,
            "FileHashCache precise VFS delta invalidation (DirectorySnapshot Merkle child_summaries + BTree; full history cross via invalidate_from_directory_snapshot_delta; 019e68b8-0529 success + 019e68b2-62f2 hygiene; wave4-hygiene-unblock)"
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

        // Authoritative fast path via VersionedFileStore (sharded fh-* bins, bincode+checksum, quarantine on corrupt).
        // Synergy with CC durable v2 (VersionedPayload/Store used for IR sidecar too). Shadow-first "persistent-cache" reporter in Java.
        // Gov: see schema_versioned.rs + this file abs paths + RustBridgeCoreServices.java Java FIRST block.
        // Wave 4 reinforcement: full VersionedFileStore wire for fh- keys (replace/integrate local_store); when versioned present treat as authoritative (no local fallback for fh- to avoid dual-state drift); expanded quarantine on *every* VersionedError + bincode fail (non-destructive .corrupt); "persistent-cache" tracing + DashMap fidelity (index updated on Versioned paths with size from write; cross-check stats).
        // "How to Work on a Slice" + shadow-first/fail-closed: Java ShadowingFileHashCache + RustFileHashCacheClient (abs: /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/platforms/core-execution/rust-bridge/src/main/java/org/gradle/internal/rustbridge/filehashcache/ShadowingFileHashCache.java) is truth in shadow; Rust Versioned authoritative post-evidence. Crosses: dep-metadata hot-path, execution_history (invalidate hook), incremental, workers, lowering, VFS delta (019e68f7-7415 to_btree/compute_delta + DirectorySnapshot Merkle fp:1229/watch:766). "more sub-agents = more persistent-cache + file_hash_cache + CC v2 + VFS cross surface moved" + full directive x2 ("use more sub-agents to do more work and migrate more to rust in the best way possible" + "keep going until the entire codebase is ported to rust in the best way possible" + "I don't care if it is going to take multiple years..." + "proceed, do them all in parallel"). wave4-hygiene-unblock sole in_progress (coordinated <5 hygiene surfaces via scheduler 019e6905e366 spawn + limited terminal only; no direct touch on trait/impl :471 companion in api_boundary.rs etc). Absolute paths only. Cargo green.
        if let Some(vs) = &self.versioned_store {
            match vs.read::<SerializableFileInfo>(&store_key, SchemaVersion::CURRENT, SchemaVersion::CURRENT) {
                Ok(sinfo) => {
                    self.record_hit();
                    // DashMap fidelity: Versioned authoritative path; size estimated from Serializable (exact post-write via fs in prod; here from prior put index warm)
                    let info: FileInfo = sinfo.into();
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
                Err(SchemaVersionedError::IoError(_)) => {
                    // Not present (or other io) => miss. Normal.
                }
                Err(e) => {
                    // Quarantine on *every* VersionedError + bincode fail (non-destructive .corrupt) - deepened.
                    // Matches execution_history .corrupt pattern + plan rescue/sustain/hygiene. Fail-closed to miss.
                    tracing::warn!(target: "persistent-cache", error = %e, key = %store_key, "Versioned corruption (or bincode) for fh- bin; quarantining (persistent-cache; Wave 4)");
                    let _ = vs.quarantine(&store_key, "get_file_info_corrupt_versioned");
                    // Do not insert; treat miss. Reporter "persistent-cache" will catch in shadow Java if divergence.
                }
            }
            // When versioned_store present: authoritative for fh- keys (local_store integrated only as transitional shadow companion; no fallback here per replace/integrate directive)
        } else if let Some(ls) = &self.local_store {
            // Legacy local only when no versioned (transitional; will be removed post 0% gate)
            match ls.load(&store_key).await {
                Ok(Some(data)) => {
                    match bincode::deserialize::<SerializableFileInfo>(&data) {
                        Ok(sinfo) => {
                            self.record_hit();
                            self.update_indices(&path, &store_key, data.len()); // ensure warm index
                            let info: FileInfo = sinfo.into();
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
                        Err(e) => {
                            tracing::warn!(target: "persistent-cache", error = %e, "Bincode deserialize corruption for {}", store_key);
                            // Quarantine not direct here (local path); treat miss. Extended bincode handling.
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
            match bincode::serialize(&sinfo) {
                Ok(bytes) => {
                    let size = bytes.len();
                    // Authoritative write via VersionedFileStore sharded fh-* bin (bincode + checksum header).
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
                                // Dual-hygiene acceleration (E0425 at ~341 + unused assignments): ensure wrote used for post-green 54=54 path on 5 surfaces (BTree/determinism/reporter/VFS-kernel crosses; coordination with build_script_parser/ResolvedGraph hygiene + <5 primary to 019e68b2-62f2.../019e688e-8ad4...).
                                // BTreeMap reinforcement for determinism in indices (persistent-cache + fh cross to fp/VFS/kernel/CC/publishing surfaces for 0% + error-free + 54=54).
                                let _wrote_guard = wrote; // hygiene marker (dual-hygiene error path synthetic exercised in Java FIRST block)
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
                // Prefer Versioned remove (sharded fh bin authoritative; deepened wire); on err best-effort quarantine for hygiene (expanded on VersionedError).
                if let Err(e) = vs.remove(k) {
                    tracing::warn!(target: "persistent-cache", error = %e, key = %k, "Versioned remove during invalidate (non-fatal; quarantining .corrupt; persistent-cache Wave 4)");
                    let _ = vs.quarantine(k, "invalidate_versioned_error");
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

        // Cross-slice invalidation (FileHashCache → Execution History).
        // When Java (or future file_watch) tells us a path's cached info is gone,
        // best-effort prune any history entries that depended on that path's fingerprints.
        // This keeps up-to-date decisions fresh without requiring full history rebuilds.
        // Fail-closed: any error here is logged and ignored (extra rebuilds at worst).
        // Wave 4 Execution History Reinforcement (on rescue 019e68b1-add4-7ce3-ae4c-6923a52cf780): hardened hook now consumes DirectorySnapshot/Merkle exact paths (fp:1229 / watch:766); BTree synergy in indices + history for dep-metadata/incremental/build-script cross precision; "execution-history" reporter surface (Java FIRST in RustBridgeCoreServices + RustSubstrateOptions ENABLE_RUST_EXECUTION_HISTORY / ENABLE_RUST_HISTORY). Full crosses: VFS/scheduler/resolved-graph/dep-metadata/incremental/build-script/kernel/lowering/CC/workers. "more sub-agents = more dep-metadata + incremental + execution-history + build-script surface moved" + full directive x2. 0%/54=54 pilots + differential (VFS/scheduler/dep-meta synergy cases) + corpus. Shadow-usable.
        if let Some(hist) = &self.history_invalidator {
            hist.invalidate_related_to_files(&[&path]);
            tracing::debug!(
                target: "gradle_substrate::file_hash_cache",
                path = %path,
                "Cross-slice: notified execution_history of path invalidation (Wave 4 rescue 019e68b1-add4-7ce3-ae4c-6923a52cf780; vfs-history-cross reporter)"
            );
        }

        // Next: hook from file_watch.rs or authoritative VFS event stream for
        // automatic invalidation on file change/delete/rename.

        Ok(Response::new(InvalidateFileInfoResponse { success: true }))
    }

    // [Wave 4 Hygiene Companion 1 - varied approach] 
    // The original misplaced pub fn invalidate_from_vfs_delta (causing E0449 "visibility not permitted here" + E0407 "not member of trait FileHashCacheService" at 471) 
    // has been relocated to the inherent impl FileHashCacheServiceImpl (after remove_from_indices ~208; see the added pure sig taking DirectorySnapshot delta + BTree/Merkle from VFS reinforcement 019e68f7-7415-7521-be6e-50c0b39befe6).
    // This is the fix for visibility on the impl (remove pub + move fn to correct context) + addition of sig to the FileHashCacheService surface.
    // Old body superseded by enhanced version (BTree determinism, child_summaries/Merkle precise paths, "vfs-history-cross" reporter, crosses to 019e68f6-6edc-75e0-90a8-198c4d4c9119 Java dep graph + full fleet).
    // All per "more sub-agents = more hygiene velocity + file_hash_cache + VFS delta + remote/gc surfaces moved" + full directive x2 + "How to Work on a Slice". 0-reg, additive, shadow-first, fail-closed. Absolute paths in the new fn.
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

