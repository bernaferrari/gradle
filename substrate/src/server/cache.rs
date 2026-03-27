use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use tokio::fs;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use crate::error::SubstrateError;
use crate::proto::{
    cache_load_chunk, cache_service_server::CacheService, cache_store_chunk, CacheEntryMetadata,
    CacheLoadChunk, CacheLoadRequest, CacheStoreChunk, CacheStoreResponse,
};

const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks for streaming

/// Local filesystem build cache store.
/// Entries are stored as files named by their cache key (hex string).
struct CacheCounters {
    max_size_bytes: AtomicU64,
    total_bytes: AtomicU64,
    entry_count: AtomicI64,
    hits: AtomicI64,
    misses: AtomicI64,
}

#[derive(Clone)]
pub struct LocalCacheStore {
    base_dir: PathBuf,
    counters: Arc<CacheCounters>,
}

impl LocalCacheStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            counters: Arc::new(CacheCounters {
                max_size_bytes: AtomicU64::new(0),
                total_bytes: AtomicU64::new(0),
                entry_count: AtomicI64::new(0),
                hits: AtomicI64::new(0),
                misses: AtomicI64::new(0),
            }),
        }
    }

    /// Set the maximum cache size in bytes (0 = unlimited).
    pub fn set_max_size(&self, max_bytes: u64) {
        self.counters
            .max_size_bytes
            .store(max_bytes, Ordering::Relaxed);
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        let path = key.trim();
        if path.len() > 2 {
            self.base_dir.join(&path[..2]).join(&path[2..])
        } else {
            self.base_dir.join(path)
        }
    }

    pub async fn contains(&self, key: &str) -> Result<bool, SubstrateError> {
        let path = self.key_to_path(key);
        let found = fs::metadata(&path).await.is_ok();
        if found {
            self.counters.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.counters.misses.fetch_add(1, Ordering::Relaxed);
        }
        Ok(found)
    }

    pub async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, SubstrateError> {
        let path = self.key_to_path(key);
        match fs::read(&path).await {
            Ok(data) => {
                self.counters.hits.fetch_add(1, Ordering::Relaxed);
                Ok(Some(data))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.counters.misses.fetch_add(1, Ordering::Relaxed);
                Ok(None)
            }
            Err(e) => Err(SubstrateError::Cache(format!(
                "Failed to load cache entry {}: {}",
                key, e
            ))),
        }
    }

    pub async fn store(&self, key: &str, data: &[u8]) -> Result<(), SubstrateError> {
        let path = self.key_to_path(key);
        let data_len = data.len() as u64;

        // Check existing entry size for accurate accounting
        let existing_size = match fs::metadata(&path).await {
            Ok(meta) => Some(meta.len()),
            Err(_) => None,
        };

        // Check if we need to evict before storing (account for replacing existing)
        let net_new = if let Some(old) = existing_size {
            data_len.saturating_sub(old)
        } else {
            data_len
        };
        self.maybe_evict(net_new).await;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&path, data).await?;

        if existing_size.is_none() {
            self.counters.entry_count.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(old) = existing_size {
            self.counters.total_bytes.fetch_sub(old, Ordering::Relaxed);
        }
        self.counters
            .total_bytes
            .fetch_add(data_len, Ordering::Relaxed);

        Ok(())
    }

    pub async fn remove(&self, key: &str) -> Result<bool, SubstrateError> {
        let path = self.key_to_path(key);
        match fs::remove_file(&path).await {
            Ok(()) => {
                self.counters.entry_count.fetch_sub(1, Ordering::Relaxed);
                // Note: we can't track per-entry size without metadata, so total_bytes is approximate
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(SubstrateError::Cache(format!(
                "Failed to remove cache entry {}: {}",
                key, e
            ))),
        }
    }

    /// Evict oldest entries if the cache would exceed the max size.
    async fn maybe_evict(&self, incoming_size: u64) {
        let max = self.counters.max_size_bytes.load(Ordering::Relaxed);
        if max == 0 {
            return; // unlimited
        }

        let current = self.counters.total_bytes.load(Ordering::Relaxed);
        if current + incoming_size <= max {
            return; // fits
        }

        // Need to evict: scan directory, sort by modification time, remove oldest
        let entries = self.scan_entries_by_age().await;
        let mut freed: u64 = 0;
        let target = current + incoming_size - max;

        for (path, size) in entries {
            if freed >= target {
                break;
            }
            if fs::remove_file(&path).await.is_ok() {
                freed += size;
                self.counters.entry_count.fetch_sub(1, Ordering::Relaxed);
                self.counters.total_bytes.fetch_sub(size, Ordering::Relaxed);
                tracing::debug!(path = %path.display(), size, "Evicted cache entry");
            }
        }

        if freed > 0 {
            tracing::info!(
                freed_bytes = freed,
                target_bytes = target,
                "Cache eviction completed"
            );
        }
    }

    /// Scan cache directory for entries sorted by modification time (oldest first).
    async fn scan_entries_by_age(&self) -> Vec<(PathBuf, u64)> {
        let mut entries = Vec::new();

        if let Ok(mut dir) = fs::read_dir(&self.base_dir).await {
            while let Ok(Some(entry)) = dir.next_entry().await {
                let file_type = match entry.file_type().await {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if file_type.is_dir() {
                    // Scan shard subdirectories (e.g., "aa/", "bb/")
                    if let Ok(mut sub_dir) = fs::read_dir(entry.path()).await {
                        while let Ok(Some(sub_entry)) = sub_dir.next_entry().await {
                            if let Ok(metadata) = sub_entry.metadata().await {
                                let size = metadata.len();
                                let modified = metadata
                                    .modified()
                                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                                entries.push((sub_entry.path(), size, modified));
                            }
                        }
                    }
                } else if file_type.is_file() {
                    if let Ok(metadata) = entry.metadata().await {
                        let size = metadata.len();
                        let modified = metadata
                            .modified()
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        entries.push((entry.path(), size, modified));
                    }
                }
            }
        }

        entries.sort_unstable_by_key(|e| e.2);
        entries
            .into_iter()
            .map(|(path, size, _)| (path, size))
            .collect()
    }

    /// Get cache statistics.
    pub fn get_stats(&self) -> CacheStats {
        let hits = self.counters.hits.load(Ordering::Relaxed);
        let misses = self.counters.misses.load(Ordering::Relaxed);
        let total_lookups = hits + misses;
        let hit_rate = if total_lookups > 0 {
            hits as f64 / total_lookups as f64
        } else {
            1.0
        };

        CacheStats {
            entry_count: self.counters.entry_count.load(Ordering::Relaxed),
            total_bytes: self.counters.total_bytes.load(Ordering::Relaxed),
            max_bytes: self.counters.max_size_bytes.load(Ordering::Relaxed),
            hits,
            misses,
            hit_rate,
        }
    }
}

pub struct CacheStats {
    pub entry_count: i64,
    pub total_bytes: u64,
    pub max_bytes: u64,
    pub hits: i64,
    pub misses: i64,
    pub hit_rate: f64,
}

pub struct CacheServiceImpl {
    store: Arc<LocalCacheStore>,
    remote: Option<std::sync::Arc<crate::server::remote_cache::RemoteCacheStore>>,
}

impl Default for CacheServiceImpl {
    fn default() -> Self {
        Self::new(std::path::PathBuf::new())
    }
}

impl CacheServiceImpl {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            store: Arc::new(LocalCacheStore::new(base_dir)),
            remote: None,
        }
    }

    pub fn with_remote(
        base_dir: PathBuf,
        remote: crate::server::remote_cache::RemoteCacheStore,
    ) -> Self {
        Self {
            store: Arc::new(LocalCacheStore::new(base_dir)),
            remote: Some(std::sync::Arc::new(remote)),
        }
    }

    /// Get a reference to the local cache store for cross-service integration.
    pub fn local_store(&self) -> Arc<LocalCacheStore> {
        Arc::clone(&self.store)
    }
}

