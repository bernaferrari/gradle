use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    build_cache_orchestration_service_server::BuildCacheOrchestrationService,
    ComputeCacheKeyRequest, ComputeCacheKeyResponse, ProbeCacheRequest, ProbeCacheResponse,
    StoreOutputsRequest, StoreOutputsResponse,
};
use crate::server::cache::{hex, LocalCacheStore};

/// Maximum number of stored cache keys to track before eviction.
const MAX_STORED_KEYS: usize = 50_000;

/// Append a string to the hasher in Gradle's format: length as varint, then UTF-8 bytes.
/// Gradle's `Hasher.putString(CharSequence)` encodes the string length as a varint
/// followed by the string's UTF-8 bytes.
fn gradle_put_string(hasher: &mut Md5, s: &str) {
    let bytes = s.as_bytes();
    // Gradle uses a varint for the byte length
    put_varint(hasher, bytes.len() as u64);
    hasher.update(bytes);
}

/// Append a varint (variable-length integer) to the hasher.
/// Gradle's Hasher uses the same varint encoding as protobuf.
fn put_varint(hasher: &mut Md5, mut value: u64) {
    loop {
        if value < 0x80 {
            hasher.update([value as u8]);
            return;
        }
        hasher.update([((value & 0x7F) | 0x80) as u8]);
        value >>= 7;
    }
}

/// Append a hash (16 bytes of MD5) to the hasher in Gradle's format.
/// Gradle's `Hasher.putHash(HashCode)` writes the raw hash bytes.
fn gradle_put_hash(hasher: &mut Md5, hash_bytes: &[u8]) {
    hasher.update(hash_bytes);
}

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
    /// Optional reference to the local cache store for real probe operations.
    local_cache: Option<Arc<LocalCacheStore>>,
}

impl BuildCacheOrchestrationServiceImpl {
    pub fn new() -> Self {
        Self {
            stored_keys: dashmap::DashMap::new(),
            sequence: AtomicI64::new(0),
            keys_evicted: AtomicI64::new(0),
            cache_hits: AtomicI64::new(0),
            cache_misses: AtomicI64::new(0),
            local_cache: None,
        }
    }

