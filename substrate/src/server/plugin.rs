use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    plugin_service_server::PluginService, ApplyPluginRequest, ApplyPluginResponse,
    GetAppliedPluginsRequest, GetAppliedPluginsResponse, HasPluginRequest, HasPluginResponse,
    PluginInfo, RegisterPluginRequest, RegisterPluginResponse,
};

/// Registered plugin metadata.
struct PluginEntry {
    plugin_id: String,
    plugin_class: String,
    version: String,
    is_imperative: bool,
    applies_to: Vec<String>,
}

/// Applied plugin tracking.
struct AppliedPlugin {
    plugin_id: String,
    plugin_class: String,
    version: String,
    applied_at_ms: i64,
    apply_order: i32,
}

/// Rust-native plugin service.
/// Manages plugin registry and application tracking.
pub struct PluginServiceImpl {
    registry: DashMap<String, PluginEntry>,
    applied: DashMap<String, Vec<AppliedPlugin>>,
    apply_counters: DashMap<String, i32>,
}

impl PluginServiceImpl {
    pub fn new() -> Self {
        Self {
            registry: DashMap::new(),
            applied: DashMap::new(),
            apply_counters: DashMap::new(),
        }
    }
}

#[tonic::async_trait]
impl PluginService for PluginServiceImpl {
    async fn register_plugin(
        &self,
        request: Request<RegisterPluginRequest>,
    ) -> Result<Response<RegisterPluginResponse>, Status> {
        let req = request.into_inner();

        self.registry.insert(
            req.plugin_id.clone(),
            PluginEntry {
                plugin_id: req.plugin_id,
                plugin_class: req.plugin_class,
                version: req.version,
                is_imperative: req.is_imperative,
                applies_to: req.applies_to,
            },
        );

        Ok(Response::new(RegisterPluginResponse { success: true }))
    }

    async fn apply_plugin(
        &self,
        request: Request<ApplyPluginRequest>,
    ) -> Result<Response<ApplyPluginResponse>, Status> {
        let req = request.into_inner();

        if !self.registry.contains_key(&req.plugin_id) {
            return Ok(Response::new(ApplyPluginResponse {
                success: false,
                error_message: format!("Plugin '{}' not found in registry", req.plugin_id),
            }));
        }

        let entry = self.registry.get(&req.plugin_id).unwrap();
        let mut order = self
            .apply_counters
            .entry(req.project_path.clone())
            .or_insert(0);

        self.applied
            .entry(req.project_path.clone())
            .or_insert_with(Vec::new)
            .push(AppliedPlugin {
                plugin_id: entry.plugin_id.clone(),
                plugin_class: entry.plugin_class.clone(),
                version: entry.version.clone(),
                applied_at_ms: chrono_now_ms(),
                apply_order: *order,
            });

        *order += 1;

        Ok(Response::new(ApplyPluginResponse {
            success: true,
            error_message: String::new(),
        }))
    }

    async fn has_plugin(
        &self,
        request: Request<HasPluginRequest>,
    ) -> Result<Response<HasPluginResponse>, Status> {
        let req = request.into_inner();

        let has = self
            .applied
            .get(&req.project_path)
            .map(|plugins| plugins.iter().any(|p| p.plugin_id == req.plugin_id))
            .unwrap_or(false);

        Ok(Response::new(HasPluginResponse { has_plugin: has }))
    }

    async fn get_applied_plugins(
        &self,
        request: Request<GetAppliedPluginsRequest>,
    ) -> Result<Response<GetAppliedPluginsResponse>, Status> {
        let req = request.into_inner();

        let plugins = self
            .applied
            .get(&req.project_path)
            .map(|project_plugins| {
                project_plugins
                    .iter()
                    .map(|p| PluginInfo {
                        plugin_id: p.plugin_id.clone(),
                        plugin_class: p.plugin_class.clone(),
                        version: p.version.clone(),
                        applied_at_ms: p.applied_at_ms,
                        apply_order: p.apply_order,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(Response::new(GetAppliedPluginsResponse { plugins }))
    }
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_apply() {
        let svc = PluginServiceImpl::new();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "java".to_string(),
            plugin_class: "org.gradle.api.plugins.JavaPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
        }))
        .await
        .unwrap();

        let resp = svc
            .apply_plugin(Request::new(ApplyPluginRequest {
                plugin_id: "java".to_string(),
                project_path: ":app".to_string(),
                apply_order: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
    }

    #[tokio::test]
    async fn test_apply_unknown_plugin() {
        let svc = PluginServiceImpl::new();

        let resp = svc
            .apply_plugin(Request::new(ApplyPluginRequest {
                plugin_id: "nonexistent".to_string(),
                project_path: ":app".to_string(),
                apply_order: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.success);
        assert!(resp.error_message.contains("not found"));
    }

    #[tokio::test]
    async fn test_has_plugin() {
        let svc = PluginServiceImpl::new();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "java".to_string(),
            plugin_class: "JavaPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
        }))
        .await
        .unwrap();

        svc.apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "java".to_string(),
            project_path: ":app".to_string(),
            apply_order: 0,
        }))
        .await
        .unwrap();

        let resp = svc
            .has_plugin(Request::new(HasPluginRequest {
                plugin_id: "java".to_string(),
                project_path: ":app".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.has_plugin);

        let resp2 = svc
            .has_plugin(Request::new(HasPluginRequest {
                plugin_id: "java".to_string(),
                project_path: ":other".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp2.has_plugin);
    }

    #[tokio::test]
    async fn test_get_applied_plugins() {
        let svc = PluginServiceImpl::new();

        for id in &["java", "kotlin", "idea"] {
            svc.register_plugin(Request::new(RegisterPluginRequest {
                plugin_id: id.to_string(),
                plugin_class: format!("{}.Plugin", id),
                version: "1.0".to_string(),
                is_imperative: false,
                applies_to: vec![],
            }))
            .await
            .unwrap();

            svc.apply_plugin(Request::new(ApplyPluginRequest {
                plugin_id: id.to_string(),
                project_path: ":app".to_string(),
                apply_order: 0,
            }))
            .await
            .unwrap();
        }

        let resp = svc
            .get_applied_plugins(Request::new(GetAppliedPluginsRequest {
                project_path: ":app".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.plugins.len(), 3);
    }
}