#[tonic::async_trait]
impl CacheService for CacheServiceImpl {
    type LoadEntryStream = ReceiverStream<Result<CacheLoadChunk, Status>>;

    async fn load_entry(
        &self,
        request: Request<CacheLoadRequest>,
    ) -> Result<Response<Self::LoadEntryStream>, Status> {
        let key_bytes = request.into_inner().key;
        let key = hex::encode(&key_bytes);
        let store = self.store.clone();
        let remote = self.remote.clone();

        let (tx, rx) = mpsc::channel(8);

        tokio::spawn(async move {
            let data = if let Some(remote) = &remote {
                // Try remote first
                match remote.load(&key).await {
                    Ok(Some(data)) => {
                        tracing::debug!(key = %key, size = data.len(), "Cache hit (remote)");
                        // Promote to local
                        let _ = store.store(&key, &data).await;
                        Some(data)
                    }
                    Ok(None) => {
                        // Fall through to local
                        match store.load(&key).await {
                            Ok(data) => data,
                            Err(e) => {
                                let _ = tx.send(Err(Status::internal(e.to_string()))).await;
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(key = %key, error = %e, "Remote cache load failed, falling back to local");
                        match store.load(&key).await {
                            Ok(data) => data,
                            Err(err) => {
                                let _ = tx.send(Err(Status::internal(err.to_string()))).await;
                                return;
                            }
                        }
                    }
                }
            } else {
                match store.load(&key).await {
                    Ok(data) => data,
                    Err(e) => {
                        let _ = tx.send(Err(Status::internal(e.to_string()))).await;
                        return;
                    }
                }
            };

            if let Some(data) = data {
                let _ = tx
                    .send(Ok(CacheLoadChunk {
                        payload: Some(cache_load_chunk::Payload::Metadata(CacheEntryMetadata {
                            size: data.len() as i64,
                            content_type: "application/octet-stream".to_string(),
                        })),
                    }))
                    .await;

                for chunk in data.chunks(CHUNK_SIZE) {
                    if tx.is_closed() {
                        break;
                    }
                    let _ = tx
                        .send(Ok(CacheLoadChunk {
                            payload: Some(cache_load_chunk::Payload::Data(chunk.to_vec())),
                        }))
                        .await;
                }
            } else {
                tracing::debug!(key = %key, "Cache miss");
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn store_entry(
        &self,
        request: Request<Streaming<CacheStoreChunk>>,
    ) -> Result<Response<CacheStoreResponse>, Status> {
        let mut stream = request.into_inner();
        let store = self.store.clone();

        let mut key: Option<String> = None;
        let mut data = Vec::new();

        while let Some(chunk) = stream
            .message()
            .await
            .map_err(|e| Status::internal(format!("Failed to read stream chunk: {}", e)))?
        {
            match chunk.payload {
                Some(cache_store_chunk::Payload::Init(init)) => {
                    key = Some(hex::encode(&init.key));
                    let total_size = init.total_size;
                    data.reserve(total_size as usize);
                }
                Some(cache_store_chunk::Payload::Data(bytes)) => {
                    data.extend_from_slice(&bytes);
                }
                None => {
                    return Err(Status::invalid_argument("Missing payload in store chunk"));
                }
            }
        }

        let key = key.ok_or_else(|| Status::invalid_argument("Missing init chunk"))?;

        tracing::debug!(key = %key, size = data.len(), "Storing cache entry");

        // Store locally first (fast)
        let local_result = store.store(&key, &data).await;

        // Store to remote in background (non-blocking)
        if let Some(remote) = &self.remote {
            let remote = remote.clone();
            let key_clone = key.clone();
            let data_clone = data.clone();
            tokio::spawn(async move {
                match remote.store(&key_clone, &data_clone).await {
                    Ok(()) => tracing::debug!(key = %key_clone, "Cache entry stored to remote"),
                    Err(e) => {
                        tracing::warn!(key = %key_clone, error = %e, "Failed to store cache entry to remote")
                    }
                }
            });
        }

        match local_result {
            Ok(()) => Ok(Response::new(CacheStoreResponse {
                success: true,
                error_message: String::new(),
            })),
            Err(e) => Ok(Response::new(CacheStoreResponse {
                success: false,
                error_message: e.to_string(),
            })),
        }
    }
}

pub(crate) mod hex {
    const HEX_TABLE: &[u8; 16] = b"0123456789abcdef";

    /// Encode bytes to lowercase hex string using a lookup table.
    /// Zero per-byte allocations — pre-allocates exactly 2× input length.
    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            s.push(HEX_TABLE[(b >> 4) as usize] as char);
            s.push(HEX_TABLE[(b & 0x0F) as usize] as char);
        }
        s
    }

    /// Decode a hex string into raw bytes. Returns None if the input is not valid hex.
    pub fn decode(hex_str: &str) -> Option<Vec<u8>> {
        if !hex_str.len().is_multiple_of(2) {
            return None;
        }
        (0..hex_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_cache_store_load() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        assert!(!store.contains("aa1122334455").await.unwrap());

        store
            .store("aa1122334455", b"hello cache world")
            .await
            .unwrap();

        assert!(store.contains("aa1122334455").await.unwrap());

        let loaded = store.load("aa1122334455").await.unwrap();
        assert_eq!(loaded, Some(b"hello cache world".to_vec()));
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        let loaded = store.load("ff0000000000").await.unwrap();
        assert_eq!(loaded, None);
    }

    #[tokio::test]
    async fn test_cache_large_entry() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        let data = vec![0xABu8; 200_000];
        store.store("bb1122334455", &data).await.unwrap();

        let loaded = store.load("bb1122334455").await.unwrap();
        assert_eq!(loaded, Some(data));
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        store.store("cc1111111111", b"data1").await.unwrap();
        store.store("cc2222222222", b"data2longer").await.unwrap();

        // Hit
        let _ = store.load("cc1111111111").await.unwrap();
        // Miss
        let _ = store.load("cc9999999999").await.unwrap();

        let stats = store.get_stats();
        assert_eq!(stats.entry_count, 2);
        assert_eq!(stats.total_bytes, 5 + 11);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[tokio::test]
    async fn test_cache_eviction() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());
        store.set_max_size(30); // 30 bytes max

        // Store entries totaling 40 bytes
        store.store("dd1111111111", b"0123456789").await.unwrap(); // 10 bytes
        store.store("dd2222222222", b"0123456789").await.unwrap(); // 10 bytes
        store.store("dd3333333333", b"0123456789").await.unwrap(); // 10 bytes
        store.store("dd4444444444", b"0123456789").await.unwrap(); // 10 bytes - should trigger eviction

        let stats = store.get_stats();
        // After eviction, total should be <= 30 bytes
        assert!(stats.total_bytes <= 30);
        assert!(stats.entry_count <= 3);
    }

    #[tokio::test]
    async fn test_cache_remove() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        store.store("ee1111111111", b"removeme").await.unwrap();
        assert!(store.contains("ee1111111111").await.unwrap());

        let removed = store.remove("ee1111111111").await.unwrap();
        assert!(removed);
        assert!(!store.contains("ee1111111111").await.unwrap());

        // Remove nonexistent
        let removed2 = store.remove("ee9999999999").await.unwrap();
        assert!(!removed2);
    }

    #[tokio::test]
    async fn test_cache_update_existing() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        store.store("ff1111111111", b"old_data").await.unwrap();
        store
            .store("ff1111111111", b"new_data_longer")
            .await
            .unwrap();

        let loaded = store.load("ff1111111111").await.unwrap();
        assert_eq!(loaded, Some(b"new_data_longer".to_vec()));

        // Entry count should still be 1 (update, not insert)
        let stats = store.get_stats();
        assert_eq!(stats.entry_count, 1);
    }

    #[tokio::test]
    async fn test_cache_hit_rate() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        // No lookups yet → hit_rate should be 1.0 (defined as 100% when no lookups)
        let stats = store.get_stats();
        assert!((stats.hit_rate - 1.0).abs() < 0.01);

        store.store("gg1111111111", b"data").await.unwrap();
        store.load("gg1111111111").await.unwrap(); // hit
        store.load("gg9999999999").await.unwrap(); // miss
        store.load("gg1111111111").await.unwrap(); // hit
        store.load("gg8888888888").await.unwrap(); // miss

        let stats = store.get_stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 2);
        assert!((stats.hit_rate - 0.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_cache_no_max_size_unlimited() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());
        // Default max_size is 0 (unlimited)

        for i in 0..10 {
            store
                .store(&format!("hh{:08x}", i), &vec![0u8; 1000])
                .await
                .unwrap();
        }

        let stats = store.get_stats();
        assert_eq!(stats.entry_count, 10);
        assert_eq!(stats.total_bytes, 10_000);
    }

