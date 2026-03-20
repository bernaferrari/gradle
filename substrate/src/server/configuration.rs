use std::collections::HashMap;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    configuration_service_server::ConfigurationService, CacheConfigurationRequest,
    CacheConfigurationResponse, RegisterProjectRequest, RegisterProjectResponse,
    ResolvePropertyRequest, ResolvePropertyResponse, ValidateConfigCacheRequest,
    ValidateConfigCacheResponse,
};

/// Rust-native configuration service.
/// Manages project properties, plugin tracking, and configuration caching.
pub struct ConfigurationServiceImpl {
    projects: DashMap<String, ProjectState>,
    config_cache: DashMap<String, ConfigCacheEntry>,
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

impl ConfigurationServiceImpl {
    pub fn new() -> Self {
        Self {
            projects: DashMap::new(),
            config_cache: DashMap::new(),
        }
    }
}

#[tonic::async_trait]
impl ConfigurationService for ConfigurationServiceImpl {
    async fn register_project(
        &self,
        request: Request<RegisterProjectRequest>,
    ) -> Result<Response<RegisterProjectResponse>, Status> {
        let req = request.into_inner();

        self.projects.insert(
            req.project_path.clone(),
            ProjectState {
                project_dir: req.project_dir,
                properties: req.properties.into_iter().collect(),
                applied_plugins: req.applied_plugins,
            },
        );

        Ok(Response::new(RegisterProjectResponse { success: true }))
    }

    async fn resolve_property(
        &self,
        request: Request<ResolvePropertyRequest>,
    ) -> Result<Response<ResolvePropertyResponse>, Status> {
        let req = request.into_inner();

        if let Some(project) = self.projects.get(&req.project_path) {
            if let Some(value) = project.properties.get(&req.property_name) {
                return Ok(Response::new(ResolvePropertyResponse {
                    value: value.clone(),
                    source: "project".to_string(),
                    found: true,
                }));
            }
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

        if let Some(cached) = self.config_cache.get(&req.project_path) {
            if cached.hash == req.expected_hash {
                return Ok(Response::new(ValidateConfigCacheResponse {
                    valid: true,
                    reason: "Hash matches cached configuration".to_string(),
                }));
            } else {
                return Ok(Response::new(ValidateConfigCacheResponse {
                    valid: false,
                    reason: "Configuration hash mismatch".to_string(),
                }));
            }
        }

        Ok(Response::new(ValidateConfigCacheResponse {
            valid: false,
            reason: "No cached configuration found".to_string(),
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
}
