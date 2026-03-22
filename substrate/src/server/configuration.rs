use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    configuration_service_server::ConfigurationService, CacheConfigurationRequest,
    CacheConfigurationResponse, GetProjectInfoRequest, ProjectInfo, RegisterProjectRequest,
    RegisterProjectResponse, ResolvePropertyRequest, ResolvePropertyResponse,
    ValidateConfigCacheRequest, ValidateConfigCacheResponse,
};

/// Maximum number of cached configurations before eviction.
const MAX_CACHED_CONFIGS: usize = 500;

/// Rust-native configuration service.
/// Manages project properties, plugin tracking, and configuration caching.
/// Supports Gradle convention properties, environment variable fallback,
/// and system property resolution.
#[derive(Default)]
pub struct ConfigurationServiceImpl {
    projects: DashMap<String, ProjectState>,
    config_cache: DashMap<String, ConfigCacheEntry>,
    // Stats
    property_resolutions: AtomicI64,
    property_hits: AtomicI64,
    cache_validations: AtomicI64,
    cache_hits: AtomicI64,
}

struct ProjectState {
    project_dir: String,
    properties: HashMap<String, String>,
    applied_plugins: Vec<String>,
}

struct ConfigCacheEntry {
    hash: Vec<u8>,
    timestamp_ms: i64,
}

/// Gradle convention property names that are automatically resolved.
const GRADLE_CONVENTION_PROPERTIES: &[(&str, &str)] = &[
    ("project.name", "project"),
    ("project.group", "group"),
    ("project.version", "version"),
    ("project.description", "description"),
    ("project.path", "project.path"),
    ("rootProject.name", "rootProject"),
    ("gradle.version", "gradle"),
];

impl ConfigurationServiceImpl {
    pub fn new() -> Self {
        Self {
            projects: DashMap::new(),
            config_cache: DashMap::new(),
            property_resolutions: AtomicI64::new(0),
            property_hits: AtomicI64::new(0),
            cache_validations: AtomicI64::new(0),
            cache_hits: AtomicI64::new(0),
        }
    }

    /// Get all properties for a project.
    pub fn get_project_properties(&self, project_path: &str) -> HashMap<String, String> {
        self.projects
            .get(project_path)
            .map(|p| p.properties.clone())
            .unwrap_or_default()
    }

    /// Resolve a property name with fallback chain:
    /// 1. Project properties (explicitly set)
    /// 2. Gradle convention properties (mapped names)
    /// 3. Environment variables (GRADLE_PROPERTY_ prefix or direct match)
    /// 4. System properties (java.util.Properties via std::env)
    fn resolve_property_fallback(&self, project_path: &str, property_name: &str) -> Option<(String, String)> {
        // 1. Project properties
        if let Some(project) = self.projects.get(project_path) {
            if let Some(value) = project.properties.get(property_name) {
                return Some((value.clone(), "project".to_string()));
            }
        }

        // 2. Gradle convention properties
        for (convention_name, mapped_name) in GRADLE_CONVENTION_PROPERTIES {
            if property_name == *convention_name {
                // Look up the mapped property name
                if let Some(project) = self.projects.get(project_path) {
                    if let Some(value) = project.properties.get(*mapped_name) {
                        return Some((value.clone(), "convention".to_string()));
                    }
                }
            }
        }

        // 3. Environment variables
        // Try GRADLE_PROPERTY_{UPPER_CASE} first, then direct match
        let env_key_upper = format!("GRADLE_PROPERTY_{}", property_name.replace('.', "_").to_uppercase());
        if let Ok(value) = std::env::var(&env_key_upper) {
            return Some((value, "env".to_string()));
        }
        if let Ok(value) = std::env::var(property_name) {
            return Some((value, "env".to_string()));
        }

        // 4. System properties (via java system properties or env vars)
        // In a Rust context, we check env vars with a "org.gradle." prefix
        let sys_prop = format!("org.gradle.{}", property_name);
        if let Ok(value) = std::env::var(&sys_prop) {
            return Some((value, "system".to_string()));
        }

        None
    }

    /// Get stats about configuration service usage.
    pub fn get_stats(&self) -> ConfigStats {
        let resolutions = self.property_resolutions.load(Ordering::Relaxed);
        let hits = self.property_hits.load(Ordering::Relaxed);
        let validations = self.cache_validations.load(Ordering::Relaxed);
        let cache_hits = self.cache_hits.load(Ordering::Relaxed);

        ConfigStats {
            registered_projects: self.projects.len() as i64,
            cached_configs: self.config_cache.len() as i64,
            property_resolutions: resolutions,
            property_hits: hits,
            property_hit_rate: if resolutions > 0 {
                hits as f64 / resolutions as f64
            } else {
                1.0
            },
            cache_validations: validations,
            cache_hits,
            cache_hit_rate: if validations > 0 {
                cache_hits as f64 / validations as f64
            } else {
                1.0
            },
        }
    }

