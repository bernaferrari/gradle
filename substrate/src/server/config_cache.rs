use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    configuration_cache_service_server::ConfigurationCacheService, CleanConfigCacheRequest,
    CleanConfigCacheResponse, LoadConfigCacheRequest, LoadConfigCacheResponse,
    StoreConfigCacheRequest, StoreConfigCacheResponse, ValidateConfigRequest,
    ValidateConfigResponse,
};
use crate::server::config_cache_ir::{
    InvalidationTriggers, PhaseGraphEnvelope, PHASE_GRAPH_SCHEMA_VERSION,
};

/// A cached configuration entry.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ConfigCacheEntry {
    serialized_config: Vec<u8>,
    entry_count: i64,
    input_hashes: Vec<String>,
    timestamp_ms: i64,
    storage_time_ms: i64,
    #[serde(default)]
    last_access_ms: i64,
    /// Build ID that owns this entry, for scope isolation.
    #[serde(default)]
    build_id: String,
    /// Phase graph envelope with invalidation metadata.
    #[serde(default)]
    phase_graph: Option<PhaseGraphEnvelope>,
}

/// Default maximum cache entries before LRU eviction kicks in.
const MAX_CACHE_ENTRIES: usize = 1000;

/// Rust-native configuration cache service.
/// Stores and retrieves serialized build configuration using bincode.
///
/// This replaces Gradle's Java-based configuration cache with a faster
/// Rust implementation using bincode serialization (10-50x faster than
/// Java serialization for complex object graphs).
pub struct ConfigurationCacheServiceImpl {
    cache: DashMap<String, ConfigCacheEntry>,
    /// build_id → cache_keys index for scope isolation.
    build_id_index: DashMap<String, Vec<String>>,
    cache_dir: PathBuf,
    total_stores: AtomicI64,
    total_hits: AtomicI64,
    total_misses: AtomicI64,
    entries_evicted: AtomicI64,
}

impl Default for ConfigurationCacheServiceImpl {
    fn default() -> Self {
        Self::new(std::path::PathBuf::new())
    }
}

