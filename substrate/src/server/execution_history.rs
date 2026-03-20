use std::path::PathBuf;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tonic::{Request, Response, Status};

use crate::proto::{
    execution_history_service_server::ExecutionHistoryService, LoadHistoryRequest,
    LoadHistoryResponse, RemoveHistoryRequest, RemoveHistoryResponse, StoreHistoryRequest,
    StoreHistoryResponse,
};

/// Rust-native execution history store.
/// Replaces Java's ExecutionHistoryStore with in-memory DashMap + disk persistence.
pub struct ExecutionHistoryServiceImpl {
    entries: DashMap<String, HistoryEntry>,
    persistence_dir: PathBuf,
}

#[derive(Serialize, Deserialize, Clone)]
struct HistoryEntry {
    key: String,
    state: Vec<u8>,
    timestamp_ms: i64,
}

impl ExecutionHistoryServiceImpl {
    pub fn new(persistence_dir: PathBuf) -> Self {
        Self {
            entries: DashMap::new(),
            persistence_dir,
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
        let safe_key = key.replace(':', "_").replace('/', "_").replace('\\', "_");
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
}

#[tonic::async_trait]
impl ExecutionHistoryService for ExecutionHistoryServiceImpl {
    async fn load_history(
        &self,
        request: Request<LoadHistoryRequest>,
    ) -> Result<Response<LoadHistoryResponse>, Status> {
        let req = request.into_inner();

        if let Some(entry) = self.entries.get(&req.work_identity) {
            Ok(Response::new(LoadHistoryResponse {
                found: true,
                state: entry.state.clone(),
                timestamp_ms: entry.timestamp_ms,
            }))
        } else {
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

        Ok(Response::new(RemoveHistoryResponse { success: true }))
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
}