    /// Evict old cache entries if at capacity.
    fn maybe_evict_cache(&self) {
        if self.config_cache.len() <= MAX_CACHED_CONFIGS {
            return;
        }

        let to_remove = self.config_cache.len() - MAX_CACHED_CONFIGS / 2;
        let keys: Vec<String> = self.config_cache.iter().take(to_remove).map(|e| e.key().clone()).collect();
        for key in keys {
            self.config_cache.remove(&key);
        }
    }
}

pub struct ConfigStats {
    pub registered_projects: i64,
    pub cached_configs: i64,
    pub property_resolutions: i64,
    pub property_hits: i64,
    pub property_hit_rate: f64,
    pub cache_validations: i64,
    pub cache_hits: i64,
    pub cache_hit_rate: f64,
}

#[tonic::async_trait]
impl ConfigurationService for ConfigurationServiceImpl {
    async fn register_project(
        &self,
        request: Request<RegisterProjectRequest>,
    ) -> Result<Response<RegisterProjectResponse>, Status> {
        let req = request.into_inner();

        // Derive convention properties from the path
        let mut properties: HashMap<String, String> = req.properties.into_iter().collect();

        // Auto-populate "project" from project_path if not set
        if !properties.contains_key("project") {
            properties.insert(
                "project".to_string(),
                req.project_path.strip_prefix(':').unwrap_or(&req.project_path).to_string(),
            );
        }

        self.projects.insert(
            req.project_path.clone(),
            ProjectState {
                project_dir: req.project_dir,
                properties,
                applied_plugins: req.applied_plugins,
            },
        );

        tracing::debug!(project = %req.project_path, "Registered project");

        Ok(Response::new(RegisterProjectResponse { success: true }))
    }

    async fn get_project_info(
        &self,
        request: Request<GetProjectInfoRequest>,
    ) -> Result<Response<ProjectInfo>, Status> {
        let req = request.into_inner();

        match self.projects.get(&req.project_path) {
            Some(project) => Ok(Response::new(ProjectInfo {
                project_path: req.project_path,
                project_dir: project.project_dir.clone(),
                properties: project.properties.clone(),
                applied_plugins: project.applied_plugins.clone(),
            })),
            None => Err(Status::not_found(format!(
                "Project not found: {}",
                req.project_path
            ))),
        }
    }

    async fn resolve_property(
        &self,
        request: Request<ResolvePropertyRequest>,
    ) -> Result<Response<ResolvePropertyResponse>, Status> {
        let req = request.into_inner();
        self.property_resolutions.fetch_add(1, Ordering::Relaxed);

        // Try fallback chain
        if let Some((value, source)) = self.resolve_property_fallback(&req.project_path, &req.property_name) {
            self.property_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Response::new(ResolvePropertyResponse {
                value,
                source,
                found: true,
            }));
        }