impl ConfigurationCacheServiceImpl {
    pub fn new(cache_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&cache_dir).ok();
        Self {
            cache: DashMap::new(),
            build_id_index: DashMap::new(),
            cache_dir,
            total_stores: AtomicI64::new(0),
            total_hits: AtomicI64::new(0),
            total_misses: AtomicI64::new(0),
            entries_evicted: AtomicI64::new(0),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn disk_path(&self, cache_key: &str) -> PathBuf {
        let safe_key = cache_key
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        self.cache_dir.join(format!("{}.bin", safe_key))
    }

    fn persist_to_disk(&self, cache_key: &str, entry: &ConfigCacheEntry) {
        let path = self.disk_path(cache_key);
        if let Ok(data) = bincode::serialize(entry) {
            std::fs::write(&path, data).ok();
        }
    }

    fn load_from_disk(&self, cache_key: &str) -> Option<ConfigCacheEntry> {
        let path = self.disk_path(cache_key);
        let data = std::fs::read(&path).ok()?;
        bincode::deserialize(&data).ok()
    }

    fn remove_from_disk(&self, cache_key: &str) {
        let path = self.disk_path(cache_key);
        std::fs::remove_file(&path).ok();
    }

    /// Build `InvalidationTriggers` from a store request.
    fn triggers_from_store_request(req: &StoreConfigCacheRequest) -> InvalidationTriggers {
        InvalidationTriggers {
            build_script_hash: req.build_script_hash.clone(),
            settings_script_hash: req.settings_script_hash.clone(),
            init_script_hashes: req
                .init_script_hashes
                .iter()
                .map(|e| (e.path.clone(), e.hash.clone()))
                .collect(),
            gradle_version: req.gradle_version.clone(),
            relevant_system_properties: req
                .system_properties
                .iter()
                .map(|e| (e.key.clone(), e.value.clone()))
                .collect(),
        }
    }

    /// Build `InvalidationTriggers` from a validate request.
    fn triggers_from_validate_request(req: &ValidateConfigRequest) -> InvalidationTriggers {
        InvalidationTriggers {
            build_script_hash: req.build_script_hash.clone(),
            settings_script_hash: req.settings_script_hash.clone(),
            init_script_hashes: req
                .init_script_hashes
                .iter()
                .map(|e| (e.path.clone(), e.hash.clone()))
                .collect(),
            gradle_version: req.gradle_version.clone(),
            relevant_system_properties: req
                .system_properties
                .iter()
                .map(|e| (e.key.clone(), e.value.clone()))
                .collect(),
        }
    }

    /// Index a cache key under its build_id for scope isolation.
    fn index_by_build_id(&self, build_id: &str, cache_key: &str) {
        if build_id.is_empty() {
            return;
        }
        if let Some(mut keys) = self.build_id_index.get_mut(build_id) {
            if !keys.contains(&cache_key.to_string()) {
                keys.push(cache_key.to_string());
            }
        } else {
            self.build_id_index
                .insert(build_id.to_string(), vec![cache_key.to_string()]);
        }
    }

    /// Remove a cache key from the build_id index.
    fn deindex_by_build_id(&self, build_id: &str, cache_key: &str) {
        if build_id.is_empty() {
            return;
        }
        if let Some(mut keys) = self.build_id_index.get_mut(build_id) {
            keys.retain(|k| k != cache_key);
            if keys.is_empty() {
                drop(keys);
                self.build_id_index.remove(build_id);
            }
        }
    }

    /// Invalidate all cache entries belonging to a build_id.
    pub fn invalidate_by_build_id(&self, build_id: &str) -> u32 {
        if build_id.is_empty() {
            return 0;
        }
        let keys: Vec<String> = self
            .build_id_index
            .get(build_id)
            .map(|keys| keys.clone())
            .unwrap_or_default();

        for key in &keys {
            if self.cache.remove(key).is_some() {
                self.remove_from_disk(key);
                self.deindex_by_build_id(build_id, key);
                self.entries_evicted.fetch_add(1, Ordering::Relaxed);
            }
        }

        self.build_id_index.remove(build_id);
        keys.len() as u32
    }
}

#[tonic::async_trait]
impl ConfigurationCacheService for ConfigurationCacheServiceImpl {
    async fn store_config_cache(
        &self,
        request: Request<StoreConfigCacheRequest>,
    ) -> Result<Response<StoreConfigCacheResponse>, Status> {
        let req = request.into_inner();
        let start = std::time::Instant::now();

        let build_id = req.build_id.clone();
        let triggers = Self::triggers_from_store_request(&req);
        let has_triggers = !triggers.build_script_hash.is_empty()
            || !triggers.settings_script_hash.is_empty()
            || !triggers.gradle_version.is_empty()
            || !triggers.init_script_hashes.is_empty()
            || !triggers.relevant_system_properties.is_empty();

        let phase_graph = if has_triggers {
            Some(PhaseGraphEnvelope {
                schema_version: PHASE_GRAPH_SCHEMA_VERSION,
                build_id: build_id.clone(),
                triggers,
                project_count: req.entry_count as u32,
                task_count: 0,
                phase_graph_bytes: req.serialized_config.to_vec(),
                creation_time_ms: Self::now_ms(),
            })
        } else {
            None
        };

        let entry = ConfigCacheEntry {
            serialized_config: req.serialized_config.to_vec(),
            entry_count: req.entry_count,
            input_hashes: req.input_hashes.into_iter().collect(),
            timestamp_ms: req.timestamp_ms,
            storage_time_ms: 0,
            last_access_ms: Self::now_ms(),
            build_id: build_id.clone(),
            phase_graph,
        };

        let storage_time_ms = start.elapsed().as_millis() as i64;

        let entry = ConfigCacheEntry {
            storage_time_ms,
            ..entry
        };

        let key = req.cache_key.clone();
        self.index_by_build_id(&build_id, &key);
        self.cache.insert(key.clone(), entry);

        // LRU eviction: if cache is over capacity, remove the least-recently-accessed entries.
        if self.cache.len() > MAX_CACHE_ENTRIES {
            let to_remove = self.cache.len() - MAX_CACHE_ENTRIES / 2;
            let mut candidates: Vec<(i64, String, String)> = self
                .cache
                .iter()
                .map(|entry| {
                    (
                        entry.value().last_access_ms,
                        entry.key().clone(),
                        entry.value().build_id.clone(),
                    )
                })
                .collect();
            candidates.sort_unstable_by_key(|(last_access_ms, _, _)| *last_access_ms);
            let to_evict: Vec<(String, String)> = candidates
                .into_iter()
                .take(to_remove)
                .map(|(_, key, bid)| (key, bid))
                .collect();
            for (k, bid) in &to_evict {
                if self.cache.remove(k).is_some() {
                    self.remove_from_disk(k);
                    self.deindex_by_build_id(bid, k);
                }
            }
            self.entries_evicted
                .fetch_add(to_evict.len() as i64, Ordering::Relaxed);
        }

        // Persist to disk
        if let Some(stored_entry) = self.cache.get(&key) {
            self.persist_to_disk(&key, &stored_entry);
        }

        self.total_stores.fetch_add(1, Ordering::Relaxed);

        tracing::info!(
            cache_key = %req.cache_key,
            entry_count = req.entry_count,
            storage_time_ms,
            "Configuration cache stored"
        );

        Ok(Response::new(StoreConfigCacheResponse {
            stored: true,
            storage_time_ms,
        }))
    }

