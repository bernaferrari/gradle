use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tonic::{Request, Response, Status};

use crate::proto::{
    execution_history_service_server::ExecutionHistoryService, GetHistoryStatsRequest,
    GetHistoryStatsResponse, LoadHistoryRequest, LoadHistoryResponse, RemoveHistoryRequest,
    RemoveHistoryResponse, StoreHistoryRequest, StoreHistoryResponse,
};

/// Rust-native execution history store.
/// Replaces Java's ExecutionHistoryStore with in-memory DashMap + disk persistence.
pub struct ExecutionHistoryServiceImpl {
    entries: DashMap<String, HistoryEntry>,
    persistence_dir: PathBuf,
    load_hits: AtomicI64,
    load_misses: AtomicI64,
    stores: AtomicI64,
    removes: AtomicI64,
    evictions: AtomicI64,
}

#[derive(Serialize, Deserialize, Clone)]
struct HistoryEntry {
    key: String,
    state: Vec<u8>,
    timestamp_ms: i64,
}

impl Default for ExecutionHistoryServiceImpl {
    fn default() -> Self {
        Self::new(std::path::PathBuf::new())
    }
}

impl ExecutionHistoryServiceImpl {
    pub fn new(persistence_dir: PathBuf) -> Self {
        Self {
            entries: DashMap::new(),
            persistence_dir,
            load_hits: AtomicI64::new(0),
            load_misses: AtomicI64::new(0),
            stores: AtomicI64::new(0),
            removes: AtomicI64::new(0),
            evictions: AtomicI64::new(0),
        }
    }

    /// Load persisted history from disk on startup.
    pub async fn load_from_disk(&self) -> Result<usize, std::io::Error> {
        if !self.persistence_dir.exists() {
            fs::create_dir_all(&self.persistence_dir).await?;
            return Ok(0);
        }

        let mut count = 0;
        let mut entries = fs::read_dir(&self.persistence_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".bin") {
                    match fs::read(entry.path()).await {
                        Ok(data) => {
                            if let Ok(hentry) = bincode::deserialize::<HistoryEntry>(&data) {
                                let original_key = hentry.key.clone();
                                self.entries.insert(original_key, hentry);
                                count += 1;
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to read history file {}: {}", name, e);
                        }
                    }
                }
            }
        }

        tracing::info!("Loaded {} execution history entries from disk", count);
        Ok(count)
    }

    fn state_file_path(&self, key: &str) -> PathBuf {
        // Sanitize key for filesystem use (work identity may contain ':' etc.)
        let safe_key = key.replace([':', '/', '\\'], "_");
        self.persistence_dir.join(format!("{}.bin", safe_key))
    }

    async fn persist_to_disk(&self, key: &str, entry: &HistoryEntry) {
        let path = self.state_file_path(key);
        if let Ok(data) = bincode::serialize(entry) {
            if let Err(e) = fs::write(&path, &data).await {
                tracing::warn!("Failed to persist history for {}: {}", key, e);
            }
        }
    }

    async fn remove_from_disk(&self, key: &str) {
        let path = self.state_file_path(key);
        if let Err(e) = fs::remove_file(&path).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Failed to remove history file for {}: {}", key, e);
            }
        }
    }

    /// Evict old entries if the store exceeds the given max_entries threshold.
    /// Removes entries with the oldest timestamps.
    fn maybe_evict(&self, max_entries: usize) {
        if self.entries.len() <= max_entries {
            return;
        }

        let to_remove_count = self.entries.len() - max_entries / 2;

        // Collect entries sorted by timestamp (oldest first)
        let mut timestamped: Vec<(i64, String)> = self
            .entries
            .iter()
            .map(|entry| (entry.value().timestamp_ms, entry.key().clone()))
            .collect();
        timestamped.sort_by_key(|(ts, _)| *ts);

        for (_, key) in timestamped.into_iter().take(to_remove_count) {
            if self.entries.remove(&key).is_some() {
                self.evictions.fetch_add(1, Ordering::Relaxed);
                // Remove disk file in background (best-effort)
                let path = self.state_file_path(&key);
                tokio::spawn(async move {
                    let _ = fs::remove_file(path).await;
                });
            }
        }
    }

    /// Store a task duration for historical lookups.
    /// Uses a dedicated key prefix to avoid collision with serialized execution state.
    pub fn store_task_duration(&self, task_path: &str, duration_ms: i64) {
        let key = format!("__duration__:{}", task_path);
        let state = duration_ms.to_le_bytes().to_vec();
        let entry = HistoryEntry {
            key: key.clone(),
            state,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        };
        self.entries.insert(key, entry);
    }

    /// Look up a historical task duration. Returns 0 if no history exists.
    pub fn get_task_duration(&self, task_path: &str) -> i64 {
        let key = format!("__duration__:{}", task_path);
        if let Some(entry) = self.entries.get(&key) {
            if entry.state.len() == 8 {
                let bytes: [u8; 8] = entry.state.clone().try_into().unwrap_or([0; 8]);
                return i64::from_le_bytes(bytes);
            }
        }
        0
    }
}

