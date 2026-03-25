use dashmap::DashMap;
use std::collections::HashSet;
use tonic::{Request, Response, Status};

use crate::proto::{
    plugin_service_server::PluginService, ApplyPluginRequest, ApplyPluginResponse,
    CheckPluginCompatibilityRequest, CheckPluginCompatibilityResponse, GetAppliedPluginsRequest,
    GetAppliedPluginsResponse, HasPluginRequest, HasPluginResponse, PluginInfo,
    RegisterPluginRequest, RegisterPluginResponse,
};

/// Registered plugin metadata.
struct PluginEntry {
    plugin_id: String,
    plugin_class: String,
    version: String,
    is_imperative: bool,
    applies_to: Vec<String>,
    /// Plugins that must be applied before this one.
    requires: Vec<String>,
    /// Plugins that are incompatible with this one.
    conflicts_with: Vec<String>,
}

/// Applied plugin tracking.
struct AppliedPlugin {
    plugin_id: String,
    plugin_class: String,
    version: String,
    applied_at_ms: i64,
    apply_order: i32,
}

/// Plugin compatibility check result.
pub struct CompatibilityResult {
    compatible: bool,
    warnings: Vec<String>,
    errors: Vec<String>,
}

/// Rust-native plugin service.
/// Manages plugin registry and application tracking with dependency resolution,
/// conflict detection, and topological ordering.
#[derive(Default)]
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

    /// Check if a plugin can be applied to a project given current state.
    pub fn check_compatibility(&self, plugin_id: &str, project_path: &str) -> CompatibilityResult {
        let mut result = CompatibilityResult {
            compatible: true,
            warnings: Vec::new(),
            errors: Vec::new(),
        };

        // Check if plugin exists
        let entry = match self.registry.get(plugin_id) {
            Some(e) => e,
            None => {
                result
                    .errors
                    .push(format!("Plugin '{}' not found in registry", plugin_id));
                result.compatible = false;
                return result;
            }
        };

        // Check if the plugin's target scope includes this project
        if !entry.applies_to.is_empty() {
            let project_matches = entry
                .applies_to
                .iter()
                .any(|target| project_path_matches(project_path, target));
            if !project_matches {
                result.errors.push(format!(
                    "Plugin '{}' targets {:?} but is being applied to '{}'",
                    plugin_id, entry.applies_to, project_path
                ));
                result.compatible = false;
            }
        }

        // Warn about imperative (legacy) plugin application style
        if entry.is_imperative {
            result.warnings.push(format!(
                "Plugin '{}' uses imperative 'plugins {{}}' application style; consider using the DSL 'plugins {{ id(\"{}\") }}' block instead",
                plugin_id, plugin_id
            ));
        }

        // Check if already applied
        if let Some(applied_plugins) = self.applied.get(project_path) {
            // Check conflicts even if already applied
            for applied in applied_plugins.iter() {
                if entry.conflicts_with.contains(&applied.plugin_id) {
                    result.errors.push(format!(
                        "Plugin '{}' conflicts with already-applied '{}'",
                        plugin_id, applied.plugin_id
                    ));
                    result.compatible = false;
                }
            }

            for applied in applied_plugins.iter() {
                if applied.plugin_id == plugin_id {
                    result.warnings.push(format!(
                        "Plugin '{}' already applied to '{}'",
                        plugin_id, project_path
                    ));
                    return result;
                }
            }

            // Check requirements
            for req_id in &entry.requires {
                let req_met = applied_plugins.iter().any(|p| p.plugin_id == *req_id);
                if !req_met {
                    result.errors.push(format!(
                        "Plugin '{}' requires '{}' which is not applied",
                        plugin_id, req_id
                    ));
                    result.compatible = false;
                }
            }
        } else {
            // No plugins applied yet — check if this one has requirements
            if !entry.requires.is_empty() {
                result.errors.push(format!(
                    "Plugin '{}' requires {:?} but no plugins are applied yet",
                    plugin_id, entry.requires
                ));
                result.compatible = false;
            }
        }

        result
    }

    /// Compute a valid apply order for a set of plugin IDs using topological sort.
    /// Returns the plugin IDs in dependency order (requirements before dependents).
    /// Returns None if there's a circular dependency.
    pub fn resolve_apply_order(&self, plugin_ids: &[String]) -> Option<Vec<String>> {
        // Build a local dependency graph from the requested plugins
        let id_set: HashSet<&str> = plugin_ids.iter().map(|s| s.as_str()).collect();

        // Collect requirements per plugin (owned strings to avoid lifetime issues)
        let mut deps: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for id in &id_set {
            deps.insert(id.to_string(), Vec::new());
        }

        for id in plugin_ids {
            if let Some(entry) = self.registry.get(id.as_str()) {
                for req in &entry.requires {
                    if id_set.contains(req.as_str()) {
                        deps.get_mut(id).unwrap().push(req.clone());
                    }
                }
            }
        }

        // Kahn's algorithm for topological sort
        // in_degree[plugin] = number of requirements that plugin has (within the set)
        let mut in_degree: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for id in plugin_ids {
            in_degree.insert(id.clone(), 0);
        }
        for (id, reqs) in &deps {
            *in_degree.get_mut(id).unwrap() += reqs.len();
        }

        let mut queue: std::collections::VecDeque<String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(id, _)| id.clone())
            .collect();

        let mut result = Vec::new();
        while let Some(id) = queue.pop_front() {
            result.push(id.clone());
            // Find all plugins that depend on this one
            for (other_id, reqs) in &deps {
                if reqs.contains(&id) {
                    let deg = in_degree.get_mut(other_id).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(other_id.clone());
                    }
                }
            }
        }

        if result.len() != plugin_ids.len() {
            None // Circular dependency
        } else {
            Some(result)
        }
    }

    /// Get all registered plugin IDs.
    pub fn registered_plugin_ids(&self) -> Vec<String> {
        self.registry.iter().map(|e| e.key().clone()).collect()
    }

    /// Get the apply order for a project.
    pub fn get_apply_order(&self, project_path: &str) -> Vec<String> {
        self.applied
            .get(project_path)
            .map(|plugins| {
                let mut ordered: Vec<_> = plugins.iter().collect();
                ordered.sort_by_key(|p| p.apply_order);
                ordered.iter().map(|p| p.plugin_id.clone()).collect()
            })
            .unwrap_or_default()
    }
}