    async fn load_config_cache(
        &self,
        request: Request<LoadConfigCacheRequest>,
    ) -> Result<Response<LoadConfigCacheResponse>, Status> {
        let req = request.into_inner();

        // Check memory cache first
        if let Some(mut entry) = self.cache.get_mut(&req.cache_key) {
            entry.last_access_ms = Self::now_ms();
            self.total_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Response::new(LoadConfigCacheResponse {
                found: true,
                serialized_config: entry.serialized_config.clone(),
                entry_count: entry.entry_count,
                timestamp_ms: entry.timestamp_ms,
            }));
        }

        // Check disk cache
        if let Some(mut entry) = self.load_from_disk(&req.cache_key) {
            entry.last_access_ms = Self::now_ms();
            self.total_hits.fetch_add(1, Ordering::Relaxed);
            let timestamp = entry.timestamp_ms;
            let count = entry.entry_count;
            let config = entry.serialized_config.clone();
            let bid = entry.build_id.clone();
            self.index_by_build_id(&bid, &req.cache_key);
            self.cache.insert(req.cache_key, entry);
            return Ok(Response::new(LoadConfigCacheResponse {
                found: true,
                serialized_config: config,
                entry_count: count,
                timestamp_ms: timestamp,
            }));
        }

        self.total_misses.fetch_add(1, Ordering::Relaxed);

        Ok(Response::new(LoadConfigCacheResponse {
            found: false,
            serialized_config: Vec::new(),
            entry_count: 0,
            timestamp_ms: 0,
        }))
    }

    async fn validate_config(
        &self,
        request: Request<ValidateConfigRequest>,
    ) -> Result<Response<ValidateConfigResponse>, Status> {
        let req = request.into_inner();

        // Check memory cache first, then disk
        let entry: Option<ConfigCacheEntry> = if let Some(entry) = self.cache.get(&req.cache_key) {
            Some(entry.value().clone())
        } else {
            self.load_from_disk(&req.cache_key)
        };
        let entry = entry.inspect(|e| {
            self.index_by_build_id(&e.build_id, &req.cache_key);
        });

        if let Some(entry) = entry {
            // If the entry has phase graph triggers, use granular validation.
            if let Some(ref pg) = entry.phase_graph {
                let current = Self::triggers_from_validate_request(&req);
                let changed = pg.triggers.changed_triggers(&current);

                if changed.is_empty() {
                    return Ok(Response::new(ValidateConfigResponse {
                        valid: true,
                        reason: "All invalidation triggers match".to_string(),
                        invalidated_triggers: vec![],
                    }));
                }

                return Ok(Response::new(ValidateConfigResponse {
                    valid: false,
                    reason: format!(
                        "Invalidation triggers changed: {}",
                        changed.join(", ")
                    ),
                    invalidated_triggers: changed,
                }));
            }

            // Fallback: legacy input_hashes comparison.
            let mut cached = entry.input_hashes.clone();
            let mut requested = req.input_hashes.clone();
            cached.sort_unstable();
            requested.sort_unstable();
            if cached == requested {
                return Ok(Response::new(ValidateConfigResponse {
                    valid: true,
                    reason: "All input hashes match".to_string(),
                    invalidated_triggers: vec![],
                }));
            }

            return Ok(Response::new(ValidateConfigResponse {
                valid: false,
                reason: "Input hashes do not match cached configuration".to_string(),
                invalidated_triggers: vec!["input_hashes".to_string()],
            }));
        }

        Ok(Response::new(ValidateConfigResponse {
            valid: false,
            reason: "No cached configuration found".to_string(),
            invalidated_triggers: vec![],
        }))
    }

