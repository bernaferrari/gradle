use std::path::PathBuf;

use tokio::fs;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use crate::error::SubstrateError;
use crate::proto::{
    cache_load_chunk, cache_service_server::CacheService, cache_store_chunk, CacheLoadChunk,
    CacheLoadRequest, CacheStoreChunk, CacheStoreResponse, CacheEntryMetadata,
};

const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks for streaming

/// Local filesystem build cache store.
/// Entries are stored as files named by their cache key (hex string).
#[derive(Clone)]
pub struct LocalCacheStore {
    base_dir: PathBuf,
}

impl LocalCacheStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
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
        Ok(fs::metadata(&path).await.is_ok())
    }

    pub async fn load(&self, key: &str) -> Result<Option<Vec<u8>>, SubstrateError> {
        let path = self.key_to_path(key);
        match fs::read(&path).await {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(SubstrateError::Cache(format!("Failed to load cache entry {}: {}", key, e))),
        }
    }

    pub async fn store(&self, key: &str, data: &[u8]) -> Result<(), SubstrateError> {
        let path = self.key_to_path(key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&path, data).await?;
        Ok(())
    }

    pub async fn remove(&self, key: &str) -> Result<bool, SubstrateError> {
        let path = self.key_to_path(key);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(SubstrateError::Cache(format!("Failed to remove cache entry {}: {}", key, e))),
        }
    }
}

pub struct CacheServiceImpl {
    store: LocalCacheStore,
}

impl CacheServiceImpl {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            store: LocalCacheStore::new(base_dir),
        }
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

        let (tx, rx) = mpsc::channel(8);

        tokio::spawn(async move {
            match store.load(&key).await {
                Ok(Some(data)) => {
                    let _ = tx
                        .send(Ok(CacheLoadChunk {
                            payload: Some(cache_load_chunk::Payload::Metadata(
                                CacheEntryMetadata {
                                    size: data.len() as i64,
                                    content_type: "application/octet-stream".to_string(),
                                },
                            )),
                        }))
                        .await;

                    for chunk in data.chunks(CHUNK_SIZE) {
                        if tx.is_closed() {
                            break;
                        }
                        let _ = tx
                            .send(Ok(CacheLoadChunk {
                                payload: Some(cache_load_chunk::Payload::Data(
                                    chunk.to_vec(),
                                )),
                            }))
                            .await;
                    }
                }
                Ok(None) => {
                    tracing::debug!(key = %key, "Cache miss");
                }
                Err(e) => {
                    let _ = tx
                        .send(Err(Status::internal(e.to_string())))
                        .await;
                }
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
        let mut total_size: i64 = 0;
        let mut data = Vec::new();

        while let Some(chunk) = stream.message().await.map_err(|e| {
            Status::internal(format!("Failed to read stream chunk: {}", e))
        })? {
            match chunk.payload {
                Some(cache_store_chunk::Payload::Init(init)) => {
                    key = Some(hex::encode(&init.key));
                    total_size = init.total_size;
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

        match store.store(&key, &data).await {
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

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
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
}
