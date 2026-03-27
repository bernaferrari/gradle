use dashmap::DashMap;
use std::collections::HashSet;
use tonic::{Request, Response, Status};

use crate::proto::{
    plugin_service_server::PluginService, ApplyPluginRequest, ApplyPluginResponse,
    CheckPluginCompatibilityRequest, CheckPluginCompatibilityResponse, ExtensionInfo,
    GetAppliedPluginsRequest, GetAppliedPluginsResponse, GetExtensionRequest,
    GetExtensionResponse, GetExtensionsRequest, GetExtensionsResponse, HasPluginRequest,
    HasPluginResponse, PluginInfo, RegisterConventionRequest, RegisterConventionResponse,
    RegisterPluginRequest, RegisterPluginResponse, ResolveConventionRequest,
    ResolveConventionResponse,
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

/// Extension registered by a plugin.
struct ExtensionEntry {
    name: String,
    extension_type: String,
    source_plugin: String,
    properties: std::collections::HashMap<String, String>,
}

/// Convention mapping rule.
struct ConventionEntry {
    project_path: String,
    plugin_id: String,
    conventions: std::collections::HashMap<String, String>,
    convention_source: String,
}

impl ConventionEntry {
    fn project_and_plugin(&self) -> (&str, &str) {
        (&self.project_path, &self.plugin_id)
    }
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
    /// Extensions keyed by (project_path, extension_name).
    extensions: DashMap<(String, String), ExtensionEntry>,
    /// Convention mappings keyed by (project_path, property_name).
    conventions: DashMap<(String, String), ConventionEntry>,
}

impl PluginServiceImpl {
    pub fn new() -> Self {
        Self {
            registry: DashMap::new(),
            applied: DashMap::new(),
            apply_counters: DashMap::new(),
            extensions: DashMap::new(),
            conventions: DashMap::new(),
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
                        if let Some(deps_list) = deps.get_mut(id) {
                            deps_list.push(req.clone());
                        }
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
            if let Some(degree) = in_degree.get_mut(id) {
                *degree += reqs.len();
            }
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
                    if let Some(deg) = in_degree.get_mut(other_id) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(other_id.clone());
                        }
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

        let entry = self
            .registry
            .get(&req.plugin_id)
            .expect("plugin should exist after compatibility check");
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

    async fn get_extension(
        &self,
        request: Request<GetExtensionRequest>,
    ) -> Result<Response<GetExtensionResponse>, Status> {
        let req = request.into_inner();
        let key = (req.project_path.clone(), req.extension_name.clone());

        if let Some(ext) = self.extensions.get(&key) {
            // If a specific property_path is requested, try to find it
            let value = if !req.property_path.is_empty() {
                ext.properties
                    .get(&req.property_path)
                    .cloned()
                    .unwrap_or_default()
            } else {
                String::new()
            };

            Ok(Response::new(GetExtensionResponse {
                found: true,
                value,
                extension_type: ext.extension_type.clone(),
                properties: ext.properties.clone(),
                error_message: String::new(),
            }))
        } else {
            Ok(Response::new(GetExtensionResponse {
                found: false,
                value: String::new(),
                extension_type: String::new(),
                properties: std::collections::HashMap::new(),
                error_message: format!(
                    "Extension '{}' not found in project '{}'",
                    req.extension_name, req.project_path
                ),
            }))
        }
    }

    async fn get_extensions(
        &self,
        request: Request<GetExtensionsRequest>,
    ) -> Result<Response<GetExtensionsResponse>, Status> {
        let req = request.into_inner();
        let mut extensions = Vec::new();

        for entry in self.extensions.iter() {
            if entry.key().0 == req.project_path {
                extensions.push(ExtensionInfo {
                    name: entry.name.clone(),
                    r#type: entry.extension_type.clone(),
                    source_plugin: entry.source_plugin.clone(),
                    properties: entry.properties.clone(),
                });
            }
        }

        // Sort by name for deterministic output
        extensions.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Response::new(GetExtensionsResponse { extensions }))
    }

    async fn register_convention(
        &self,
        request: Request<RegisterConventionRequest>,
    ) -> Result<Response<RegisterConventionResponse>, Status> {
        let req = request.into_inner();

        for (property_name, default_value) in &req.conventions {
            let key = (req.project_path.clone(), property_name.clone());
            let entry = ConventionEntry {
                project_path: req.project_path.clone(),
                plugin_id: req.plugin_id.clone(),
                conventions: req.conventions.clone(),
                convention_source: req.convention_source.clone(),
            };
            let _ = entry.project_and_plugin(); // verify fields are accessible
            self.conventions.insert(key, entry);
            tracing::debug!(
                project_path = %req.project_path,
                property = %property_name,
                source = %req.convention_source,
                value = %default_value,
                "Convention registered"
            );
        }

        Ok(Response::new(RegisterConventionResponse { registered: true }))
    }

    async fn resolve_convention(
        &self,
        request: Request<ResolveConventionRequest>,
    ) -> Result<Response<ResolveConventionResponse>, Status> {
        let req = request.into_inner();
        let key = (req.project_path.clone(), req.property_name.clone());

        if let Some(conv) = self.conventions.get(&key) {
            let value = conv
                .conventions
                .get(&req.property_name)
                .cloned()
                .unwrap_or_default();

            // Check if source matches preferred sources
            let source_matches = req.preferred_sources.is_empty()
                || req
                    .preferred_sources
                    .iter()
                    .any(|s| s == &conv.convention_source);

            if source_matches {
                Ok(Response::new(ResolveConventionResponse {
                    found: true,
                    value,
                    source: conv.convention_source.clone(),
                    resolved_by: "convention".to_string(),
                }))
            } else {
                Ok(Response::new(ResolveConventionResponse {
                    found: false,
                    value: String::new(),
                    source: String::new(),
                    resolved_by: String::new(),
                }))
            }
        } else {
            Ok(Response::new(ResolveConventionResponse {
                found: false,
                value: String::new(),
                source: String::new(),
                resolved_by: String::new(),
            }))
        }
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

    // --- Extension tests ---

    #[tokio::test]
    async fn test_get_extension_not_found() {
        let svc = PluginServiceImpl::new();

        let resp = svc
            .get_extension(Request::new(GetExtensionRequest {
                project_path: ":app".to_string(),
                extension_name: "android".to_string(),
                property_path: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
        assert!(!resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_register_and_get_extension() {
        let svc = PluginServiceImpl::new();

        let mut props = std::collections::HashMap::new();
        props.insert(
            "compileSdkVersion".to_string(),
            "34".to_string(),
        );
        props.insert(
            "minSdkVersion".to_string(),
            "24".to_string(),
        );

        svc.extensions.insert(
            (":app".to_string(), "android".to_string()),
            ExtensionEntry {
                name: "android".to_string(),
                extension_type: "com.android.build.gradle.LibraryExtension".to_string(),
                source_plugin: "com.android.library".to_string(),
                properties: props.clone(),
            },
        );

        let resp = svc
            .get_extension(Request::new(GetExtensionRequest {
                project_path: ":app".to_string(),
                extension_name: "android".to_string(),
                property_path: "compileSdkVersion".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.value, "34");
        assert_eq!(
            resp.extension_type,
            "com.android.build.gradle.LibraryExtension"
        );
        assert_eq!(resp.properties.len(), 2);
    }

    #[tokio::test]
    async fn test_get_extensions_lists_all() {
        let svc = PluginServiceImpl::new();

        svc.extensions.insert(
            (":app".to_string(), "android".to_string()),
            ExtensionEntry {
                name: "android".to_string(),
                extension_type: "AndroidExt".to_string(),
                source_plugin: "android".to_string(),
                properties: std::collections::HashMap::new(),
            },
        );
        svc.extensions.insert(
            (":app".to_string(), "kotlin".to_string()),
            ExtensionEntry {
                name: "kotlin".to_string(),
                extension_type: "KotlinExt".to_string(),
                source_plugin: "kotlin-android".to_string(),
                properties: std::collections::HashMap::new(),
            },
        );
        // Extension in different project — should not appear
        svc.extensions.insert(
            (":lib".to_string(), "java".to_string()),
            ExtensionEntry {
                name: "java".to_string(),
                extension_type: "JavaExt".to_string(),
                source_plugin: "java".to_string(),
                properties: std::collections::HashMap::new(),
            },
        );

        let resp = svc
            .get_extensions(Request::new(GetExtensionsRequest {
                project_path: ":app".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.extensions.len(), 2);
        // Should be sorted by name
        assert_eq!(resp.extensions[0].name, "android");
        assert_eq!(resp.extensions[1].name, "kotlin");
    }

    // --- Convention tests ---

    #[tokio::test]
    async fn test_register_and_resolve_convention() {
        let svc = PluginServiceImpl::new();

        let mut conventions = std::collections::HashMap::new();
        conventions.insert("sourceCompatibility".to_string(), "17".to_string());
        conventions.insert("targetCompatibility".to_string(), "17".to_string());

        svc.register_convention(Request::new(RegisterConventionRequest {
            project_path: ":app".to_string(),
            plugin_id: "java".to_string(),
            conventions: conventions.clone(),
            convention_source: "java".to_string(),
        }))
        .await
        .unwrap();

        let resp = svc
            .resolve_convention(Request::new(ResolveConventionRequest {
                project_path: ":app".to_string(),
                property_name: "sourceCompatibility".to_string(),
                preferred_sources: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.value, "17");
        assert_eq!(resp.source, "java");
        assert_eq!(resp.resolved_by, "convention");
    }

    #[tokio::test]
    async fn test_resolve_convention_not_found() {
        let svc = PluginServiceImpl::new();

        let resp = svc
            .resolve_convention(Request::new(ResolveConventionRequest {
                project_path: ":app".to_string(),
                property_name: "nonexistent".to_string(),
                preferred_sources: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
    }

    #[tokio::test]
    async fn test_resolve_convention_with_preferred_source() {
        let svc = PluginServiceImpl::new();

        // Register java convention
        let mut java_convs = std::collections::HashMap::new();
        java_convs.insert("jvmTarget".to_string(), "17".to_string());
        svc.register_convention(Request::new(RegisterConventionRequest {
            project_path: ":app".to_string(),
            plugin_id: "java".to_string(),
            conventions: java_convs,
            convention_source: "java".to_string(),
        }))
        .await
        .unwrap();

        // Register kotlin convention
        let mut kt_convs = std::collections::HashMap::new();
        kt_convs.insert("jvmTarget".to_string(), "21".to_string());
        svc.register_convention(Request::new(RegisterConventionRequest {
            project_path: ":app".to_string(),
            plugin_id: "kotlin".to_string(),
            conventions: kt_convs,
            convention_source: "kotlin".to_string(),
        }))
        .await
        .unwrap();

        // Prefer kotlin source — but java was registered first so it wins
        // (convention is per-key, last-write-wins)
        let resp = svc
            .resolve_convention(Request::new(ResolveConventionRequest {
                project_path: ":app".to_string(),
                property_name: "jvmTarget".to_string(),
                preferred_sources: vec!["kotlin".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        // Kotlin convention was registered last, so it wins (last-write-wins in DashMap)
        // But if the source doesn't match preferred, it returns not found
        // In our case kotlin IS the preferred source, so it should be found
        assert!(resp.found, "kotlin convention should be found with preferred source kotlin");
    }
}