    /// Create with a reference to the local cache store for real cache probing.
    pub fn with_local_cache(local_cache: Arc<LocalCacheStore>) -> Self {
        Self {
            local_cache: Some(local_cache),
            ..Self::new()
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

        // 1. Implementation hash (raw bytes — implementation_hash is already a hex string
        //    representing an MD5/SHA-1 hash of the task implementation)
        let impl_bytes = hex::decode(implementation_hash)
            .unwrap_or_else(|| implementation_hash.as_bytes().to_vec());
        gradle_put_hash(&mut hasher, &impl_bytes);

        // 2. Input properties — sorted by property name, each as (name, value_hash)
        //    Gradle's algorithm: putString(propertyName) then valueSnapshot.appendToHasher()
        let mut sorted_props: Vec<_> = input_property_hashes.iter().collect();
        sorted_props.sort_by_key(|(k, _)| *k);
        for (key, hash) in sorted_props {
            gradle_put_string(&mut hasher, key);
            // The hash value is itself an MD5 hex string; decode to raw bytes
            let hash_bytes = hex::decode(hash)
                .unwrap_or_else(|| hash.as_bytes().to_vec());
            gradle_put_hash(&mut hasher, &hash_bytes);
        }

        // 3. Input file hashes — sorted by property name, each as (name, fingerprint_hash)
        let mut sorted_files: Vec<_> = input_file_hashes.iter().collect();
        sorted_files.sort_by_key(|(k, _)| *k);
        for (key, hash) in sorted_files {
            gradle_put_string(&mut hasher, key);
            let hash_bytes = hex::decode(hash)
                .unwrap_or_else(|| hash.as_bytes().to_vec());
            gradle_put_hash(&mut hasher, &hash_bytes);
        }

        // 4. Output property names — sorted alphabetically
        let mut sorted_outputs: Vec<_> = output_property_names.iter().collect();
        sorted_outputs.sort();
        for output in sorted_outputs {
            gradle_put_string(&mut hasher, output);
        }

        // 5. Work identity (appended last for uniqueness per task)
        gradle_put_string(&mut hasher, work_identity);

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

        // First check the metadata index (tracks what was stored this session)
        if let Some(entry) = self.stored_keys.get(&key) {
            // If we have a local cache reference, verify the entry actually exists on disk
            if let Some(cache) = &self.local_cache {
                // The cache service hex-encodes keys, so we must match that format
                let hex_key = hex::encode(&req.cache_key);
                match cache.contains(&hex_key).await {
                    Ok(true) => {
                        self.cache_hits.fetch_add(1, Ordering::Relaxed);
                        return Ok(Response::new(ProbeCacheResponse {
                            available: true,
                            location: "local".to_string(),
                            output_properties: entry.output_properties.clone(),
                            execution_time_ms: entry.execution_time_ms,
                        }));
                    }
                    Ok(false) => {
                        // Metadata says stored but actual cache entry is gone (GC'd, etc.)
                        tracing::debug!(cache_key = %key, "Cache metadata present but entry missing from disk");
                        self.cache_misses.fetch_add(1, Ordering::Relaxed);
                        return Ok(Response::new(ProbeCacheResponse {
                            available: false,
                            location: String::new(),
                            output_properties: Vec::new(),
                            execution_time_ms: 0,
                        }));
                    }
                    Err(e) => {
                        tracing::debug!(cache_key = %key, error = %e, "Cache probe error, falling back to metadata");
                    }
                }
            }

            // No local cache reference — trust metadata only
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Response::new(ProbeCacheResponse {
                available: true,
                location: "local".to_string(),
                output_properties: entry.output_properties.clone(),
                execution_time_ms: entry.execution_time_ms,
            }));
        }

        // Not in metadata — check local cache directly for entries stored before this session
        if let Some(cache) = &self.local_cache {
            let hex_key = hex::encode(&req.cache_key);
            if let Ok(true) = cache.contains(&hex_key).await {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(Response::new(ProbeCacheResponse {
                    available: true,
                    location: "local".to_string(),
                    output_properties: Vec::new(),
                    execution_time_ms: 0,
                }));
            }
        }

        self.cache_misses.fetch_add(1, Ordering::Relaxed);
        Ok(Response::new(ProbeCacheResponse {
            available: false,
            location: String::new(),
            output_properties: Vec::new(),
            execution_time_ms: 0,
        }))
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
            input_file_hashes: vec![("classpath".to_string(), "hash3".to_string())]
                .into_iter()
                .collect(),
            output_property_names: vec!["classes".to_string()],
        };