#[tonic::async_trait]
impl PluginService for PluginServiceImpl {
    async fn register_plugin(
        &self,
        request: Request<RegisterPluginRequest>,
    ) -> Result<Response<RegisterPluginResponse>, Status> {
        let req = request.into_inner();

        let plugin_id = req.plugin_id.clone();
        let is_imperative = req.is_imperative;
        let applies_to = req.applies_to.clone();
        let requires = req.requires.clone();
        let conflicts_with = req.conflicts_with.clone();

        self.registry.insert(
            plugin_id.clone(),
            PluginEntry {
                plugin_id: req.plugin_id,
                plugin_class: req.plugin_class,
                version: req.version,
                is_imperative: req.is_imperative,
                applies_to: req.applies_to,
                requires: req.requires,
                conflicts_with: req.conflicts_with,
            },
        );

        tracing::debug!(
            plugin_id = %plugin_id,
            is_imperative = is_imperative,
            applies_to = ?applies_to,
            requires = ?requires,
            conflicts_with = ?conflicts_with,
            "Registered plugin"
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

        // Run compatibility check
        let compat = self.check_compatibility(&req.plugin_id, &req.project_path);
        if !compat.compatible {
            return Ok(Response::new(ApplyPluginResponse {
                success: false,
                error_message: compat.errors.join("; "),
            }));
        }

        let entry = self.registry.get(&req.plugin_id).unwrap();
        let mut order = self
            .apply_counters
            .entry(req.project_path.clone())
            .or_insert(0);

        self.applied
            .entry(req.project_path.clone())
            .or_default()
            .push(AppliedPlugin {
                plugin_id: entry.plugin_id.clone(),
                plugin_class: entry.plugin_class.clone(),
                version: entry.version.clone(),
                applied_at_ms: chrono_now_ms(),
                apply_order: *order,
            });

        *order += 1;

        tracing::debug!(
            plugin_id = %req.plugin_id,
            project = %req.project_path,
            is_imperative = entry.is_imperative,
            apply_order = *order,
            "Applied plugin"
        );

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
                let mut ordered: Vec<_> = project_plugins.iter().collect();
                ordered.sort_by_key(|p| p.apply_order);
                ordered
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

    async fn check_plugin_compatibility(
        &self,
        request: Request<CheckPluginCompatibilityRequest>,
    ) -> Result<Response<CheckPluginCompatibilityResponse>, Status> {
        let req = request.into_inner();
        let result = self.check_compatibility(&req.plugin_id, &req.project_path);

        Ok(Response::new(CheckPluginCompatibilityResponse {
            compatible: result.compatible,
            errors: result.errors,
            warnings: result.warnings,
        }))
    }
}

/// Check whether a project path matches a plugin's target scope.
///
/// A target of `"*"` or `"all"` matches any project path.
/// Otherwise the target is treated as a prefix — e.g. `"java"` matches
/// `":app"` (since the colon indicates a subproject of a Java project)
/// or `":core"` (any subproject). A bare prefix like `"java-app"` matches
/// project paths that start with that prefix.
fn project_path_matches(project_path: &str, target: &str) -> bool {
    if target == "*" || target == "all" {
        return true;
    }
    // An empty project path (root project) only matches wildcard targets.
    if project_path.is_empty() {
        return false;
    }
    // Extract the project name (strip leading ":")
    let project_name = project_path.strip_prefix(':').unwrap_or(project_path);
    target == project_name || project_name.starts_with(&format!("{}-", target))
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
            requires: vec![],
            conflicts_with: vec![],
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
            requires: vec![],
            conflicts_with: vec![],
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
    async fn test_get_applied_plugins_ordered() {
        let svc = PluginServiceImpl::new();

        for id in &["java", "kotlin", "idea"] {
            svc.register_plugin(Request::new(RegisterPluginRequest {
                plugin_id: id.to_string(),
                plugin_class: format!("{}.Plugin", id),
                version: "1.0".to_string(),
                is_imperative: false,
                applies_to: vec![],
                requires: vec![],
                conflicts_with: vec![],
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
        // Verify ordering
        assert_eq!(resp.plugins[0].plugin_id, "java");
        assert_eq!(resp.plugins[1].plugin_id, "kotlin");
        assert_eq!(resp.plugins[2].plugin_id, "idea");
        // Verify apply_order is monotonically increasing
        assert!(resp.plugins[0].apply_order < resp.plugins[1].apply_order);
        assert!(resp.plugins[1].apply_order < resp.plugins[2].apply_order);
    }

    #[tokio::test]
    async fn test_plugin_conflict_detection() {
        let svc = PluginServiceImpl::new();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "java".to_string(),
            plugin_class: "JavaPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec![],
            conflicts_with: vec!["groovy".to_string()],
        }))
        .await
        .unwrap();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "groovy".to_string(),
            plugin_class: "GroovyPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["java".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        // Apply java first
        let result = svc.check_compatibility("java", ":app");
        assert!(result.compatible);
        assert!(result.errors.is_empty());

        svc.apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "java".to_string(),
            project_path: ":app".to_string(),
            apply_order: 0,
        }))
        .await
        .unwrap();

        // Try to apply groovy — requires java (which is applied, OK)
        let result = svc.check_compatibility("groovy", ":app");
        assert!(result.compatible);

        // Now try java again — should warn about already applied
        let result = svc.check_compatibility("java", ":app");
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("already applied")));

        // Apply groovy, then try java — should conflict
        svc.apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "groovy".to_string(),
            project_path: ":app".to_string(),
            apply_order: 1,
        }))
        .await
        .unwrap();

        // Try to apply java again (now conflicts with groovy)
        let result = svc.check_compatibility("java", ":app");
        assert!(!result.compatible);
        assert!(result.errors.iter().any(|e| e.contains("conflicts")));
    }

    #[tokio::test]
    async fn test_plugin_requires_missing() {
        let svc = PluginServiceImpl::new();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "kotlin".to_string(),
            plugin_class: "KotlinPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["java".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        // Kotlin requires java but nothing is applied
        let result = svc.check_compatibility("kotlin", ":app");
        assert!(!result.compatible);
        assert!(result.errors.iter().any(|e| e.contains("requires")));
    }

    #[tokio::test]
    async fn test_apply_checks_compatibility() {
        let svc = PluginServiceImpl::new();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "kotlin".to_string(),
            plugin_class: "KotlinPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["java".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        // Applying kotlin without java should fail
        let resp = svc
            .apply_plugin(Request::new(ApplyPluginRequest {
                plugin_id: "kotlin".to_string(),
                project_path: ":app".to_string(),
                apply_order: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.success);
        assert!(resp.error_message.contains("requires"));
    }

    #[tokio::test]
    async fn test_resolve_apply_order_simple() {
        let svc = PluginServiceImpl::new();

        // java has no deps, kotlin requires java
        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "java".to_string(),
            plugin_class: "JavaPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec![],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "kotlin".to_string(),
            plugin_class: "KotlinPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["java".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        let order = svc.resolve_apply_order(&["kotlin".to_string(), "java".to_string()]);

        let order = order.expect("Should resolve without cycles");
        assert_eq!(order.len(), 2);
        // Java must come before kotlin
        let java_idx = order.iter().position(|p| p == "java").unwrap();
        let kotlin_idx = order.iter().position(|p| p == "kotlin").unwrap();
        assert!(java_idx < kotlin_idx);
    }

    #[tokio::test]
    async fn test_resolve_apply_order_chain() {
        let svc = PluginServiceImpl::new();

        // A -> B -> C (C requires B, B requires A)
        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "a".to_string(),
            plugin_class: "A".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec![],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "b".to_string(),
            plugin_class: "B".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["a".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "c".to_string(),
            plugin_class: "C".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["b".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        let order = svc.resolve_apply_order(&["c".to_string(), "a".to_string(), "b".to_string()]);

        let order = order.expect("Should resolve without cycles");
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn test_resolve_apply_order_circular() {
        let svc = PluginServiceImpl::new();

        // A requires B, B requires A (circular)
        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "a".to_string(),
            plugin_class: "A".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["b".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "b".to_string(),
            plugin_class: "B".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["a".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        let order = svc.resolve_apply_order(&["a".to_string(), "b".to_string()]);

        assert!(order.is_none(), "Circular dependency should return None");
    }

    #[tokio::test]
    async fn test_registered_plugin_ids() {
        let svc = PluginServiceImpl::new();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "java".to_string(),
            plugin_class: "JavaPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec![],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        svc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "kotlin".to_string(),
            plugin_class: "KotlinPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec![],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        let ids = svc.registered_plugin_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"java".to_string()));
        assert!(ids.contains(&"kotlin".to_string()));
    }
}