        Ok(Response::new(ResolvePropertyResponse {
            value: String::new(),
            source: String::new(),
            found: false,
        }))
    }

    async fn cache_configuration(
        &self,
        request: Request<CacheConfigurationRequest>,
    ) -> Result<Response<CacheConfigurationResponse>, Status> {
        let req = request.into_inner();

        let entry = ConfigCacheEntry {
            hash: req.config_hash,
            timestamp_ms: req.timestamp_ms,
        };

        let hit = self
            .config_cache
            .get(&req.project_path)
            .map(|existing| existing.hash == entry.hash)
            .unwrap_or(false);

        self.config_cache.insert(req.project_path.clone(), entry);
        self.maybe_evict_cache();

        Ok(Response::new(CacheConfigurationResponse {
            cached: true,
            hit,
        }))
    }

    async fn validate_config_cache(
        &self,
        request: Request<ValidateConfigCacheRequest>,
    ) -> Result<Response<ValidateConfigCacheResponse>, Status> {
        let req = request.into_inner();
        self.cache_validations.fetch_add(1, Ordering::Relaxed);

        if let Some(cached) = self.config_cache.get(&req.project_path) {
            if cached.hash == req.expected_hash {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(Response::new(ValidateConfigCacheResponse {
                    valid: true,
                    reason: "Hash matches cached configuration".to_string(),
                    cached_timestamp_ms: cached.timestamp_ms,
                }));
            } else {
                return Ok(Response::new(ValidateConfigCacheResponse {
                    valid: false,
                    reason: "Configuration hash mismatch".to_string(),
                    cached_timestamp_ms: cached.timestamp_ms,
                }));
            }
        }

        Ok(Response::new(ValidateConfigCacheResponse {
            valid: false,
            reason: "No cached configuration found".to_string(),
            cached_timestamp_ms: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_resolve() {
        let svc = ConfigurationServiceImpl::new();

        let mut props = HashMap::new();
        props.insert("version".to_string(), "1.0".to_string());
        props.insert("sourceCompatibility".to_string(), "17".to_string());

        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/app".to_string(),
            properties: props,
            applied_plugins: vec!["java".to_string(), "idea".to_string()],
        }))
        .await
        .unwrap();

        let resp = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":app".to_string(),
                property_name: "version".to_string(),
                requested_by: ":compileJava".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.value, "1.0");
        assert_eq!(resp.source, "project");
    }

    #[tokio::test]
    async fn test_missing_property() {
        let svc = ConfigurationServiceImpl::new();

        let resp = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":app".to_string(),
                property_name: "missing".to_string(),
                requested_by: ":test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
    }

    #[tokio::test]
    async fn test_convention_property_project_name() {
        let svc = ConfigurationServiceImpl::new();

        let mut props = HashMap::new();
        props.insert("project".to_string(), "my-app".to_string());

        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/app".to_string(),
            properties: props,
            applied_plugins: vec![],
        }))
        .await
        .unwrap();

        // Resolve via convention name
        let resp = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":app".to_string(),
                property_name: "project.name".to_string(),
                requested_by: "test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.value, "my-app");
        assert_eq!(resp.source, "convention");
    }

    #[tokio::test]
    async fn test_convention_property_project_version() {
        let svc = ConfigurationServiceImpl::new();

        let mut props = HashMap::new();
        props.insert("version".to_string(), "2.0.0".to_string());

        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/app".to_string(),
            properties: props,
            applied_plugins: vec![],
        }))
        .await
        .unwrap();

        let resp = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":app".to_string(),
                property_name: "project.version".to_string(),
                requested_by: "test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.value, "2.0.0");
        assert_eq!(resp.source, "convention");
    }

    #[tokio::test]
    async fn test_auto_derive_project_name() {
        let svc = ConfigurationServiceImpl::new();

        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: ":my-app".to_string(),
            project_dir: "/tmp/my-app".to_string(),
            properties: HashMap::new(),
            applied_plugins: vec![],
        }))
        .await
        .unwrap();

        // The "project" property should be auto-derived from the path
        let resp = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":my-app".to_string(),
                property_name: "project".to_string(),
                requested_by: "test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.value, "my-app");

        // And project.name should resolve via convention
        let resp = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":my-app".to_string(),
                property_name: "project.name".to_string(),
                requested_by: "test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.value, "my-app");
        assert_eq!(resp.source, "convention");
    }

    #[tokio::test]
    async fn test_env_var_fallback() {
        let svc = ConfigurationServiceImpl::new();

        // Register a project without the property
        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/app".to_string(),
            properties: HashMap::new(),
            applied_plugins: vec![],
        }))
        .await
        .unwrap();

        // Set an env var for the test
        std::env::set_var("GRADLE_PROPERTY_CUSTOM_PROP", "env_value");
        let resp = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":app".to_string(),
                property_name: "custom.prop".to_string(),
                requested_by: "test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        // Clean up
        std::env::remove_var("GRADLE_PROPERTY_CUSTOM_PROP");

        assert!(resp.found);
        assert_eq!(resp.value, "env_value");
        assert_eq!(resp.source, "env");
    }

    #[tokio::test]
    async fn test_system_property_fallback() {
        let svc = ConfigurationServiceImpl::new();

        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/app".to_string(),
            properties: HashMap::new(),
            applied_plugins: vec![],
        }))
        .await
        .unwrap();

        // Set a system property-style env var
        std::env::set_var("org.gradle.java.home", "/usr/lib/jvm/java-17");
        let resp = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":app".to_string(),
                property_name: "java.home".to_string(),
                requested_by: "test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        std::env::remove_var("org.gradle.java.home");

        assert!(resp.found);
        assert_eq!(resp.value, "/usr/lib/jvm/java-17");
        assert_eq!(resp.source, "system");
    }

    #[tokio::test]
    async fn test_priority_order() {
        let svc = ConfigurationServiceImpl::new();

        // Register project with a property
        let mut props = HashMap::new();
        props.insert("custom".to_string(), "project_value".to_string());
        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/app".to_string(),
            properties: props,
            applied_plugins: vec![],
        }))
        .await
        .unwrap();

        // Set env var with same name
        std::env::set_var("GRADLE_PROPERTY_CUSTOM", "env_value");

        let resp = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":app".to_string(),
                property_name: "custom".to_string(),
                requested_by: "test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        std::env::remove_var("GRADLE_PROPERTY_CUSTOM");

        // Project property should win over env var
        assert!(resp.found);
        assert_eq!(resp.value, "project_value");
        assert_eq!(resp.source, "project");
    }

    #[tokio::test]
    async fn test_config_cache() {
        let svc = ConfigurationServiceImpl::new();

        // Cache miss
        let resp1 = svc
            .cache_configuration(Request::new(CacheConfigurationRequest {
                project_path: ":app".to_string(),
                config_hash: vec![1, 2, 3],
                timestamp_ms: 100,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp1.hit);

        // Cache hit
        let resp2 = svc
            .cache_configuration(Request::new(CacheConfigurationRequest {
                project_path: ":app".to_string(),
                config_hash: vec![1, 2, 3],
                timestamp_ms: 200,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp2.hit);

        // Cache miss (different hash)
        let resp3 = svc
            .cache_configuration(Request::new(CacheConfigurationRequest {
                project_path: ":app".to_string(),
                config_hash: vec![4, 5, 6],
                timestamp_ms: 300,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp3.hit);
    }

    #[tokio::test]
    async fn test_validate_config() {
        let svc = ConfigurationServiceImpl::new();

        // No cache
        let resp = svc
            .validate_config_cache(Request::new(ValidateConfigCacheRequest {
                project_path: ":app".to_string(),
                expected_hash: vec![1, 2, 3],
                input_files: vec![],
                build_script_hashes: vec![],
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.valid);

        // Store cache
        svc.cache_configuration(Request::new(CacheConfigurationRequest {
            project_path: ":app".to_string(),
            config_hash: vec![1, 2, 3],
            timestamp_ms: 100,
        }))
        .await
        .unwrap();

        // Valid
        let resp = svc
            .validate_config_cache(Request::new(ValidateConfigCacheRequest {
                project_path: ":app".to_string(),
                expected_hash: vec![1, 2, 3],
                input_files: vec![],
                build_script_hashes: vec![],
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.valid);

        // Invalid
        let resp = svc
            .validate_config_cache(Request::new(ValidateConfigCacheRequest {
                project_path: ":app".to_string(),
                expected_hash: vec![9, 9, 9],
                input_files: vec![],
                build_script_hashes: vec![],
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.valid);
    }

    #[tokio::test]
    async fn test_configuration_stats() {
        let svc = ConfigurationServiceImpl::new();

        // Register a project with properties
        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/app".to_string(),
            properties: vec![
                ("version".to_string(), "1.0".to_string()),
                ("group".to_string(), "com.example".to_string()),
            ].into_iter().collect(),
            applied_plugins: vec!["java".to_string()],
        })).await.unwrap();

        // Resolve hit
        let _ = svc.resolve_property(Request::new(ResolvePropertyRequest {
            project_path: ":app".to_string(),
            property_name: "version".to_string(),
            requested_by: "test".to_string(),
        })).await.unwrap();

        // Resolve miss
        let _ = svc.resolve_property(Request::new(ResolvePropertyRequest {
            project_path: ":app".to_string(),
            property_name: "missing".to_string(),
            requested_by: "test".to_string(),
        })).await.unwrap();

        // Validate cache hit
        svc.cache_configuration(Request::new(CacheConfigurationRequest {
            project_path: ":app".to_string(),
            config_hash: vec![1, 2, 3],
            timestamp_ms: 100,
        })).await.unwrap();

        svc.validate_config_cache(Request::new(ValidateConfigCacheRequest {
            project_path: ":app".to_string(),
            expected_hash: vec![1, 2, 3],
            input_files: vec![],
            build_script_hashes: vec![],
        })).await.unwrap();

        // Validate cache miss
        svc.validate_config_cache(Request::new(ValidateConfigCacheRequest {
            project_path: ":app".to_string(),
            expected_hash: vec![9, 9, 9],
            input_files: vec![],
            build_script_hashes: vec![],
        })).await.unwrap();

        let stats = svc.get_stats();
        assert_eq!(stats.registered_projects, 1);
        assert_eq!(stats.cached_configs, 1);
        assert_eq!(stats.property_resolutions, 2);
        assert_eq!(stats.property_hits, 1);
        assert!((stats.property_hit_rate - 0.5).abs() < f64::EPSILON);
        assert_eq!(stats.cache_validations, 2);
        assert_eq!(stats.cache_hits, 1);
    }

    #[tokio::test]
    async fn test_cache_eviction() {
        let svc = ConfigurationServiceImpl::new();

        // Fill cache beyond capacity
        for i in 0..(MAX_CACHED_CONFIGS + 100) {
            svc.cache_configuration(Request::new(CacheConfigurationRequest {
                project_path: format!(":project-{}", i),
                config_hash: vec![i as u8],
                timestamp_ms: 100,
            }))
            .await
            .unwrap();
        }

        // Should have evicted entries
        assert!(svc.config_cache.len() <= MAX_CACHED_CONFIGS);
    }
}
