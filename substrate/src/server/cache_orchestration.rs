use std::sync::atomic::{AtomicI64, Ordering};

use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    build_cache_orchestration_service_server::BuildCacheOrchestrationService,
    ComputeCacheKeyRequest, ComputeCacheKeyResponse, ProbeCacheRequest, ProbeCacheResponse,
    StoreOutputsRequest, StoreOutputsResponse,
};

/// Maximum number of stored cache keys to track before eviction.
const MAX_STORED_KEYS: usize = 50_000;

/// A tracked cached output entry: what outputs were stored and when.
struct CachedOutputEntry {
    /// Monotonically increasing sequence number for eviction ordering.
    sequence: i64,
    /// Execution time in ms when this was stored.
    execution_time_ms: i64,
    /// Output property names that were cached.
    output_properties: Vec<String>,
}

/// Rust-native build cache orchestration service.
/// Computes cache keys and coordinates cache operations.
#[derive(Default)]
pub struct BuildCacheOrchestrationServiceImpl {
    // Track which cache keys have been stored with their output metadata
    stored_keys: dashmap::DashMap<String, CachedOutputEntry>,
    /// Monotonically increasing sequence number for eviction ordering.
    sequence: AtomicI64,
    /// Count of evicted entries.
    keys_evicted: AtomicI64,
    /// Total number of cache hits.
    cache_hits: AtomicI64,
    /// Total number of cache misses.
    cache_misses: AtomicI64,
}

impl BuildCacheOrchestrationServiceImpl {
    pub fn new() -> Self {
        Self {
            stored_keys: dashmap::DashMap::new(),
            sequence: AtomicI64::new(0),
            keys_evicted: AtomicI64::new(0),
            cache_hits: AtomicI64::new(0),
            cache_misses: AtomicI64::new(0),
        }
    }

