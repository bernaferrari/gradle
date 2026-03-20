use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    build_cache_orchestration_service_server::BuildCacheOrchestrationService,
    ComputeCacheKeyRequest, ComputeCacheKeyResponse, ProbeCacheRequest, ProbeCacheResponse,
    StoreOutputsRequest, StoreOutputsResponse,
};

/// Rust-native build cache orchestration service.
/// Computes cache keys and coordinates cache operations.
pub struct BuildCacheOrchestrationServiceImpl {
    // Track which cache keys have been stored (local cache availability)
    stored_keys: dashmap::DashMap<String, bool>,
}

impl BuildCacheOrchestrationServiceImpl {
    pub fn new() -> Self {
        Self {
            stored_keys: dashmap::DashMap::new(),
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

        let available = self.stored_keys.contains_key(&key);

        Ok(Response::new(ProbeCacheResponse {
            available,
            location: if available { "local".to_string() } else { String::new() },
        }))
    }

    async fn store_outputs(
        &self,
        request: Request<StoreOutputsRequest>,
    ) -> Result<Response<StoreOutputsResponse>, Status> {
        let req = request.into_inner();
        let key = String::from_utf8_lossy(&req.cache_key).to_string();

        // Mark as stored in local cache tracking
        // The actual data is stored via the existing CacheService (streaming)
        self.stored_keys.insert(key.clone(), true);

        tracing::debug!(
            cache_key = %key,
            execution_time_ms = req.execution_time_ms,
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
        })).await.unwrap().into_inner();
        assert!(store.success);

        // Now available
        let probe = svc.probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"test-key".to_vec(),
        })).await.unwrap().into_inner();
        assert!(probe.available);
        assert_eq!(probe.location, "local");
    }
}