    async fn clean_config_cache(
        &self,
        request: Request<CleanConfigCacheRequest>,
    ) -> Result<Response<CleanConfigCacheResponse>, Status> {
        let req = request.into_inner();
        let now = Self::now_ms();
        let mut removed = 0i32;
        let mut space_recovered = 0i64;

        // Snapshot current cache length for max_entries check.
        let current_len = self.cache.len() as i32;

        // If a build_id is specified, clean all entries for that build
        // (build-scoped clean doesn't require age/count filters).
        let build_scoped = !req.build_id.is_empty();

        // If a build_id is specified, only clean entries belonging to that build.
        let target_keys: Vec<String> = if build_scoped {
            self.build_id_index
                .get(&req.build_id)
                .map(|keys| keys.clone())
                .unwrap_or_default()
        } else {
            // All keys
            self.cache
                .iter()
                .map(|entry| entry.key().clone())
                .collect()
        };

        for key in &target_keys {
            let should_remove = if build_scoped {
                // Build-scoped: remove all entries for this build
                true
            } else if let Some(entry) = self.cache.get(key) {
                let too_old =
                    req.max_age_ms > 0 && now - entry.timestamp_ms > req.max_age_ms;
                let too_many =
                    req.max_entries > 0 && current_len > req.max_entries;
                too_old || too_many
            } else {
                false
            };

            if should_remove {
                if let Some((_, entry)) = self.cache.remove(key) {
                    removed += 1;
                    space_recovered += entry.serialized_config.len() as i64;
                    self.remove_from_disk(key);
                    self.deindex_by_build_id(&entry.build_id, key);
                }
            }
        }

        tracing::info!(
            removed,
            space_recovered_bytes = space_recovered,
            "Configuration cache cleaned"
        );

        Ok(Response::new(CleanConfigCacheResponse {
            entries_removed: removed,
            space_recovered_bytes: space_recovered,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_svc() -> ConfigurationCacheServiceImpl {
        let dir = tempdir().unwrap();
        ConfigurationCacheServiceImpl::new(dir.path().to_path_buf())
    }

    #[tokio::test]
    async fn test_store_and_load() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":app:compileJava".to_string(),
            serialized_config: vec![1, 2, 3, 4, 5].into(),
            entry_count: 10,
            input_hashes: vec!["hash1".to_string(), "hash2".to_string()],
            timestamp_ms: 1000,
            ..Default::default()
        }))
        .await
        .unwrap();

        let resp = svc
            .load_config_cache(Request::new(LoadConfigCacheRequest {
                cache_key: ":app:compileJava".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.entry_count, 10);
        assert_eq!(resp.serialized_config.len(), 5);
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let svc = make_svc();

        let resp = svc
            .load_config_cache(Request::new(LoadConfigCacheRequest {
                cache_key: ":nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
    }

    #[tokio::test]
    async fn test_validate_config() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":app".to_string(),
            serialized_config: vec![].into(),
            entry_count: 1,
            input_hashes: vec!["h1".to_string(), "h2".to_string()],
            timestamp_ms: 100,
            ..Default::default()
        }))
        .await
        .unwrap();

        // Valid
        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":app".to_string(),
                input_hashes: vec!["h1".to_string(), "h2".to_string()],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.valid);

        // Invalid - different hashes
        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":app".to_string(),
                input_hashes: vec!["h1".to_string(), "h3".to_string()],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.valid);

