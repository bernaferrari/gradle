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
    pub(crate) entries: DashMap<String, HistoryEntry>,
    persistence_dir: PathBuf,
    load_hits: AtomicI64,
    load_misses: AtomicI64,
    stores: AtomicI64,
    removes: AtomicI64,
    evictions: AtomicI64,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct HistoryEntry {
    pub(crate) key: String,
    pub(crate) state: Vec<u8>,
    pub(crate) timestamp_ms: i64,
}

impl Default for ExecutionHistoryServiceImpl {
    fn default() -> Self {
        Self::new(std::path::PathBuf::new())
    }
}

impl Clone for ExecutionHistoryServiceImpl {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            persistence_dir: self.persistence_dir.clone(),
            load_hits: AtomicI64::new(self.load_hits.load(Ordering::Relaxed)),
            load_misses: AtomicI64::new(self.load_misses.load(Ordering::Relaxed)),
            stores: AtomicI64::new(self.stores.load(Ordering::Relaxed)),
            removes: AtomicI64::new(self.removes.load(Ordering::Relaxed)),
            evictions: AtomicI64::new(self.evictions.load(Ordering::Relaxed)),
        }
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

        self.entries
            .insert(req.work_identity.clone(), entry.clone());
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

    #[tokio::test]
    async fn test_store_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":overwrite".to_string(),
            state: vec![1, 2],
            timestamp_ms: 100,
        }))
        .await
        .unwrap();

        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":overwrite".to_string(),
            state: vec![3, 4, 5],
            timestamp_ms: 200,
        }))
        .await
        .unwrap();

        let resp = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":overwrite".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.state, vec![3, 4, 5]);
        assert_eq!(resp.timestamp_ms, 200);
    }

    #[tokio::test]
    async fn test_remove_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        // Removing nonexistent key should succeed
        let resp = svc
            .remove_history(Request::new(RemoveHistoryRequest {
                work_identity: ":nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
    }

    #[tokio::test]
    async fn test_stats_initial_state() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        let stats = svc
            .get_history_stats(Request::new(GetHistoryStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(stats.entry_count, 0);
        assert_eq!(stats.total_bytes_stored, 0);
        assert_eq!(stats.load_hits, 0);
        assert_eq!(stats.load_misses, 0);
        assert_eq!(stats.stores, 0);
        assert_eq!(stats.removes, 0);
        assert!((stats.hit_rate - 1.0).abs() < f64::EPSILON); // no loads = 100%
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
            svc.get_task_duration(":compileJava"),
            500,
            "Duration should be from dedicated prefix, not regular state"
        );
    }

    #[tokio::test]
    async fn test_store_and_retrieve_durations_for_multiple_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        // Store durations for several distinct tasks
        svc.store_task_duration(":app:compileKotlin", 3200);
        svc.store_task_duration(":app:processResources", 150);
        svc.store_task_duration(":app:compileJava", 4800);
        svc.store_task_duration(":lib:compileKotlin", 2100);
        svc.store_task_duration(":lib:test", 12500);

        // Verify each task returns its own duration
        assert_eq!(svc.get_task_duration(":app:compileKotlin"), 3200);
        assert_eq!(svc.get_task_duration(":app:processResources"), 150);
        assert_eq!(svc.get_task_duration(":app:compileJava"), 4800);
        assert_eq!(svc.get_task_duration(":lib:compileKotlin"), 2100);
        assert_eq!(svc.get_task_duration(":lib:test"), 12500);

        // A task that was never stored should return 0
        assert_eq!(svc.get_task_duration(":app:jar"), 0);

        // Updating an existing task's duration overwrites the old value
        svc.store_task_duration(":app:compileKotlin", 4100);
        assert_eq!(svc.get_task_duration(":app:compileKotlin"), 4100);

        // The other tasks should remain unaffected
        assert_eq!(svc.get_task_duration(":app:processResources"), 150);
        assert_eq!(svc.get_task_duration(":lib:test"), 12500);
    }

    #[tokio::test]
    async fn test_stats_for_task_with_no_history_returns_zeros() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        // Without storing anything, stats should reflect an empty store
        let stats = svc
            .get_history_stats(Request::new(GetHistoryStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(stats.entry_count, 0);
        assert_eq!(stats.total_bytes_stored, 0);
        assert_eq!(stats.load_hits, 0);
        assert_eq!(stats.load_misses, 0);
        assert_eq!(stats.stores, 0);
        assert_eq!(stats.removes, 0);
        // When no loads have occurred, hit_rate defaults to 1.0
        assert!((stats.hit_rate - 1.0).abs() < f64::EPSILON);

        // Now perform load requests against nonexistent tasks to verify counters
        svc.load_history(Request::new(LoadHistoryRequest {
            work_identity: ":noSuchTask".to_string(),
        }))
        .await
        .unwrap();

        svc.load_history(Request::new(LoadHistoryRequest {
            work_identity: ":alsoMissing".to_string(),
        }))
        .await
        .unwrap();

        let stats = svc
            .get_history_stats(Request::new(GetHistoryStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(stats.entry_count, 0);
        assert_eq!(stats.load_misses, 2);
        assert_eq!(stats.load_hits, 0);
        // hit_rate should be 0.0 (0 hits out of 2 total loads)
        assert!((stats.hit_rate - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_store_and_load_detailed_execution_record() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        // Simulate a detailed execution record: serialized fingerprint + output snapshot
        let execution_state: Vec<u8> = (0u8..=255).collect();
        let timestamp_ms = 1_700_000_000_000i64; // a realistic-ish timestamp

        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":app:compileKotlin".to_string(),
            state: execution_state.clone(),
            timestamp_ms,
        }))
        .await
        .unwrap();

        // Load it back and verify all fields
        let resp = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":app:compileKotlin".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.state.len(), 256);
        assert_eq!(resp.state, execution_state);
        assert_eq!(resp.timestamp_ms, timestamp_ms);

        // Loading with a wrong work_identity should not return this data
        let wrong = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":app:compileJava".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!wrong.found);
        assert!(wrong.state.is_empty());
        assert_eq!(wrong.timestamp_ms, 0);

        // Overwrite with a smaller state and verify the load returns updated data
        let smaller_state = vec![10, 20, 30];
        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":app:compileKotlin".to_string(),
            state: smaller_state.clone(),
            timestamp_ms: timestamp_ms + 60_000,
        }))
        .await
        .unwrap();

        let updated = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":app:compileKotlin".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(updated.found);
        assert_eq!(updated.state, smaller_state);
        assert_eq!(updated.timestamp_ms, timestamp_ms + 60_000);
    }

    #[tokio::test]
    async fn test_clear_history_for_specific_task_leaves_others_intact() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExecutionHistoryServiceImpl::new(dir.path().to_path_buf());

        // Store records for three different tasks
        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":app:compileKotlin".to_string(),
            state: vec![1, 2, 3],
            timestamp_ms: 100,
        }))
        .await
        .unwrap();

        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":app:compileJava".to_string(),
            state: vec![4, 5, 6, 7],
            timestamp_ms: 200,
        }))
        .await
        .unwrap();

        svc.store_history(Request::new(StoreHistoryRequest {
            work_identity: ":app:test".to_string(),
            state: vec![8, 9],
            timestamp_ms: 300,
        }))
        .await
        .unwrap();

        // Verify all three exist
        for identity in &[":app:compileKotlin", ":app:compileJava", ":app:test"] {
            let resp = svc
                .load_history(Request::new(LoadHistoryRequest {
                    work_identity: identity.to_string(),
                }))
                .await
                .unwrap()
                .into_inner();
            assert!(resp.found, "Expected {} to be found", identity);
        }

        // Remove only :app:compileJava
        svc.remove_history(Request::new(RemoveHistoryRequest {
            work_identity: ":app:compileJava".to_string(),
        }))
        .await
        .unwrap();

        // The removed task should no longer be found
        let removed = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":app:compileJava".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!removed.found);

        // The other two tasks should still be present with correct data
        let kotlin = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":app:compileKotlin".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(kotlin.found);
        assert_eq!(kotlin.state, vec![1, 2, 3]);
        assert_eq!(kotlin.timestamp_ms, 100);

        let test = svc
            .load_history(Request::new(LoadHistoryRequest {
                work_identity: ":app:test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(test.found);
        assert_eq!(test.state, vec![8, 9]);
        assert_eq!(test.timestamp_ms, 300);

        // Stats should reflect 2 remaining entries and 1 removal
        let stats = svc
            .get_history_stats(Request::new(GetHistoryStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(stats.entry_count, 2);
        assert_eq!(stats.removes, 1);
        assert_eq!(stats.stores, 3);
        // 3 initial loads (all hits) + 1 load (miss for removed) + 2 loads (kotlin + test hits) = 5 hits, 1 miss
        assert_eq!(stats.load_hits, 5);
        assert_eq!(stats.load_misses, 1);

        // Removing the same task again should be a no-op and not affect counts
        svc.remove_history(Request::new(RemoveHistoryRequest {
            work_identity: ":app:compileJava".to_string(),
        }))
        .await
        .unwrap();

        let stats2 = svc
            .get_history_stats(Request::new(GetHistoryStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(stats2.entry_count, 2);
        assert_eq!(stats2.removes, 2); // incremented even for no-op remove
    }
}