        let resp1 = svc
            .compute_cache_key(Request::new(make_req()))
            .await
            .unwrap()
            .into_inner();
        let resp2 = svc
            .compute_cache_key(Request::new(make_req()))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp1.cache_key_string, resp2.cache_key_string);
    }

    #[tokio::test]
    async fn test_compute_cache_key_different_inputs() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        let resp1 = svc
            .compute_cache_key(Request::new(ComputeCacheKeyRequest {
                work_identity: ":compileJava".to_string(),
                implementation_hash: "abc".to_string(),
                input_property_hashes: vec![("x".to_string(), "1".to_string())]
                    .into_iter()
                    .collect(),
                input_file_hashes: HashMap::new(),
                output_property_names: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        let resp2 = svc
            .compute_cache_key(Request::new(ComputeCacheKeyRequest {
                work_identity: ":compileJava".to_string(),
                implementation_hash: "abc".to_string(),
                input_property_hashes: vec![("x".to_string(), "2".to_string())]
                    .into_iter()
                    .collect(),
                input_file_hashes: HashMap::new(),
                output_property_names: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_ne!(resp1.cache_key_string, resp2.cache_key_string);
    }

    #[tokio::test]
    async fn test_probe_and_store() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        // Initially not available
        let probe = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: b"test-key".to_vec(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!probe.available);

        // Store
        let store = svc
            .store_outputs(Request::new(StoreOutputsRequest {
                cache_key: b"test-key".to_vec(),
                execution_time_ms: 500,
                output_properties: vec!["classes".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(store.success);

        // Now available
        let probe = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: b"test-key".to_vec(),
            }))
            .await
            .unwrap()
            .into_inner();
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
            }))
            .await
            .unwrap();
        }

        // After eviction, should be well below the max
        assert!(svc.stored_keys.len() <= MAX_STORED_KEYS);

        // Oldest keys should have been evicted
        let probe = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: b"key-0".to_vec(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!probe.available, "Oldest key should have been evicted");

        // Recent keys should still be present
        let probe = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: format!("key-{}", MAX_STORED_KEYS + 50).as_bytes().to_vec(),
            }))
            .await
            .unwrap()
            .into_inner();
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
        }))
        .await
        .unwrap();

        // Probe should return the stored outputs
        let probe = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: b"key-outputs".to_vec(),
            }))
            .await
            .unwrap()
            .into_inner();

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

        let probe = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: b"nonexistent".to_vec(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!probe.available);
        assert!(probe.output_properties.is_empty());
        assert_eq!(probe.execution_time_ms, 0);
    }

    #[tokio::test]
    async fn test_cache_key_includes_implementation_hash() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        let resp1 = svc
            .compute_cache_key(Request::new(ComputeCacheKeyRequest {
                work_identity: ":compileJava".to_string(),
                implementation_hash: "impl-v1".to_string(),
                input_property_hashes: HashMap::new(),
                input_file_hashes: HashMap::new(),
                output_property_names: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        let resp2 = svc
            .compute_cache_key(Request::new(ComputeCacheKeyRequest {
                work_identity: ":compileJava".to_string(),
                implementation_hash: "impl-v2".to_string(),
                input_property_hashes: HashMap::new(),
                input_file_hashes: HashMap::new(),
                output_property_names: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_ne!(resp1.cache_key_string, resp2.cache_key_string);
    }

    #[tokio::test]
    async fn test_cache_key_includes_work_identity() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        let resp1 = svc
            .compute_cache_key(Request::new(ComputeCacheKeyRequest {
                work_identity: ":compileJava".to_string(),
                implementation_hash: "impl".to_string(),
                input_property_hashes: HashMap::new(),
                input_file_hashes: HashMap::new(),
                output_property_names: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        let resp2 = svc
            .compute_cache_key(Request::new(ComputeCacheKeyRequest {
                work_identity: ":test".to_string(),
                implementation_hash: "impl".to_string(),
                input_property_hashes: HashMap::new(),
                input_file_hashes: HashMap::new(),
                output_property_names: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_ne!(resp1.cache_key_string, resp2.cache_key_string);
    }

    #[tokio::test]
    async fn test_store_overwrites_existing() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        svc.store_outputs(Request::new(StoreOutputsRequest {
            cache_key: b"overwrite-key".to_vec(),
            execution_time_ms: 100,
            output_properties: vec!["old_output".to_string()],
        }))
        .await
        .unwrap();

        svc.store_outputs(Request::new(StoreOutputsRequest {
            cache_key: b"overwrite-key".to_vec(),
            execution_time_ms: 200,
            output_properties: vec!["new_output".to_string()],
        }))
        .await
        .unwrap();

        let probe = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: b"overwrite-key".to_vec(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(probe.available);
        assert_eq!(probe.execution_time_ms, 200);
        assert_eq!(probe.output_properties, vec!["new_output".to_string()]);
    }

    #[tokio::test]
    async fn test_cache_hit_miss_tracking() {
        let svc = BuildCacheOrchestrationServiceImpl::new();

        // Miss
        let _ = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: b"miss-key".to_vec(),
            }))
            .await
            .unwrap();

        // Store
        svc.store_outputs(Request::new(StoreOutputsRequest {
            cache_key: b"hit-key".to_vec(),
            execution_time_ms: 100,
            output_properties: vec![],
        }))
        .await
        .unwrap();

        // Hit
        let _ = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: b"hit-key".to_vec(),
            }))
            .await
            .unwrap();

        // Another miss
        let _ = svc
            .probe_cache(Request::new(ProbeCacheRequest {
                cache_key: b"miss-key-2".to_vec(),
            }))
            .await
            .unwrap();

        assert_eq!(svc.cache_hits.load(Ordering::Relaxed), 1);
        assert_eq!(svc.cache_misses.load(Ordering::Relaxed), 2);
    }

    // ---- Gradle-compatible cache key computation tests ----

    #[test]
    fn test_cache_key_deterministic() {
        let props: std::collections::HashMap<String, String> =
            [("prop1".to_string(), "aaaa".to_string())]
                .into_iter()
                .collect();
        let files: std::collections::HashMap<String, String> =
            [("file1".to_string(), "bbbb".to_string())]
                .into_iter()
                .collect();
        let outputs = vec!["output1".to_string()];

        let key1 = BuildCacheOrchestrationServiceImpl::compute_cache_key(
            ":compileJava",
            "cc112233445566778899aabbccddeeff00",
            &props,
            &files,
            &outputs,
        );
        let key2 = BuildCacheOrchestrationServiceImpl::compute_cache_key(
            ":compileJava",
            "cc112233445566778899aabbccddeeff00",
            &props,
            &files,
            &outputs,
        );
        assert_eq!(key1, key2, "Cache key must be deterministic");
    }

    #[test]
    fn test_cache_key_changes_with_different_inputs() {
        let props1: std::collections::HashMap<String, String> =
            [("prop1".to_string(), "aaaa".to_string())]
                .into_iter()
                .collect();
        let props2: std::collections::HashMap<String, String> =
            [("prop1".to_string(), "bbbb".to_string())]
                .into_iter()
                .collect();
        let files: std::collections::HashMap<String, String> =
            [("file1".to_string(), "cccc".to_string())]
                .into_iter()
                .collect();
        let outputs = vec!["output1".to_string()];

        let key1 = BuildCacheOrchestrationServiceImpl::compute_cache_key(
            ":compileJava",
            "cc112233445566778899aabbccddeeff00",
            &props1,
            &files,
            &outputs,
        );
        let key2 = BuildCacheOrchestrationServiceImpl::compute_cache_key(
            ":compileJava",
            "cc112233445566778899aabbccddeeff00",
            &props2,
            &files,
            &outputs,
        );
        assert_ne!(key1, key2, "Different property values should produce different keys");
    }

    #[test]
    fn test_cache_key_independent_of_insertion_order() {
        let props1: std::collections::HashMap<String, String> = [
            ("alpha".to_string(), "1111".to_string()),
            ("beta".to_string(), "2222".to_string()),
        ]
        .into_iter()
        .collect();
        let props2: std::collections::HashMap<String, String> = [
            ("beta".to_string(), "2222".to_string()),
            ("alpha".to_string(), "1111".to_string()),
        ]
        .into_iter()
        .collect();
        let files = std::collections::HashMap::new();
        let outputs = vec![];

        let key1 = BuildCacheOrchestrationServiceImpl::compute_cache_key(
            ":test",
            "aabbccdd11223344556677889900",
            &props1,
            &files,
            &outputs,
        );
        let key2 = BuildCacheOrchestrationServiceImpl::compute_cache_key(
            ":test",
            "aabbccdd11223344556677889900",
            &props2,
            &files,
            &outputs,
        );
        assert_eq!(key1, key2, "Property insertion order must not affect cache key");
    }

    #[test]
    fn test_cache_key_uses_gradle_varint_format() {
        // Verify that the cache key uses Gradle's varint-encoded string format
        // (not the old pipe-delimited format)
        let key = BuildCacheOrchestrationServiceImpl::compute_cache_key(
            ":compileJava",
            "aabbccdd11223344556677889900",
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &[],
        );
        // The key should NOT contain pipe characters (old format used |impl|, |props|, etc.)
        assert!(!key.contains('|'), "Cache key should use Gradle varint format, not pipe-delimited");
        // The key should be a valid hex string (MD5 = 32 hex chars)
        assert_eq!(key.len(), 32, "MD5 hash should produce 32 hex characters");
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_cache_key_empty_inputs() {
        let key = BuildCacheOrchestrationServiceImpl::compute_cache_key(
            ":test",
            "",
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &[],
        );
        assert_eq!(key.len(), 32);
    }
}