    /// Evict old entries if the store exceeds the capacity.
    /// Uses the sequence number for eviction ordering — evicts oldest entries.
    fn maybe_evict_keys(&self) {
        if self.stored_keys.len() <= MAX_STORED_KEYS {
            return;
        }

        let to_remove_count = self.stored_keys.len() - MAX_STORED_KEYS / 2;

        // Collect entries sorted by sequence number (oldest first)
        let mut sequenced: Vec<(i64, String)> = self
            .stored_keys
            .iter()
            .map(|entry| (entry.value().sequence, entry.key().clone()))
            .collect();
        sequenced.sort_by_key(|(seq, _)| *seq);

        for (_seq, key) in sequenced.into_iter().take(to_remove_count) {
            if self.stored_keys.remove(&key).is_some() {
                self.keys_evicted.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn compute_cache_key(
        work_identity: &str,
        implementation_hash: &str,
        input_property_hashes: &std::collections::HashMap<String, String>,
        input_file_hashes: &std::collections::HashMap<String, String>,
        output_property_names: &[String],
    ) -> String {
        let mut hasher = Md5::new();

        // Include all components in a deterministic order
        hasher.update(work_identity.as_bytes());
        hasher.update(b"|impl|");
        hasher.update(implementation_hash.as_bytes());
        hasher.update(b"|props|");

        // Sort input property hashes for determinism
        let mut sorted_props: Vec<_> = input_property_hashes.iter().collect();
        sorted_props.sort_by_key(|(k, _)| *k);
        for (key, hash) in sorted_props {
            hasher.update(key.as_bytes());
            hasher.update(b"=");
            hasher.update(hash.as_bytes());
            hasher.update(b";");
        }

        hasher.update(b"|files|");
        let mut sorted_files: Vec<_> = input_file_hashes.iter().collect();
        sorted_files.sort_by_key(|(k, _)| *k);
        for (key, hash) in sorted_files {
            hasher.update(key.as_bytes());
            hasher.update(b"=");
            hasher.update(hash.as_bytes());
            hasher.update(b";");
        }

        hasher.update(b"|outputs|");
        let mut sorted_outputs: Vec<_> = output_property_names.iter().collect();
        sorted_outputs.sort();
        for output in sorted_outputs {
            hasher.update(output.as_bytes());
            hasher.update(b";");
        }

        format!("{:x}", hasher.finalize())
    }
}

#[tonic::async_trait]
impl BuildCacheOrchestrationService for BuildCacheOrchestrationServiceImpl {
    async fn compute_cache_key(
        &self,
        request: Request<ComputeCacheKeyRequest>,
    ) -> Result<Response<ComputeCacheKeyResponse>, Status> {
        let req = request.into_inner();

        let input_property_hashes: std::collections::HashMap<String, String> =
            req.input_property_hashes.into_iter().collect();
        let input_file_hashes: std::collections::HashMap<String, String> =
            req.input_file_hashes.into_iter().collect();
        let output_property_names: Vec<String> = req.output_property_names;

        let cache_key = Self::compute_cache_key(
            &req.work_identity,
            &req.implementation_hash,
            &input_property_hashes,
            &input_file_hashes,
            &output_property_names,
        );

        tracing::debug!(
            work = %req.work_identity,
            cache_key = %cache_key,
            "Computed cache key"
        );

        Ok(Response::new(ComputeCacheKeyResponse {
            cache_key: cache_key.as_bytes().to_vec(),
            cache_key_string: cache_key,
        }))
    }

    async fn probe_cache(
        &self,
        request: Request<ProbeCacheRequest>,
    ) -> Result<Response<ProbeCacheResponse>, Status> {
        let req = request.into_inner();
        let key = String::from_utf8_lossy(&req.cache_key).to_string();

        if let Some(entry) = self.stored_keys.get(&key) {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            Ok(Response::new(ProbeCacheResponse {
                available: true,
                location: "local".to_string(),
                output_properties: entry.output_properties.clone(),
                execution_time_ms: entry.execution_time_ms,
            }))
        } else {
            self.cache_misses.fetch_add(1, Ordering::Relaxed);
            Ok(Response::new(ProbeCacheResponse {
                available: false,
                location: String::new(),
                output_properties: Vec::new(),
                execution_time_ms: 0,
            }))
        }
    }

    async fn store_outputs(
        &self,
        request: Request<StoreOutputsRequest>,
    ) -> Result<Response<StoreOutputsResponse>, Status> {
        let req = request.into_inner();
        let key = String::from_utf8_lossy(&req.cache_key).to_string();

        // Use monotonically increasing sequence for eviction ordering
        let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
        self.stored_keys.insert(
            key.clone(),
            CachedOutputEntry {
                sequence: seq,
                execution_time_ms: req.execution_time_ms,
                output_properties: req.output_properties,
            },
        );
        self.maybe_evict_keys();

        tracing::debug!(
            cache_key = %key,
            execution_time_ms = req.execution_time_ms,
            total_keys = self.stored_keys.len(),
            "Marked outputs as cached"
        );

        Ok(Response::new(StoreOutputsResponse {
            success: true,
            error_message: String::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_compute_cache_key_deterministic() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        let make_req = || ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "abc123".to_string(),
            input_property_hashes: vec![
                ("source".to_string(), "hash1".to_string()),
                ("target".to_string(), "hash2".to_string()),
            ]
            .into_iter()
            .collect(),
            input_file_hashes: vec![
                ("classpath".to_string(), "hash3".to_string()),
            ]
            .into_iter()
            .collect(),
            output_property_names: vec!["classes".to_string()],
        };

        let resp1 = svc.compute_cache_key(Request::new(make_req())).await.unwrap().into_inner();
        let resp2 = svc.compute_cache_key(Request::new(make_req())).await.unwrap().into_inner();

        assert_eq!(resp1.cache_key_string, resp2.cache_key_string);
    }

    #[tokio::test]
    async fn test_compute_cache_key_different_inputs() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        let resp1 = svc.compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "abc".to_string(),
            input_property_hashes: vec![("x".to_string(), "1".to_string())].into_iter().collect(),
            input_file_hashes: HashMap::new(),
            output_property_names: vec![],
        })).await.unwrap().into_inner();

        let resp2 = svc.compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "abc".to_string(),
            input_property_hashes: vec![("x".to_string(), "2".to_string())].into_iter().collect(),
            input_file_hashes: HashMap::new(),
            output_property_names: vec![],
        })).await.unwrap().into_inner();

        assert_ne!(resp1.cache_key_string, resp2.cache_key_string);
    }

    #[tokio::test]
    async fn test_probe_and_store() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        // Initially not available
        let probe = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"test-key".to_vec(),
        })).await.unwrap().into_inner();
        assert!(!probe.available);

        // Store
        let store = svc.store_outputs(Request::new(StoreOutputsRequest {
            cache_key: b"test-key".to_vec(),
            execution_time_ms: 500,
            output_properties: vec!["classes".to_string()],
        })).await.unwrap().into_inner();
        assert!(store.success);

        // Now available
        let probe = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"test-key".to_vec(),
        })).await.unwrap().into_inner();
        assert!(probe.available);
        assert_eq!(probe.location, "local");
    }

    #[tokio::test]
    async fn test_eviction_on_capacity() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        // Store more than MAX_STORED_KEYS entries to trigger eviction
        for i in 0..(MAX_STORED_KEYS + 100) {
            let key = format!("key-{}", i);
            svc.store_outputs(Request::new(StoreOutputsRequest {
                cache_key: key.as_bytes().to_vec(),
                execution_time_ms: i as i64,
                output_properties: Vec::new(),
            })).await.unwrap();
        }

        // After eviction, should be well below the max
        assert!(svc.stored_keys.len() <= MAX_STORED_KEYS);

        // Oldest keys should have been evicted
        let probe = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"key-0".to_vec(),
        })).await.unwrap().into_inner();
        assert!(!probe.available, "Oldest key should have been evicted");

        // Recent keys should still be present
        let probe = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: format!("key-{}", MAX_STORED_KEYS + 50).as_bytes().to_vec(),
        })).await.unwrap().into_inner();
        assert!(probe.available, "Recent key should still be present");
    }

    #[tokio::test]
    async fn test_probe_returns_output_properties() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        // Store with output properties
        svc.store_outputs(Request::new(StoreOutputsRequest {
            cache_key: b"key-outputs".to_vec(),
            execution_time_ms: 250,
            output_properties: vec!["classes".to_string(), "resources".to_string()],
        })).await.unwrap();

        // Probe should return the stored outputs
        let probe = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"key-outputs".to_vec(),
        })).await.unwrap().into_inner();

        assert!(probe.available);
        assert_eq!(probe.location, "local");
        assert_eq!(probe.execution_time_ms, 250);
        assert_eq!(probe.output_properties.len(), 2);
        assert!(probe.output_properties.contains(&"classes".to_string()));
        assert!(probe.output_properties.contains(&"resources".to_string()));
    }

    #[tokio::test]
    async fn test_probe_miss_returns_empty_outputs() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        let probe = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"nonexistent".to_vec(),
        })).await.unwrap().into_inner();

        assert!(!probe.available);
        assert!(probe.output_properties.is_empty());
        assert_eq!(probe.execution_time_ms, 0);
    }

    #[tokio::test]
    async fn test_cache_key_includes_implementation_hash() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        let resp1 = svc.compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "impl-v1".to_string(),
            input_property_hashes: HashMap::new(),
            input_file_hashes: HashMap::new(),
            output_property_names: vec![],
        })).await.unwrap().into_inner();

        let resp2 = svc.compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "impl-v2".to_string(),
            input_property_hashes: HashMap::new(),
            input_file_hashes: HashMap::new(),
            output_property_names: vec![],
        })).await.unwrap().into_inner();

        assert_ne!(resp1.cache_key_string, resp2.cache_key_string);
    }

    #[tokio::test]
    async fn test_cache_key_includes_work_identity() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        let resp1 = svc.compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "impl".to_string(),
            input_property_hashes: HashMap::new(),
            input_file_hashes: HashMap::new(),
            output_property_names: vec![],
        })).await.unwrap().into_inner();

        let resp2 = svc.compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":test".to_string(),
            implementation_hash: "impl".to_string(),
            input_property_hashes: HashMap::new(),
            input_file_hashes: HashMap::new(),
            output_property_names: vec![],
        })).await.unwrap().into_inner();

        assert_ne!(resp1.cache_key_string, resp2.cache_key_string);
    }

    #[tokio::test]
    async fn test_store_overwrites_existing() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        svc.store_outputs(Request::new(StoreOutputsRequest {
            cache_key: b"overwrite-key".to_vec(),
            execution_time_ms: 100,
            output_properties: vec!["old_output".to_string()],
        })).await.unwrap();

        svc.store_outputs(Request::new(StoreOutputsRequest {
            cache_key: b"overwrite-key".to_vec(),
            execution_time_ms: 200,
            output_properties: vec!["new_output".to_string()],
        })).await.unwrap();

        let probe = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"overwrite-key".to_vec(),
        })).await.unwrap().into_inner();

        assert!(probe.available);
        assert_eq!(probe.execution_time_ms, 200);
        assert_eq!(probe.output_properties, vec!["new_output".to_string()]);
    }

    #[tokio::test]
    async fn test_cache_hit_miss_tracking() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        // Miss
        let _ = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"miss-key".to_vec(),
        })).await.unwrap();

        // Store
        svc.store_outputs(Request::new(StoreOutputsRequest {
            cache_key: b"hit-key".to_vec(),
            execution_time_ms: 100,
            output_properties: vec![],
        })).await.unwrap();

        // Hit
        let _ = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"hit-key".to_vec(),
        })).await.unwrap();

        // Another miss
        let _ = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"miss-key-2".to_vec(),
        })).await.unwrap();

        assert_eq!(svc.cache_hits.load(Ordering::Relaxed), 1);
        assert_eq!(svc.cache_misses.load(Ordering::Relaxed), 2);
    }
}