        // Invalid - wrong count
        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":app".to_string(),
                input_hashes: vec!["h1".to_string()],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.valid);
    }

    #[tokio::test]
    async fn test_validate_config_ignores_hash_order() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":ordered".to_string(),
            serialized_config: vec![].into(),
            entry_count: 1,
            input_hashes: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
            timestamp_ms: 100,
            ..Default::default()
        }))
        .await
        .unwrap();

        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":ordered".to_string(),
                input_hashes: vec!["h3".to_string(), "h1".to_string(), "h2".to_string()],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.valid, "same hashes in different order should validate");
    }

    #[tokio::test]
    async fn test_clean_cache() {
        let svc = make_svc();

        for i in 0..5 {
            svc.store_config_cache(Request::new(StoreConfigCacheRequest {
                cache_key: format!(":key{}", i),
                serialized_config: vec![0; 100].into(),
                entry_count: 1,
                input_hashes: vec![],
                timestamp_ms: 100,
                ..Default::default()
            }))
            .await
            .unwrap();
        }

        let resp = svc
            .clean_config_cache(Request::new(CleanConfigCacheRequest {
                max_age_ms: 0,
                max_entries: 2,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.entries_removed >= 3);
        assert!(resp.space_recovered_bytes > 0);
    }

    #[tokio::test]
    async fn test_validate_nonexistent_key() {
        let svc = make_svc();

        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":nonexistent".to_string(),
                input_hashes: vec!["h1".to_string()],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.valid);
        assert!(resp.reason.contains("No cached configuration"));
    }

    #[tokio::test]
    async fn test_clean_nonexistent_key() {
        let svc = make_svc();

        let resp = svc
            .clean_config_cache(Request::new(CleanConfigCacheRequest {
                max_age_ms: 0,
                max_entries: 0,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.entries_removed, 0);
        assert_eq!(resp.space_recovered_bytes, 0);
    }

    #[tokio::test]
    async fn test_update_existing_key() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":update".to_string(),
            serialized_config: vec![1, 2, 3].into(),
            entry_count: 5,
            input_hashes: vec!["old-hash".to_string()],
            timestamp_ms: 100,
            ..Default::default()
        }))
        .await
        .unwrap();

        // Store again with different data
        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":update".to_string(),
            serialized_config: vec![4, 5, 6, 7].into(),
            entry_count: 8,
            input_hashes: vec!["new-hash".to_string()],
            timestamp_ms: 200,
            ..Default::default()
        }))
        .await
        .unwrap();

        let resp = svc
            .load_config_cache(Request::new(LoadConfigCacheRequest {
                cache_key: ":update".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.entry_count, 8);
        assert_eq!(resp.serialized_config.len(), 4);
    }

    #[tokio::test]
    async fn test_validate_wrong_length() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":len".to_string(),
            serialized_config: vec![].into(),
            entry_count: 1,
            input_hashes: vec!["h1".to_string(), "h2".to_string()],
            timestamp_ms: 100,
            ..Default::default()
        }))
        .await
        .unwrap();

        // Request with 1 hash instead of 2
        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":len".to_string(),
                input_hashes: vec!["h1".to_string()],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.valid);
        assert!(resp.reason.contains("do not match"));
    }

    #[tokio::test]
    async fn test_disk_persistence() {
        let dir = tempdir().unwrap();
        let cache_key = ":persistTest";

        {
            let svc = ConfigurationCacheServiceImpl::new(dir.path().to_path_buf());
            svc.store_config_cache(Request::new(StoreConfigCacheRequest {
                cache_key: cache_key.to_string(),
                serialized_config: b"hello config".to_vec().into(),
                entry_count: 5,
                input_hashes: vec!["hash1".to_string()],
                timestamp_ms: 100,
                ..Default::default()
            }))
            .await
            .unwrap();
        }

        // New instance should load from disk
        let svc2 = ConfigurationCacheServiceImpl::new(dir.path().to_path_buf());
        let resp = svc2
            .load_config_cache(Request::new(LoadConfigCacheRequest {
                cache_key: cache_key.to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.entry_count, 5);
        assert_eq!(resp.serialized_config.len(), 12);
    }

    #[tokio::test]
    async fn test_store_and_load_different_build_ids() {
        let svc = make_svc();

        // Store configurations for three different build IDs
        let build_ids = vec!["build-aaa-123", "build-bbb-456", "build-ccc-789"];
        for (i, build_id) in build_ids.iter().enumerate() {
            svc.store_config_cache(Request::new(StoreConfigCacheRequest {
                cache_key: format!("{}:compileKotlin", build_id),
                serialized_config: vec![i as u8; 32].into(),
                entry_count: (i + 1) as i64 * 10,
                input_hashes: vec![format!("hash-{}", i)],
                timestamp_ms: 1000 + i as i64 * 100,
                build_id: build_id.to_string(),
                ..Default::default()
            }))
            .await
            .unwrap();
        }

        // Load each build's configuration and verify independence
        for (i, build_id) in build_ids.iter().enumerate() {
            let resp = svc
                .load_config_cache(Request::new(LoadConfigCacheRequest {
                    cache_key: format!("{}:compileKotlin", build_id),
                }))
                .await
                .unwrap()
                .into_inner();

            assert!(
                resp.found,
                "Expected configuration for build ID {}",
                build_id
            );
            assert_eq!(
                resp.serialized_config.len(),
                32,
                "Unexpected config length for build ID {}",
                build_id
            );
            assert_eq!(
                resp.entry_count,
                (i + 1) as i64 * 10,
                "Unexpected entry count for build ID {}",
                build_id
            );
            assert_eq!(
                resp.timestamp_ms,
                1000 + i as i64 * 100,
                "Unexpected timestamp for build ID {}",
                build_id
            );
        }

        // Loading a cross-build key should miss
        let miss_resp = svc
            .load_config_cache(Request::new(LoadConfigCacheRequest {
                cache_key: "build-aaa-123:compileJava".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!miss_resp.found);
    }

    #[tokio::test]
    async fn test_invalidate_specific_entry() {
        let svc = make_svc();

        // Store three entries
        for i in 0..3 {
            svc.store_config_cache(Request::new(StoreConfigCacheRequest {
                cache_key: format!(":task{}", i),
                serialized_config: vec![i as u8; 16].into(),
                entry_count: (i + 1) as i64,
                input_hashes: vec![format!("hash{}", i)],
                timestamp_ms: 500,
                ..Default::default()
            }))
            .await
            .unwrap();
        }

        let clean_resp = svc
            .clean_config_cache(Request::new(CleanConfigCacheRequest {
                max_age_ms: 200,
                max_entries: 0,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(clean_resp.entries_removed, 3);

        // Verify all are gone
        for i in 0..3 {
            let resp = svc
                .load_config_cache(Request::new(LoadConfigCacheRequest {
                    cache_key: format!(":task{}", i),
                }))
                .await
                .unwrap()
                .into_inner();
            assert!(!resp.found, "Entry :task{} should have been cleaned", i);
        }

        // Re-add two entries and selectively clean only one by max_entries
        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":taskA".to_string(),
            serialized_config: vec![1; 8].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 5000,
            ..Default::default()
        }))
        .await
        .unwrap();
        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":taskB".to_string(),
            serialized_config: vec![2; 8].into(),
            entry_count: 2,
            input_hashes: vec![],
            timestamp_ms: 5000,
            ..Default::default()
        }))
        .await
        .unwrap();

        let selective_resp = svc
            .clean_config_cache(Request::new(CleanConfigCacheRequest {
                max_age_ms: 0,
                max_entries: 1,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(selective_resp.entries_removed, 2);
    }

    #[tokio::test]
    async fn test_load_nonexistent_returns_empty() {
        let svc = make_svc();

        let resp = svc
            .load_config_cache(Request::new(LoadConfigCacheRequest {
                cache_key: ":completely:unknown:build:target".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
        assert!(resp.serialized_config.is_empty());
        assert_eq!(resp.entry_count, 0);
        assert_eq!(resp.timestamp_ms, 0);
    }

    #[tokio::test]
    async fn test_store_large_number_of_entries() {
        let svc = make_svc();
        let num_entries: usize = 200;

        for i in 0..num_entries {
            let resp = svc
                .store_config_cache(Request::new(StoreConfigCacheRequest {
                    cache_key: format!(":large:task{}", i),
                    serialized_config: vec![i as u8; 64].into(),
                    entry_count: i as i64,
                    input_hashes: vec![format!("input-hash-{}", i)],
                    timestamp_ms: i as i64 * 10,
                    ..Default::default()
                }))
                .await
                .unwrap()
                .into_inner();

            assert!(resp.stored, "Failed to store entry {}", i);
        }

        assert_eq!(svc.total_stores.load(Ordering::Relaxed), num_entries as i64);

        for i in [0, num_entries / 2, num_entries - 1] {
            let resp = svc
                .load_config_cache(Request::new(LoadConfigCacheRequest {
                    cache_key: format!(":large:task{}", i),
                }))
                .await
                .unwrap()
                .into_inner();

            assert!(resp.found, "Expected entry :large:task{} to be found", i);
            assert_eq!(resp.serialized_config.len(), 64);
            assert_eq!(resp.entry_count, i as i64);
            assert_eq!(resp.timestamp_ms, i as i64 * 10);
        }

        assert_eq!(svc.total_hits.load(Ordering::Relaxed), 3);

        svc.load_config_cache(Request::new(LoadConfigCacheRequest {
            cache_key: ":large:task99999".to_string(),
        }))
        .await
        .unwrap();
        assert_eq!(svc.total_misses.load(Ordering::Relaxed), 1);

        let validate_resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: format!(":large:task{}", num_entries / 2),
                input_hashes: vec![format!("input-hash-{}", num_entries / 2)],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(validate_resp.valid);
    }

    // --- Phase graph invalidation tests ---

    #[tokio::test]
    async fn test_store_with_triggers_and_validate_match() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":triggered".to_string(),
            serialized_config: vec![42].into(),
            entry_count: 5,
            input_hashes: vec![],
            timestamp_ms: 100,
            build_script_hash: "abc123".to_string(),
            settings_script_hash: "def456".to_string(),
            gradle_version: "8.5".to_string(),
            build_id: "build-1".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap();

        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":triggered".to_string(),
                input_hashes: vec![],
                build_script_hash: "abc123".to_string(),
                settings_script_hash: "def456".to_string(),
                gradle_version: "8.5".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.valid);
        assert!(resp.invalidated_triggers.is_empty());
    }

    #[tokio::test]
    async fn test_validate_build_script_hash_change() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":bs-change".to_string(),
            serialized_config: vec![].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 100,
            build_script_hash: "old-hash".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap();

        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":bs-change".to_string(),
                input_hashes: vec![],
                build_script_hash: "new-hash".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.valid);
        assert!(resp
            .invalidated_triggers
            .iter()
            .any(|t| t == "build_script_hash"));
    }

    #[tokio::test]
    async fn test_validate_gradle_version_change() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":gv-change".to_string(),
            serialized_config: vec![].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 100,
            gradle_version: "8.5".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap();

        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":gv-change".to_string(),
                input_hashes: vec![],
                gradle_version: "8.6".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.valid);
        assert!(resp
            .invalidated_triggers
            .iter()
            .any(|t| t == "gradle_version"));
    }

    #[tokio::test]
    async fn test_validate_init_script_added() {
        let svc = make_svc();

        // Store with a build_script_hash so phase graph is created
        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":init-added".to_string(),
            serialized_config: vec![].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 100,
            build_script_hash: "sha-build".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap();

        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":init-added".to_string(),
                input_hashes: vec![],
                build_script_hash: "sha-build".to_string(),
                init_script_hashes: vec![crate::proto::ScriptHashEntry {
                    path: "init.gradle".to_string(),
                    hash: "h1".to_string(),
                }],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.valid);
        assert!(resp
            .invalidated_triggers
            .iter()
            .any(|t| t.contains("init_script_added")));
    }

    #[tokio::test]
    async fn test_validate_system_property_changed() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":prop-change".to_string(),
            serialized_config: vec![].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 100,
            system_properties: vec![crate::proto::StringEntry {
                key: "profile".to_string(),
                value: "dev".to_string(),
            }],
            ..Default::default()
        }))
        .await
        .unwrap();

        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":prop-change".to_string(),
                input_hashes: vec![],
                system_properties: vec![crate::proto::StringEntry {
                    key: "profile".to_string(),
                    value: "prod".to_string(),
                }],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.valid);
        assert!(resp
            .invalidated_triggers
            .iter()
            .any(|t| t.contains("property_changed")));
    }

    #[tokio::test]
    async fn test_backward_compat_old_entry_without_triggers() {
        let svc = make_svc();

        // Store without any trigger fields (simulates old client)
        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: ":old-entry".to_string(),
            serialized_config: vec![1, 2, 3].into(),
            entry_count: 3,
            input_hashes: vec!["legacy-hash".to_string()],
            timestamp_ms: 100,
            ..Default::default()
        }))
        .await
        .unwrap();

        // Validate with legacy input_hashes only
        let resp = svc
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: ":old-entry".to_string(),
                input_hashes: vec!["legacy-hash".to_string()],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.valid, "old entries without triggers should still validate via input_hashes");
    }

    #[tokio::test]
    async fn test_invalidate_by_build_id() {
        let svc = make_svc();

        // Store entries for two builds
        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: "build-A:task1".to_string(),
            serialized_config: vec![1].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 100,
            build_id: "build-A".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap();
        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: "build-A:task2".to_string(),
            serialized_config: vec![2].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 100,
            build_id: "build-A".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap();
        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: "build-B:task1".to_string(),
            serialized_config: vec![3].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 100,
            build_id: "build-B".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap();

        // Invalidate build-A
        let removed = svc.invalidate_by_build_id("build-A");
        assert_eq!(removed, 2);

        // build-A entries gone
        let resp_a1 = svc
            .load_config_cache(Request::new(LoadConfigCacheRequest {
                cache_key: "build-A:task1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp_a1.found);

        // build-B still present
        let resp_b1 = svc
            .load_config_cache(Request::new(LoadConfigCacheRequest {
                cache_key: "build-B:task1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp_b1.found);
    }

    #[tokio::test]
    async fn test_clean_by_build_id() {
        let svc = make_svc();

        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: "build-X:k1".to_string(),
            serialized_config: vec![1].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 100,
            build_id: "build-X".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap();
        svc.store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: "build-Y:k1".to_string(),
            serialized_config: vec![2].into(),
            entry_count: 1,
            input_hashes: vec![],
            timestamp_ms: 100,
            build_id: "build-Y".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap();

        // Clean only build-X
        let resp = svc
            .clean_config_cache(Request::new(CleanConfigCacheRequest {
                max_age_ms: 0,
                max_entries: 0,
                build_id: "build-X".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.entries_removed, 1);

        // build-Y still present
        let resp_y = svc
            .load_config_cache(Request::new(LoadConfigCacheRequest {
                cache_key: "build-Y:k1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp_y.found);
    }

    #[tokio::test]
    async fn test_phase_graph_disk_persistence() {
        let dir = tempdir().unwrap();
        let cache_key = ":phase-persist";

        {
            let svc = ConfigurationCacheServiceImpl::new(dir.path().to_path_buf());
            svc.store_config_cache(Request::new(StoreConfigCacheRequest {
                cache_key: cache_key.to_string(),
                serialized_config: b"phase-data".to_vec().into(),
                entry_count: 7,
                input_hashes: vec![],
                timestamp_ms: 100,
                build_script_hash: "sha-abc".to_string(),
                gradle_version: "8.7".to_string(),
                build_id: "persist-build".to_string(),
                system_properties: vec![crate::proto::StringEntry {
                    key: "mode".to_string(),
                    value: "release".to_string(),
                }],
                ..Default::default()
            }))
            .await
            .unwrap();
        }

        // New instance loads from disk, validates with triggers
        let svc2 = ConfigurationCacheServiceImpl::new(dir.path().to_path_buf());
        let resp = svc2
            .validate_config(Request::new(ValidateConfigRequest {
                cache_key: cache_key.to_string(),
                input_hashes: vec![],
                build_script_hash: "sha-abc".to_string(),
                gradle_version: "8.7".to_string(),
                system_properties: vec![crate::proto::StringEntry {
                    key: "mode".to_string(),
                    value: "release".to_string(),
                }],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.valid, "phase graph triggers should survive disk persistence");
    }
}