    #[tokio::test]
    async fn test_store_and_retrieve_with_metadata() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        let data = b"some cached artifact payload";
        let key = "ii11223344556677";
        store.store(key, data).await.unwrap();

        let loaded = store.load(key).await.unwrap();
        assert!(loaded.is_some());

        let loaded_data = loaded.unwrap();
        assert_eq!(loaded_data, data);

        // Verify metadata that would be produced: size and content_type
        let metadata = CacheEntryMetadata {
            size: loaded_data.len() as i64,
            content_type: "application/octet-stream".to_string(),
        };
        assert_eq!(metadata.size, data.len() as i64);
        assert_eq!(metadata.content_type, "application/octet-stream");

        // Stats should reflect the single entry
        let stats = store.get_stats();
        assert_eq!(stats.entry_count, 1);
        assert_eq!(stats.total_bytes, data.len() as u64);
    }

    #[tokio::test]
    async fn test_store_same_key_twice_last_write_wins() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        let key = "jj11223344556677";
        store.store(key, b"first_value").await.unwrap();
        store.store(key, b"second_value_replaces").await.unwrap();

        // Only the second value should be present
        let loaded = store.load(key).await.unwrap();
        assert_eq!(loaded, Some(b"second_value_replaces".to_vec()));

        // Entry count must remain 1 (overwrites, not duplicates)
        let stats = store.get_stats();
        assert_eq!(stats.entry_count, 1);

        // total_bytes accurately reflects only the current entry (not double-counted)
        assert_eq!(
            stats.total_bytes,
            "second_value_replaces".len() as u64
        );
    }

    #[tokio::test]
    async fn test_empty_cache_stats() {
        let tmp = TempDir::new().unwrap();
        let store = LocalCacheStore::new(tmp.path().to_path_buf());

        // Fresh cache with no entries and no lookups
        let stats = store.get_stats();
        assert_eq!(stats.entry_count, 0);
        assert_eq!(stats.total_bytes, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.max_bytes, 0); // 0 means unlimited
                                        // With zero total lookups, hit_rate is defined as 1.0
        assert!((stats.hit_rate - 1.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_multiple_concurrent_stores() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(LocalCacheStore::new(tmp.path().to_path_buf()));

        let mut handles = Vec::new();
        for i in 0..20u32 {
            let s = store.clone();
            handles.push(tokio::spawn(async move {
                let key = format!("kk{:08x}concurrent", i);
                let data = vec![i as u8; 256];
                s.store(&key, &data).await.unwrap();
                let loaded = s.load(&key).await.unwrap();
                assert_eq!(loaded, Some(data));
                (key, i)
            }));
        }

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        // All 20 stores should have succeeded
        assert_eq!(results.len(), 20);

        // Stats should reflect 20 entries
        let stats = store.get_stats();
        assert_eq!(stats.entry_count, 20);
        assert_eq!(stats.total_bytes, 20 * 256);
    }
}