#[tonic::async_trait]
impl ExecutionHistoryService for ExecutionHistoryServiceImpl {
    async fn load_history(
        &self,
        request: Request<LoadHistoryRequest>,
    ) -> Result<Response<LoadHistoryResponse>, Status> {
        let req = request.into_inner();

        if let Some(entry) = self.entries.get(&req.work_identity) {
            self.load_hits.fetch_add(1, Ordering::Relaxed);
            Ok(Response::new(LoadHistoryResponse {
                found: true,
                state: entry.state.clone(),
                timestamp_ms: entry.timestamp_ms,
            }))
        } else {
            self.load_misses.fetch_add(1, Ordering::Relaxed);
            Ok(Response::new(LoadHistoryResponse {
                found: false,
                state: Vec::new(),
                timestamp_ms: 0,
            }))
        }
    }

    async fn store_history(
        &self,
        request: Request<StoreHistoryRequest>,
    ) -> Result<Response<StoreHistoryResponse>, Status> {
        let req = request.into_inner();
        let entry = HistoryEntry {
            key: req.work_identity.clone(),
            state: req.state,
            timestamp_ms: req.timestamp_ms,
        };

        self.entries.insert(req.work_identity.clone(), entry.clone());
        self.persist_to_disk(&req.work_identity, &entry).await;
        self.maybe_evict(10_000);
        self.stores.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(
            work = %req.work_identity,
            size = entry.state.len(),
            "Stored execution history"
        );

        Ok(Response::new(StoreHistoryResponse { success: true }))
    }

    async fn remove_history(
        &self,
        request: Request<RemoveHistoryRequest>,
    ) -> Result<Response<RemoveHistoryResponse>, Status> {
        let req = request.into_inner();

        self.entries.remove(&req.work_identity);
        self.remove_from_disk(&req.work_identity).await;
        self.removes.fetch_add(1, Ordering::Relaxed);

        Ok(Response::new(RemoveHistoryResponse { success: true }))
    }

    async fn get_history_stats(
        &self,
        _request: Request<GetHistoryStatsRequest>,
    ) -> Result<Response<GetHistoryStatsResponse>, Status> {
        let entry_count = self.entries.len() as i64;
        let total_bytes: i64 = self
            .entries
            .iter()
            .map(|e| e.value().state.len() as i64)
            .sum();

        let load_hits = self.load_hits.load(Ordering::Relaxed);
        let load_misses = self.load_misses.load(Ordering::Relaxed);
        let stores = self.stores.load(Ordering::Relaxed);
        let removes = self.removes.load(Ordering::Relaxed);

        let total_loads = load_hits + load_misses;
        let hit_rate = if total_loads > 0 {
            load_hits as f64 / total_loads as f64
        } else {
            1.0
        };

        Ok(Response::new(GetHistoryStatsResponse {
            entry_count,
            total_bytes_stored: total_bytes,
            load_hits,
            load_misses,
            stores,
            removes,
            hit_rate,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":compileJava".to_string(),
            state: vec![1, 2, 3, 4],
            timestamp_ms: 12345,
        }))
        .await
        .unwrap();

        let resp = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":compileJava".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.state, vec![1, 2, 3, 4]);
        assert_eq!(resp.timestamp_ms, 12345);
    }

    #[tokio::test]
    async fn test_load_missing() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        let resp = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":missing".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
    }

    #[tokio::test]
    async fn test_remove() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":test".to_string(),
            state: vec![42],
            timestamp_ms: 1,
        }))
        .await
        .unwrap();

        svc.remove_history(Request::new(RemoveHistoryRequest {
            work_identity: ":test".to_string(),
        }))
        .await
        .unwrap();

        let resp = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
    }

    #[tokio::test]
    async fn test_disk_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // Store in first instance
        let svc1 = ExecutionHistoryServiceImpl::new(path.clone());
        svc1.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":persistTest".to_string(),
            state: vec![99, 100],
            timestamp_ms: 999,
        }))
        .await
        .unwrap();

        // Load in second instance (simulating daemon restart)
        let svc2 = ExecutionHistoryServiceImpl::new(path);
        let loaded = svc2.load_from_disk().await.unwrap();
        assert_eq!(loaded, 1);

        let resp = svc2
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":persistTest".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.state, vec![99, 100]);
        assert_eq!(resp.timestamp_ms, 999);
    }

    #[tokio::test]
    async fn test_history_stats() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        // Store two entries
        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":a".to_string(),
            state: vec![1, 2, 3],
            timestamp_ms: 100,
        }))
        .await
        .unwrap();

        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":b".to_string(),
            state: vec![4, 5],
            timestamp_ms: 200,
        }))
        .await
        .unwrap();

        // Hit and miss
        svc.load_history(Request::new(LoadHistoryRequest {
            work_identity: ":a".to_string(),
        }))
        .await
        .unwrap();

        svc.load_history(Request::new(LoadHistoryRequest {
            work_identity: ":missing".to_string(),
        }))
        .await
        .unwrap();

        // Remove one
        svc.remove_history(Request::new(RemoveHistoryRequest {
            work_identity: ":b".to_string(),
        }))
        .await
        .unwrap();

        let stats = svc
            .get_history_stats(Request::new(GetHistoryStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(stats.entry_count, 1); // :b was removed
        assert_eq!(stats.total_bytes_stored, 3); // only :a remains
        assert_eq!(stats.load_hits, 1);
        assert_eq!(stats.load_misses, 1);
        assert_eq!(stats.stores, 2);
        assert_eq!(stats.removes, 1);
        assert!((stats.hit_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_store_and_get_task_duration() {
        let svc = ExecutionHistoryServiceImpl::new(std::path::PathBuf::new());

        // No history yet
        assert_eq!(svc.get_task_duration(":compileJava"), 0);

        // Store a duration
        svc.store_task_duration(":compileJava", 1234);
        assert_eq!(svc.get_task_duration(":compileJava"), 1234);

        // Update with new duration
        svc.store_task_duration(":compileJava", 5678);
        assert_eq!(svc.get_task_duration(":compileJava"), 5678);

        // Different task
        assert_eq!(svc.get_task_duration(":test"), 0);
        svc.store_task_duration(":test", 999);
        assert_eq!(svc.get_task_duration(":test"), 999);
    }

    #[test]
    fn test_task_duration_uses_dedicated_prefix() {
        let svc = ExecutionHistoryServiceImpl::new(std::path::PathBuf::new());

        // Store a duration for :compileJava
        svc.store_task_duration(":compileJava", 500);

        // Store regular execution state for the same identity
        svc.entries.insert(
            ":compileJava".to_string(),
            HistoryEntry {
                key: ":compileJava".to_string(),
                state: vec![1, 2, 3],
                timestamp_ms: 1000,
            },
        );

        // Duration should still be retrievable (uses __duration__ prefix)
        assert_eq!(svc.get_task_duration(":compileJava"), 500);

        // Regular state should not be confused with duration
        // (state length is 3, not 8, so duration would return 0)
        assert_eq!(
            svc.get_task_duration(":compileJava"), 500,
            "Duration should be from dedicated prefix, not regular state"
        );
    }
}
