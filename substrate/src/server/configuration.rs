use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    configuration_service_server::ConfigurationService, CacheConfigurationRequest,
    CacheConfigurationResponse, GetProjectInfoRequest, GetPropertyRequest, GetPropertyResponse,
    ListPropertiesRequest, ListPropertiesResponse, ProjectInfo, PropertyEntry,
    RegisterProjectRequest, RegisterProjectResponse, ResolvePropertiesRequest,
    ResolvePropertiesResponse, ResolvePropertyRequest, ResolvePropertyResponse, ResolvedProperty,
    SetPropertyRequest, SetPropertyResponse, ValidateConfigCacheRequest,
    ValidateConfigCacheResponse,
};

// ---------------------------------------------------------------------------
// Property source layers
// ---------------------------------------------------------------------------

/// Ordered by Gradle precedence: highest number wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PropertySource {
    /// Properties defined in build.gradle / build.gradle.kts (lowest precedence).
    BuildScript = 0,
    /// User-defined extra properties (`project.ext.*`).
    Extra = 1,
    /// Properties from `gradle.properties` files.
    GradleProperties = 2,
    /// Environment variables (`ORG_GRADLE_PROJECT_<UPPER>`).
    EnvVariable = 3,
    /// JVM system properties (`-D` flags).
    SystemProperty = 4,
    /// Command-line `-P` flags (highest precedence).
    CommandLine = 5,
}

impl PropertySource {
    /// Human-readable tag used in proto responses and logging.
    pub fn as_str(&self) -> &'static str {
        match self {
            PropertySource::CommandLine => "command_line",
            PropertySource::SystemProperty => "system_property",
            PropertySource::EnvVariable => "env_variable",
            PropertySource::GradleProperties => "gradle_properties",
            PropertySource::BuildScript => "build_script",
            PropertySource::Extra => "extra",
        }
    }

    /// Parse from a proto/source string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "command_line" | "command-line" | "commandline" => Some(PropertySource::CommandLine),
            "system_property" | "system-property" | "systemproperty" | "system" => {
                Some(PropertySource::SystemProperty)
            }
            "env_variable" | "env-variable" | "envvariable" | "env" => {
                Some(PropertySource::EnvVariable)
            }
            "gradle_properties" | "gradle-properties" | "gradleproperties" | "gradle" => {
                Some(PropertySource::GradleProperties)
            }
            "build_script" | "build-script" | "buildscript" => Some(PropertySource::BuildScript),
            "extra" | "ext" => Some(PropertySource::Extra),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Convention property mappings
// ---------------------------------------------------------------------------

/// Mapping from Gradle dotted access patterns to the flat property key stored
/// in the project.  The value on the right is the key that actually lives in
/// the property map; the left is the conventional dotted name.
const CONVENTION_PROPERTIES: &[(&str, &str)] = &[
    ("project.name", "project"),
    ("project.group", "group"),
    ("project.version", "version"),
    ("project.description", "description"),
    ("project.path", "project.path"),
    ("rootProject.name", "rootProject"),
    ("gradle.version", "gradle"),
];

// ---------------------------------------------------------------------------
// Maximum number of cached configurations before eviction.
// ---------------------------------------------------------------------------

const MAX_CACHED_CONFIGS: usize = 500;

/// Guard against infinite recursion during interpolation.
const MAX_INTERPOLATION_DEPTH: usize = 32;

// ---------------------------------------------------------------------------
// Core service
// ---------------------------------------------------------------------------

/// Rust-native configuration service.
///
/// Manages project properties across multiple layers following Gradle's
/// precedence ordering, supports convention property access patterns, and
/// provides `${...}` interpolation.
#[derive(Default)]
pub struct ConfigurationServiceImpl {
    /// Per-project state keyed by Gradle project path (`:app`, `:lib`, etc.)
    projects: DashMap<String, ProjectState>,

    /// Global command-line properties (`-P` flags).  These are not
    /// per-project in Gradle; they apply to the whole build.
    command_line_props: DashMap<String, String>,

    /// Configuration cache keyed by project path.
    config_cache: DashMap<String, ConfigCacheEntry>,

    // --- counters ---
    property_resolutions: AtomicI64,
    property_hits: AtomicI64,
    cache_validations: AtomicI64,
    cache_hits: AtomicI64,
}

// ---------------------------------------------------------------------------
// Project state
// ---------------------------------------------------------------------------

struct ProjectState {
    project_dir: String,
    /// Layered property storage: source -> (key -> value).
    layers: HashMap<PropertySource, HashMap<String, String>>,
    applied_plugins: Vec<String>,
}

impl ProjectState {
    fn new(
        project_dir: String,
        gradle_props: HashMap<String, String>,
        applied_plugins: Vec<String>,
    ) -> Self {
        let mut layers = HashMap::with_capacity(3);
        layers.insert(PropertySource::GradleProperties, gradle_props);
        // Initialise empty layers so they can be mutated later.
        layers.insert(PropertySource::BuildScript, HashMap::new());
        layers.insert(PropertySource::Extra, HashMap::new());
        Self {
            project_dir,
            layers,
            applied_plugins,
        }
    }

    /// Get a property from a specific layer.
    fn get_from_layer(&self, source: PropertySource, key: &str) -> Option<&String> {
        self.layers.get(&source).and_then(|l| l.get(key))
    }

    /// Set a property in a specific layer, returning the previous value.
    fn set_in_layer(
        &mut self,
        source: PropertySource,
        key: String,
        value: String,
    ) -> Option<String> {
        let layer = self.layers.entry(source).or_default();
        layer.insert(key, value)
    }

    /// Collect all entries from a specific layer.
    fn list_layer(&self, source: &PropertySource) -> Vec<(String, String)> {
        self.layers
            .get(source)
            .map(|l| l.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    /// All non-empty layers flattened, highest-precedence first.
    fn all_properties_sorted(&self) -> Vec<(String, String, PropertySource)> {
        let total_props: usize = self.layers.values().map(|l| l.len()).sum();
        let mut entries: Vec<(String, String, PropertySource)> = Vec::with_capacity(total_props);
        // Iterate layers from highest precedence to lowest.
        let mut sources: Vec<PropertySource> = self.layers.keys().copied().collect();
        sources.sort_unstable_by(|a, b| b.cmp(a));
        let mut seen = HashSet::with_capacity(total_props);
        for source in sources {
            if let Some(layer) = self.layers.get(&source) {
                for (k, v) in layer {
                    if seen.insert(k.clone()) {
                        entries.push((k.clone(), v.clone(), source));
                    }
                }
            }
        }
        entries
    }
}

// ---------------------------------------------------------------------------
// Config cache entry
// ---------------------------------------------------------------------------

struct ConfigCacheEntry {
    hash: Vec<u8>,
    timestamp_ms: i64,
}

// ---------------------------------------------------------------------------
// Property access pattern normalisation
// ---------------------------------------------------------------------------

/// Normalise a Gradle property access expression into the underlying flat
/// property key.
///
/// Supported patterns:
/// - `project.property('name')`  ->  `name`
/// - `project.hasProperty('name')`  ->  `name`  (used for existence check)
/// - `project.ext.name`  ->  `name`  (marks as extra-property lookup)
/// - `project.name` / `project.version` / etc.  -> convention mapping
/// - `rootProject.name`  -> convention mapping
/// - `gradle.version`  -> convention mapping
/// - Bare name (e.g. `foo`)  ->  `foo`
///
/// Returns `(normalised_key, is_ext_access)`.
fn normalize_access_pattern(raw: &str) -> (String, bool) {
    let trimmed = raw.trim();

    // project.property('name') or project.hasProperty('name')
    if let Some(rest) = trimmed
        .strip_prefix("project.property(")
        .or_else(|| trimmed.strip_prefix("project.hasProperty("))
    {
        // Extract the quoted name – handle single or double quotes.
        let rest = rest.trim_start();
        if let Some(quoted) = rest
            .strip_prefix('\'')
            .and_then(|r| r.split('\'').next())
            .or_else(|| rest.strip_prefix('"').and_then(|r| r.split('"').next()))
        {
            return (quoted.trim().to_string(), false);
        }
    }

    // project.ext.name  or  ext.name
    if let Some(rest) = trimmed
        .strip_prefix("project.ext.")
        .or_else(|| trimmed.strip_prefix("ext."))
    {
        return (rest.to_string(), true);
    }

    // Check convention mappings first (project.name, project.version, etc.)
    for (pattern, mapped) in CONVENTION_PROPERTIES {
        if trimmed == *pattern {
            return (mapped.to_string(), false);
        }
    }

    // project.<anything_else> – strip the "project." prefix and treat as a
    // direct property lookup.
    if let Some(rest) = trimmed.strip_prefix("project.") {
        return (rest.to_string(), false);
    }

    // Bare name.
    (trimmed.to_string(), false)
}

// ---------------------------------------------------------------------------
// Interpolation
// ---------------------------------------------------------------------------

/// Resolve `${property.name}` and `${property.name:-default}` references in
/// a template string.
///
/// The `resolve_fn` closure receives a property name and returns
/// `Some(value)` if found or `None` if missing.
fn interpolate_template(
    template: &str,
    resolve_fn: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    interpolate_template_depth(template, resolve_fn, 0)
}

fn interpolate_template_depth(
    template: &str,
    resolve_fn: &dyn Fn(&str) -> Option<String>,
    depth: usize,
) -> Result<String, String> {
    if depth > MAX_INTERPOLATION_DEPTH {
        return Err(format!(
            "Max interpolation depth ({}) exceeded – possible circular reference",
            MAX_INTERPOLATION_DEPTH
        ));
    }

    let mut result = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch == '$' && chars.peek().map(|(_, c)| *c) == Some('{') {
            // Consume `${`.
            chars.next();
            // Collect until `}`.
            let expr_start = idx + 2;
            let mut expr_end = expr_start;
            let mut found_close = false;
            for (i, c) in chars.by_ref() {
                if c == '}' {
                    expr_end = i;
                    found_close = true;
                    break;
                }
            }
            if !found_close {
                // Unclosed `${` – emit literally.
                result.push_str("${");
                result.push_str(&template[expr_start..]);
                break;
            }
            let expr = &template[expr_start..expr_end];
            let (prop_name, default) = parse_interpolation_expr(expr);

            if let Some(value) = resolve_fn(prop_name) {
                // Recursively interpolate the resolved value (nested refs).
                let nested = interpolate_template_depth(&value, resolve_fn, depth + 1)?;
                result.push_str(&nested);
            } else if let Some(default) = default {
                result.push_str(default);
            } else {
                return Err(format!(
                    "Property '{}' not found for interpolation",
                    prop_name
                ));
            }
        } else {
            result.push(ch);
        }
    }
    Ok(result)
}

/// Parse `${name:-default}` or `${name}` expressions.
/// Returns `(name, Option<default>)`.
fn parse_interpolation_expr(expr: &str) -> (&str, Option<&str>) {
    // Look for `:-` separator.
    if let Some(pos) = expr.find(":-") {
        let name = expr[..pos].trim();
        let default = expr[pos + 2..].trim();
        (name, Some(default))
    } else {
        (expr.trim(), None)
    }
}

// ---------------------------------------------------------------------------
// Impl
// ---------------------------------------------------------------------------

impl ConfigurationServiceImpl {
    pub fn new() -> Self {
        Self {
            projects: DashMap::new(),
            command_line_props: DashMap::new(),
            config_cache: DashMap::new(),
            property_resolutions: AtomicI64::new(0),
            property_hits: AtomicI64::new(0),
            cache_validations: AtomicI64::new(0),
            cache_hits: AtomicI64::new(0),
        }
    }

    // -- Inject command-line properties (called at build start) --------------

    /// Register command-line `-P` properties.  These have the highest
    /// precedence across all projects.
    pub fn set_command_line_properties(&self, props: HashMap<String, String>) {
        for (k, v) in props {
            self.command_line_props.insert(k, v);
        }
    }

    // -- Internal layered resolution -----------------------------------------

    /// Resolve a property following Gradle's precedence ordering.
    ///
    /// Precedence (highest first):
    ///   1. Command-line (`-P`)
    ///   2. System properties (`-D` / `org.gradle.*` env)
    ///   3. Environment variables (`ORG_GRADLE_PROJECT_<UPPER>`)
    ///   4. `gradle.properties`
    ///   5. Build script properties
    ///   6. Extra properties (`project.ext.*`)
    ///
    /// Convention mappings (e.g. `project.name` -> `project`) are resolved
    /// at each layer as a fallback.
    ///
    /// If `is_ext_access` is true the search is restricted to the Extra layer
    /// only (matching `project.ext.name` semantics).
    fn resolve_property_internal(
        &self,
        project_path: &str,
        property_name: &str,
        is_ext_access: bool,
    ) -> Option<(String, String)> {
        if is_ext_access {
            // project.ext.name – only look in the extra layer.
            if let Some(project) = self.projects.get(project_path) {
                if let Some(value) = project.get_from_layer(PropertySource::Extra, property_name) {
                    return Some((value.clone(), PropertySource::Extra.as_str().to_string()));
                }
            }
            return None;
        }

        // --- Layer 1: Command-line (-P) ---
        if let Some(value) = self.command_line_props.get(property_name) {
            return Some((
                value.clone(),
                PropertySource::CommandLine.as_str().to_string(),
            ));
        }

        // --- Layer 2: System properties ---
        // In a Rust substrate we check for org.gradle.<name> env vars first,
        // then fall back to a generic system-property lookup.
        let sys_prop = format!("org.gradle.{}", property_name);
        if let Ok(value) = std::env::var(&sys_prop) {
            return Some((value, PropertySource::SystemProperty.as_str().to_string()));
        }

        // --- Layer 3: Environment variables ---
        // Gradle checks ORG_GRADLE_PROJECT_<UPPER_CASE_NAME> first, then
        // GRADLE_PROPERTY_<UPPER_CASE_NAME>.
        let env_suffix = property_name.replace('.', "_").to_uppercase();
        let env_key = format!("ORG_GRADLE_PROJECT_{}", env_suffix);
        if let Ok(value) = std::env::var(&env_key) {
            return Some((value, PropertySource::EnvVariable.as_str().to_string()));
        }
        let env_key_alt = format!("GRADLE_PROPERTY_{}", env_suffix);
        if let Ok(value) = std::env::var(&env_key_alt) {
            return Some((value, PropertySource::EnvVariable.as_str().to_string()));
        }
        // Direct env var match (lower precedence).
        if let Ok(value) = std::env::var(property_name) {
            return Some((value, PropertySource::EnvVariable.as_str().to_string()));
        }

        // --- Layers 4-6: Project-scoped layers ---
        if let Some(project) = self.projects.get(project_path) {
            // Convention mapping: if the requested name matches a convention
            // pattern, try the mapped key at each project layer.
            let effective_key = CONVENTION_PROPERTIES
                .iter()
                .find(|(pat, _)| *pat == property_name)
                .map(|(_, mapped)| *mapped)
                .unwrap_or(property_name);

            for source in &[
                PropertySource::GradleProperties,
                PropertySource::BuildScript,
                PropertySource::Extra,
            ] {
                if let Some(value) = project.get_from_layer(*source, effective_key) {
                    return Some((value.clone(), source.as_str().to_string()));
                }
            }
        }

        None
    }

    /// Check whether a property exists (any layer).
    #[cfg(test)]
    fn has_property_internal(
        &self,
        project_path: &str,
        property_name: &str,
        is_ext_access: bool,
    ) -> bool {
        self.resolve_property_internal(project_path, property_name, is_ext_access)
            .is_some()
    }

    /// Get all properties for a project (highest-precedence wins per key).
    pub fn get_project_properties(&self, project_path: &str) -> HashMap<String, String> {
        self.projects
            .get(project_path)
            .map(|p| {
                p.all_properties_sorted()
                    .into_iter()
                    .map(|(k, v, _)| (k, v))
                    .collect()
            })
            .unwrap_or_default()
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
        let keys: Vec<String> = self
            .config_cache
            .iter()
            .take(to_remove)
            .map(|e| e.key().clone())
            .collect();
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

// ---------------------------------------------------------------------------
// gRPC service implementation
// ---------------------------------------------------------------------------

#[tonic::async_trait]
impl ConfigurationService for ConfigurationServiceImpl {
    // -- RegisterProject ----------------------------------------------------

    async fn register_project(
        &self,
        request: Request<RegisterProjectRequest>,
    ) -> Result<Response<RegisterProjectResponse>, Status> {
        let req = request.into_inner();

        let mut gradle_props: HashMap<String, String> = req.properties.into_iter().collect();

        // Auto-populate the "project" property from the project path.
        if !gradle_props.contains_key("project") {
            gradle_props.insert(
                "project".to_string(),
                req.project_path
                    .strip_prefix(':')
                    .unwrap_or(&req.project_path)
                    .to_string(),
            );
        }
        // Auto-populate project.path.
        if !gradle_props.contains_key("project.path") {
            gradle_props.insert("project.path".to_string(), req.project_path.clone());
        }

        let state = ProjectState::new(req.project_dir, gradle_props, req.applied_plugins);
        self.projects.insert(req.project_path.clone(), state);

        tracing::debug!(project = %req.project_path, "Registered project");

        Ok(Response::new(RegisterProjectResponse { success: true }))
    }

    // -- GetProperty --------------------------------------------------------

    async fn get_property(
        &self,
        request: Request<GetPropertyRequest>,
    ) -> Result<Response<GetPropertyResponse>, Status> {
        let req = request.into_inner();
        self.property_resolutions.fetch_add(1, Ordering::Relaxed);

        let (property_name, is_ext) = normalize_access_pattern(&req.property_name);
        let _access_pattern = req.access_pattern;

        if let Some((value, source)) =
            self.resolve_property_internal(&req.project_path, &property_name, is_ext)
        {
            self.property_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Response::new(GetPropertyResponse {
                value,
                source,
                found: true,
            }));
        }

        Ok(Response::new(GetPropertyResponse {
            value: String::new(),
            source: String::new(),
            found: false,
        }))
    }

    // -- SetProperty --------------------------------------------------------

    async fn set_property(
        &self,
        request: Request<SetPropertyRequest>,
    ) -> Result<Response<SetPropertyResponse>, Status> {
        let req = request.into_inner();

        let target =
            PropertySource::from_str_loose(&req.target_layer).unwrap_or(PropertySource::Extra);

        // Command-line properties are set globally, not per-project.
        if target == PropertySource::CommandLine {
            let previous = self
                .command_line_props
                .insert(req.property_name.clone(), req.value.clone());
            let had_previous = previous.is_some();
            let previous_value = previous.unwrap_or_default();
            return Ok(Response::new(SetPropertyResponse {
                success: true,
                previous_value,
                had_previous,
            }));
        }

        // System properties – set via env var.
        if target == PropertySource::SystemProperty {
            let env_key = format!("org.gradle.{}", req.property_name);
            let prev = std::env::var(&env_key).ok();
            let had_previous = prev.is_some();
            let previous_value = prev.unwrap_or_default();
            std::env::set_var(&env_key, &req.value);
            return Ok(Response::new(SetPropertyResponse {
                success: true,
                previous_value,
                had_previous,
            }));
        }

        let (property_name, _is_ext) = normalize_access_pattern(&req.property_name);

        let mut project = self
            .projects
            .get_mut(&req.project_path)
            .ok_or_else(|| Status::not_found(format!("Project not found: {}", req.project_path)))?;

        let previous = project.set_in_layer(target, property_name, req.value);
        let had_previous = previous.is_some();
        let previous_value = previous.unwrap_or_default();

        Ok(Response::new(SetPropertyResponse {
            success: true,
            previous_value,
            had_previous,
        }))
    }

    // -- ListProperties ------------------------------------------------------

    async fn list_properties(
        &self,
        request: Request<ListPropertiesRequest>,
    ) -> Result<Response<ListPropertiesResponse>, Status> {
        let req = request.into_inner();

        let project = self
            .projects
            .get(&req.project_path)
            .ok_or_else(|| Status::not_found(format!("Project not found: {}", req.project_path)))?;

        let filter = if req.filter_source.is_empty() {
            None
        } else {
            PropertySource::from_str_loose(&req.filter_source)
        };

        let mut entries: Vec<PropertyEntry> = Vec::with_capacity(64);

        if let Some(source_filter) = filter {
            // Single-layer listing.
            for (k, v) in project.list_layer(&source_filter) {
                entries.push(PropertyEntry {
                    key: k,
                    value: v,
                    source: source_filter.as_str().to_string(),
                });
            }
        } else {
            // All project layers (highest precedence first, deduped).
            for (k, v, source) in project.all_properties_sorted() {
                entries.push(PropertyEntry {
                    key: k,
                    value: v,
                    source: source.as_str().to_string(),
                });
            }
        }

        // Optionally include command-line properties.
        if filter.is_none() && req.include_inherited {
            let mut seen_keys: HashSet<String> = entries.iter().map(|e| e.key.clone()).collect();
            for kv in self.command_line_props.iter() {
                if seen_keys.insert(kv.key().clone()) {
                    entries.push(PropertyEntry {
                        key: kv.key().clone(),
                        value: kv.value().clone(),
                        source: PropertySource::CommandLine.as_str().to_string(),
                    });
                }
            }
        }

        Ok(Response::new(ListPropertiesResponse { entries }))
    }

    // -- ResolveProperties (interpolation) -----------------------------------

    async fn resolve_properties(
        &self,
        request: Request<ResolvePropertiesRequest>,
    ) -> Result<Response<ResolvePropertiesResponse>, Status> {
        let req = request.into_inner();

        let project_path = req.project_path.clone();
        let resolve = |name: &str| -> Option<String> {
            self.resolve_property_internal(&project_path, name, false)
                .map(|(v, _)| v)
        };
        let results: Vec<ResolvedProperty> = req
            .templates
            .into_iter()
            .map(|template| match interpolate_template(&template, &resolve) {
                Ok(resolved) => ResolvedProperty {
                    template: template.clone(),
                    resolved_value: resolved,
                    success: true,
                    error: String::new(),
                },
                Err(e) => ResolvedProperty {
                    template: template.clone(),
                    resolved_value: String::new(),
                    success: false,
                    error: e,
                },
            })
            .collect();

        Ok(Response::new(ResolvePropertiesResponse {
            resolved: results,
        }))
    }

    // -- ResolveProperty (legacy, preserved for backward compat) -------------

    async fn resolve_property(
        &self,
        request: Request<ResolvePropertyRequest>,
    ) -> Result<Response<ResolvePropertyResponse>, Status> {
        let req = request.into_inner();
        self.property_resolutions.fetch_add(1, Ordering::Relaxed);

        if let Some((value, source)) =
            self.resolve_property_internal(&req.project_path, &req.property_name, false)
        {
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

    // -- CacheConfiguration -------------------------------------------------

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

    // -- ValidateConfigCache ------------------------------------------------

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

    // -- GetProjectInfo -----------------------------------------------------

    async fn get_project_info(
        &self,
        request: Request<GetProjectInfoRequest>,
    ) -> Result<Response<ProjectInfo>, Status> {
        let req = request.into_inner();

        match self.projects.get(&req.project_path) {
            Some(project) => {
                let properties = project
                    .all_properties_sorted()
                    .into_iter()
                    .map(|(k, v, _)| (k, v))
                    .collect();
                Ok(Response::new(ProjectInfo {
                    project_path: req.project_path,
                    project_dir: project.project_dir.clone(),
                    properties,
                    applied_plugins: project.applied_plugins.clone(),
                }))
            }
            None => Err(Status::not_found(format!(
                "Project not found: {}",
                req.project_path
            ))),
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- helpers ------------------------------------------------------------

    async fn register_test_project(
        svc: &ConfigurationServiceImpl,
        path: &str,
        props: HashMap<&str, &str>,
    ) {
        let props: HashMap<String, String> = props
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: path.to_string(),
            project_dir: format!("/tmp/{}", path.trim_start_matches(':')),
            properties: props,
            applied_plugins: vec![],
        }))
        .await
        .unwrap();
    }

    async fn register_test_project_with_plugins(
        svc: &ConfigurationServiceImpl,
        path: &str,
        props: HashMap<&str, &str>,
        plugins: Vec<&str>,
    ) {
        let props: HashMap<String, String> = props
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let plugins: Vec<String> = plugins.iter().map(|s| s.to_string()).collect();
        svc.register_project(Request::new(RegisterProjectRequest {
            project_path: path.to_string(),
            project_dir: format!("/tmp/{}", path.trim_start_matches(':')),
            properties: props,
            applied_plugins: plugins,
        }))
        .await
        .unwrap();
    }

    async fn resolve_via_get(
        svc: &ConfigurationServiceImpl,
        project: &str,
        name: &str,
    ) -> GetPropertyResponse {
        svc.get_property(Request::new(GetPropertyRequest {
            project_path: project.to_string(),
            property_name: name.to_string(),
            access_pattern: String::new(),
        }))
        .await
        .unwrap()
        .into_inner()
    }

    // -- normalisation tests ------------------------------------------------

    #[test]
    fn test_normalize_bare_name() {
        let (key, ext) = normalize_access_pattern("myProp");
        assert_eq!(key, "myProp");
        assert!(!ext);
    }

    #[test]
    fn test_normalize_project_property_single_quote() {
        let (key, ext) = normalize_access_pattern("project.property('foo.bar')");
        assert_eq!(key, "foo.bar");
        assert!(!ext);
    }

    #[test]
    fn test_normalize_project_property_double_quote() {
        let (key, ext) = normalize_access_pattern("project.property(\"foo.bar\")");
        assert_eq!(key, "foo.bar");
        assert!(!ext);
    }

    #[test]
    fn test_normalize_has_property() {
        let (key, ext) = normalize_access_pattern("project.hasProperty('debug')");
        assert_eq!(key, "debug");
        assert!(!ext);
    }

    #[test]
    fn test_normalize_ext_access() {
        let (key, ext) = normalize_access_pattern("project.ext.customProp");
        assert_eq!(key, "customProp");
        assert!(ext);
    }

    #[test]
    fn test_normalize_ext_shorthand() {
        let (key, ext) = normalize_access_pattern("ext.customProp");
        assert_eq!(key, "customProp");
        assert!(ext);
    }

    #[test]
    fn test_normalize_convention_project_name() {
        let (key, ext) = normalize_access_pattern("project.name");
        assert_eq!(key, "project");
        assert!(!ext);
    }

    #[test]
    fn test_normalize_convention_project_version() {
        let (key, ext) = normalize_access_pattern("project.version");
        assert_eq!(key, "version");
        assert!(!ext);
    }

    #[test]
    fn test_normalize_project_dot_arbitrary() {
        let (key, ext) = normalize_access_pattern("project.someArbitrary");
        assert_eq!(key, "someArbitrary");
        assert!(!ext);
    }

    // -- PropertySource tests ------------------------------------------------

    #[test]
    fn test_property_source_precedence_ordering() {
        assert!(PropertySource::CommandLine > PropertySource::SystemProperty);
        assert!(PropertySource::SystemProperty > PropertySource::EnvVariable);
        assert!(PropertySource::EnvVariable > PropertySource::GradleProperties);
        assert!(PropertySource::GradleProperties > PropertySource::Extra);
        assert!(PropertySource::Extra > PropertySource::BuildScript);
    }

    #[test]
    fn test_property_source_from_str() {
        assert_eq!(
            PropertySource::from_str_loose("command_line"),
            Some(PropertySource::CommandLine)
        );
        assert_eq!(
            PropertySource::from_str_loose("system-property"),
            Some(PropertySource::SystemProperty)
        );
        assert_eq!(
            PropertySource::from_str_loose("env"),
            Some(PropertySource::EnvVariable)
        );
        assert_eq!(
            PropertySource::from_str_loose("gradle"),
            Some(PropertySource::GradleProperties)
        );
        assert_eq!(
            PropertySource::from_str_loose("build_script"),
            Some(PropertySource::BuildScript)
        );
        assert_eq!(
            PropertySource::from_str_loose("extra"),
            Some(PropertySource::Extra)
        );
        assert_eq!(PropertySource::from_str_loose("bogus"), None);
    }

    // -- Core layered resolution tests ---------------------------------------

    #[tokio::test]
    async fn test_register_and_get_property() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(
            &svc,
            ":app",
            vec![("version", "1.0"), ("sourceCompatibility", "17")]
                .into_iter()
                .collect(),
        )
        .await;

        let resp = resolve_via_get(&svc, ":app", "version").await;
        assert!(resp.found);
        assert_eq!(resp.value, "1.0");
        assert_eq!(resp.source, "gradle_properties");
    }

    #[tokio::test]
    async fn test_missing_property() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", HashMap::new()).await;

        let resp = resolve_via_get(&svc, ":app", "nonexistent").await;
        assert!(!resp.found);
    }

    #[tokio::test]
    async fn test_command_line_overrides_gradle_properties() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", vec![("debug", "false")].into_iter().collect()).await;
        svc.set_command_line_properties(
            vec![("debug".to_string(), "true".to_string())]
                .into_iter()
                .collect(),
        );

        let resp = resolve_via_get(&svc, ":app", "debug").await;
        assert!(resp.found);
        assert_eq!(resp.value, "true");
        assert_eq!(resp.source, "command_line");
    }

    #[tokio::test]
    async fn test_system_property_overrides_env() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", HashMap::new()).await;

        std::env::set_var("ORG_GRADLE_PROJECT_MY_PROP", "env_val");
        std::env::set_var("org.gradle.my.prop", "sys_val");

        let resp = resolve_via_get(&svc, ":app", "my.prop").await;
        assert!(resp.found);
        assert_eq!(resp.value, "sys_val");
        assert_eq!(resp.source, "system_property");

        std::env::remove_var("ORG_GRADLE_PROJECT_MY_PROP");
        std::env::remove_var("org.gradle.my.prop");
    }

    #[tokio::test]
    async fn test_env_variable_fallback() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", HashMap::new()).await;

        std::env::set_var("ORG_GRADLE_PROJECT_CUSTOM_PROP", "env_value");
        let resp = resolve_via_get(&svc, ":app", "custom.prop").await;

        std::env::remove_var("ORG_GRADLE_PROJECT_CUSTOM_PROP");

        assert!(resp.found);
        assert_eq!(resp.value, "env_value");
        assert_eq!(resp.source, "env_variable");
    }

    #[tokio::test]
    async fn test_gradle_property_env_prefix_fallback() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", HashMap::new()).await;

        std::env::set_var("GRADLE_PROPERTY_CUSTOM_PROP", "env_value");
        let resp = resolve_via_get(&svc, ":app", "custom.prop").await;

        std::env::remove_var("GRADLE_PROPERTY_CUSTOM_PROP");

        assert!(resp.found);
        assert_eq!(resp.value, "env_value");
        assert_eq!(resp.source, "env_variable");
    }

    #[tokio::test]
    async fn test_convention_property_project_name() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(
            &svc,
            ":app",
            vec![("project", "my-app")].into_iter().collect(),
        )
        .await;

        let resp = resolve_via_get(&svc, ":app", "project.name").await;
        assert!(resp.found);
        assert_eq!(resp.value, "my-app");
        assert_eq!(resp.source, "gradle_properties");
    }

    #[tokio::test]
    async fn test_convention_property_project_version() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(
            &svc,
            ":app",
            vec![("version", "2.0.0")].into_iter().collect(),
        )
        .await;

        let resp = resolve_via_get(&svc, ":app", "project.version").await;
        assert!(resp.found);
        assert_eq!(resp.value, "2.0.0");
    }

    #[tokio::test]
    async fn test_auto_derive_project_name() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":my-app", HashMap::new()).await;

        let resp = resolve_via_get(&svc, ":my-app", "project").await;
        assert!(resp.found);
        assert_eq!(resp.value, "my-app");

        let resp = resolve_via_get(&svc, ":my-app", "project.name").await;
        assert!(resp.found);
        assert_eq!(resp.value, "my-app");
    }

    #[tokio::test]
    async fn test_ext_property_isolation() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(
            &svc,
            ":app",
            vec![("foo", "from_gradle_props")].into_iter().collect(),
        )
        .await;

        // Set an extra property.
        svc.set_property(Request::new(SetPropertyRequest {
            project_path: ":app".to_string(),
            property_name: "project.ext.foo".to_string(),
            value: "from_extra".to_string(),
            target_layer: "extra".to_string(),
        }))
        .await
        .unwrap();

        // Direct access to "foo" should return gradle_properties (higher prec).
        let resp = resolve_via_get(&svc, ":app", "foo").await;
        assert!(resp.found);
        assert_eq!(resp.value, "from_gradle_props");
        assert_eq!(resp.source, "gradle_properties");

        // project.ext.foo should return extra.
        let resp = resolve_via_get(&svc, ":app", "project.ext.foo").await;
        assert!(resp.found);
        assert_eq!(resp.value, "from_extra");
        assert_eq!(resp.source, "extra");
    }

    #[tokio::test]
    async fn test_has_property_via_access_pattern() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", vec![("debug", "true")].into_iter().collect()).await;

        assert!(svc.has_property_internal(":app", "debug", false));
        assert!(!svc.has_property_internal(":app", "missing", false));
    }

    // -- SetProperty tests --------------------------------------------------

    #[tokio::test]
    async fn test_set_property_returns_previous() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", vec![("version", "1.0")].into_iter().collect()).await;

        let resp = svc
            .set_property(Request::new(SetPropertyRequest {
                project_path: ":app".to_string(),
                property_name: "version".to_string(),
                value: "2.0".to_string(),
                target_layer: "build_script".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert!(!resp.had_previous); // different layer, so no previous in that layer
    }

    #[tokio::test]
    async fn test_set_property_in_same_layer_updates() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", vec![("version", "1.0")].into_iter().collect()).await;

        // Set in gradle_properties layer (same layer as registration).
        let resp = svc
            .set_property(Request::new(SetPropertyRequest {
                project_path: ":app".to_string(),
                property_name: "version".to_string(),
                value: "3.0".to_string(),
                target_layer: "gradle_properties".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert!(resp.had_previous);
        assert_eq!(resp.previous_value, "1.0");

        // Verify updated value.
        let get_resp = resolve_via_get(&svc, ":app", "version").await;
        assert_eq!(get_resp.value, "3.0");
    }

    #[tokio::test]
    async fn test_set_command_line_property() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", vec![("debug", "false")].into_iter().collect()).await;

        let resp = svc
            .set_property(Request::new(SetPropertyRequest {
                project_path: ":app".to_string(),
                property_name: "debug".to_string(),
                value: "true".to_string(),
                target_layer: "command_line".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert!(!resp.had_previous);

        // Command-line should now win.
        let get_resp = resolve_via_get(&svc, ":app", "debug").await;
        assert_eq!(get_resp.value, "true");
        assert_eq!(get_resp.source, "command_line");
    }

    #[tokio::test]
    async fn test_set_property_unknown_project() {
        let svc = ConfigurationServiceImpl::new();
        let result = svc
            .set_property(Request::new(SetPropertyRequest {
                project_path: ":nonexistent".to_string(),
                property_name: "x".to_string(),
                value: "y".to_string(),
                target_layer: "extra".to_string(),
            }))
            .await;
        assert!(result.is_err());
    }

    // -- ListProperties tests ------------------------------------------------

    #[tokio::test]
    async fn test_list_properties_all() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(
            &svc,
            ":app",
            vec![("version", "1.0"), ("group", "com.example")]
                .into_iter()
                .collect(),
        )
        .await;

        let resp = svc
            .list_properties(Request::new(ListPropertiesRequest {
                project_path: ":app".to_string(),
                filter_source: String::new(),
                include_inherited: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Should have at least version, group, project (auto-derived), project.path
        assert!(resp.entries.len() >= 3);
        let keys: Vec<&str> = resp.entries.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"version"));
        assert!(keys.contains(&"group"));
    }

    #[tokio::test]
    async fn test_list_properties_filter_by_source() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", vec![("version", "1.0")].into_iter().collect()).await;

        // Set an extra property.
        svc.set_property(Request::new(SetPropertyRequest {
            project_path: ":app".to_string(),
            property_name: "myExtra".to_string(),
            value: "extraVal".to_string(),
            target_layer: "extra".to_string(),
        }))
        .await
        .unwrap();

        let resp = svc
            .list_properties(Request::new(ListPropertiesRequest {
                project_path: ":app".to_string(),
                filter_source: "extra".to_string(),
                include_inherited: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.entries.len(), 1);
        assert_eq!(resp.entries[0].key, "myExtra");
        assert_eq!(resp.entries[0].value, "extraVal");
        assert_eq!(resp.entries[0].source, "extra");
    }

    #[tokio::test]
    async fn test_list_properties_includes_command_line() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", vec![("version", "1.0")].into_iter().collect()).await;
        svc.set_command_line_properties(
            vec![("debug".to_string(), "true".to_string())]
                .into_iter()
                .collect(),
        );

        let resp = svc
            .list_properties(Request::new(ListPropertiesRequest {
                project_path: ":app".to_string(),
                filter_source: String::new(),
                include_inherited: true,
            }))
            .await
            .unwrap()
            .into_inner();

        let keys: Vec<&str> = resp.entries.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"debug"));
    }

    #[tokio::test]
    async fn test_list_properties_unknown_project() {
        let svc = ConfigurationServiceImpl::new();
        let result = svc
            .list_properties(Request::new(ListPropertiesRequest {
                project_path: ":nonexistent".to_string(),
                filter_source: String::new(),
                include_inherited: false,
            }))
            .await;
        assert!(result.is_err());
    }

    // -- Interpolation tests ------------------------------------------------

    #[test]
    fn test_interpolate_simple() {
        let resolve = |name: &str| -> Option<String> {
            match name {
                "version" => Some("1.0".to_string()),
                _ => None,
            }
        };
        let result = interpolate_template("${version}", &resolve).unwrap();
        assert_eq!(result, "1.0");
    }

    #[test]
    fn test_interpolate_embedded() {
        let resolve = |name: &str| -> Option<String> {
            match name {
                "group" => Some("com.example".to_string()),
                "name" => Some("mylib".to_string()),
                _ => None,
            }
        };
        let result =
            interpolate_template("${group}:${name}:${version:-SNAPSHOT}", &resolve).unwrap();
        assert_eq!(result, "com.example:mylib:SNAPSHOT");
    }

    #[test]
    fn test_interpolate_no_refs() {
        let resolve = |_name: &str| -> Option<String> { None };
        let result = interpolate_template("plain string", &resolve).unwrap();
        assert_eq!(result, "plain string");
    }

    #[test]
    fn test_interpolate_missing_without_default() {
        let resolve = |_name: &str| -> Option<String> { None };
        let result = interpolate_template("${missing}", &resolve);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing"));
    }

    #[test]
    fn test_interpolate_nested() {
        let resolve = |name: &str| -> Option<String> {
            match name {
                "base" => Some("/opt/app".to_string()),
                "dir" => Some("${base}/lib".to_string()),
                _ => None,
            }
        };
        let result = interpolate_template("${dir}/app.jar", &resolve).unwrap();
        assert_eq!(result, "/opt/app/lib/app.jar");
    }

    #[test]
    fn test_interpolate_circular_detection() {
        let resolve = |name: &str| -> Option<String> {
            match name {
                "a" => Some("${b}".to_string()),
                "b" => Some("${a}".to_string()),
                _ => None,
            }
        };
        let result = interpolate_template("${a}", &resolve);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Max interpolation depth"));
    }

    #[test]
    fn test_interpolate_unclosed_brace() {
        let resolve = |name: &str| -> Option<String> {
            match name {
                "x" => Some("val".to_string()),
                _ => None,
            }
        };
        let result = interpolate_template("prefix ${x unclosed", &resolve).unwrap();
        // Unclosed braces are emitted literally.
        assert_eq!(result, "prefix ${x unclosed");
    }

    #[test]
    fn test_interpolate_empty_expr() {
        let resolve = |_name: &str| -> Option<String> { None };
        let result = interpolate_template("${}", &resolve);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_resolve_properties_rpc() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(
            &svc,
            ":app",
            vec![
                ("group", "com.example"),
                ("name", "mylib"),
                ("version", "1.0"),
            ]
            .into_iter()
            .collect(),
        )
        .await;

        let resp = svc
            .resolve_properties(Request::new(ResolvePropertiesRequest {
                project_path: ":app".to_string(),
                templates: vec![
                    "${group}:${name}:${version}".to_string(),
                    "${group}:${name}:${version:-SNAPSHOT}".to_string(),
                    "no refs here".to_string(),
                    "${missing}".to_string(),
                ],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.resolved.len(), 4);
        assert_eq!(resp.resolved[0].resolved_value, "com.example:mylib:1.0");
        assert!(resp.resolved[0].success);

        assert_eq!(resp.resolved[1].resolved_value, "com.example:mylib:1.0");
        assert!(resp.resolved[1].success);

        assert_eq!(resp.resolved[2].resolved_value, "no refs here");
        assert!(resp.resolved[2].success);

        assert!(!resp.resolved[3].success);
        assert!(resp.resolved[3].error.contains("missing"));
    }

    // -- Full precedence chain test -----------------------------------------

    #[tokio::test]
    async fn test_full_precedence_chain() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(
            &svc,
            ":app",
            vec![("prop", "gradle_properties")].into_iter().collect(),
        )
        .await;

        // Set at build_script layer.
        svc.set_property(Request::new(SetPropertyRequest {
            project_path: ":app".to_string(),
            property_name: "prop".to_string(),
            value: "build_script".to_string(),
            target_layer: "build_script".to_string(),
        }))
        .await
        .unwrap();

        // Set at extra layer.
        svc.set_property(Request::new(SetPropertyRequest {
            project_path: ":app".to_string(),
            property_name: "prop".to_string(),
            value: "extra".to_string(),
            target_layer: "extra".to_string(),
        }))
        .await
        .unwrap();

        // Without any higher layer, gradle_properties wins (registered first).
        let resp = resolve_via_get(&svc, ":app", "prop").await;
        assert_eq!(resp.value, "gradle_properties");
        assert_eq!(resp.source, "gradle_properties");

        // Now add env var.
        std::env::set_var("ORG_GRADLE_PROJECT_PROP", "env_variable");
        let resp = resolve_via_get(&svc, ":app", "prop").await;
        assert_eq!(resp.value, "env_variable");
        assert_eq!(resp.source, "env_variable");
        std::env::remove_var("ORG_GRADLE_PROJECT_PROP");

        // Add system property.
        std::env::set_var("org.gradle.prop", "system_property");
        let resp = resolve_via_get(&svc, ":app", "prop").await;
        assert_eq!(resp.value, "system_property");
        assert_eq!(resp.source, "system_property");
        std::env::remove_var("org.gradle.prop");

        // Add command-line (highest).
        svc.set_command_line_properties(
            vec![("prop".to_string(), "command_line".to_string())]
                .into_iter()
                .collect(),
        );
        let resp = resolve_via_get(&svc, ":app", "prop").await;
        assert_eq!(resp.value, "command_line");
        assert_eq!(resp.source, "command_line");
    }

    // -- Legacy ResolveProperty RPC tests (backward compat) -----------------

    #[tokio::test]
    async fn test_legacy_resolve_property() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project(&svc, ":app", vec![("version", "1.0")].into_iter().collect()).await;

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
    }

    // -- Config cache tests -------------------------------------------------

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

        svc.cache_configuration(Request::new(CacheConfigurationRequest {
            project_path: ":app".to_string(),
            config_hash: vec![1, 2, 3],
            timestamp_ms: 100,
        }))
        .await
        .unwrap();

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
    async fn test_cache_eviction() {
        let svc = ConfigurationServiceImpl::new();

        for i in 0..(MAX_CACHED_CONFIGS + 100) {
            svc.cache_configuration(Request::new(CacheConfigurationRequest {
                project_path: format!(":project-{}", i),
                config_hash: vec![i as u8],
                timestamp_ms: 100,
            }))
            .await
            .unwrap();
        }

        assert!(svc.config_cache.len() <= MAX_CACHED_CONFIGS);
    }

    // -- Stats tests --------------------------------------------------------

    #[tokio::test]
    async fn test_configuration_stats() {
        let svc = ConfigurationServiceImpl::new();

        register_test_project(
            &svc,
            ":app",
            vec![("version", "1.0"), ("group", "com.example")]
                .into_iter()
                .collect(),
        )
        .await;

        // Resolve hit
        let _ = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":app".to_string(),
                property_name: "version".to_string(),
                requested_by: "test".to_string(),
            }))
            .await
            .unwrap();

        // Resolve miss
        let _ = svc
            .resolve_property(Request::new(ResolvePropertyRequest {
                project_path: ":app".to_string(),
                property_name: "missing".to_string(),
                requested_by: "test".to_string(),
            }))
            .await
            .unwrap();

        svc.cache_configuration(Request::new(CacheConfigurationRequest {
            project_path: ":app".to_string(),
            config_hash: vec![1, 2, 3],
            timestamp_ms: 100,
        }))
        .await
        .unwrap();

        svc.validate_config_cache(Request::new(ValidateConfigCacheRequest {
            project_path: ":app".to_string(),
            expected_hash: vec![1, 2, 3],
            input_files: vec![],
            build_script_hashes: vec![],
        }))
        .await
        .unwrap();

        svc.validate_config_cache(Request::new(ValidateConfigCacheRequest {
            project_path: ":app".to_string(),
            expected_hash: vec![9, 9, 9],
            input_files: vec![],
            build_script_hashes: vec![],
        }))
        .await
        .unwrap();

        let stats = svc.get_stats();
        assert_eq!(stats.registered_projects, 1);
        assert_eq!(stats.cached_configs, 1);
        assert_eq!(stats.property_resolutions, 2);
        assert_eq!(stats.property_hits, 1);
        assert!((stats.property_hit_rate - 0.5).abs() < f64::EPSILON);
        assert_eq!(stats.cache_validations, 2);
        assert_eq!(stats.cache_hits, 1);
    }

    // -- GetProjectInfo tests -----------------------------------------------

    #[tokio::test]
    async fn test_get_project_info() {
        let svc = ConfigurationServiceImpl::new();
        register_test_project_with_plugins(
            &svc,
            ":app",
            vec![("version", "1.0")].into_iter().collect(),
            vec!["java", "idea"],
        )
        .await;

        let resp = svc
            .get_project_info(Request::new(GetProjectInfoRequest {
                project_path: ":app".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.project_path, ":app");
        assert_eq!(resp.project_dir, "/tmp/app");
        assert!(resp.properties.contains_key("version"));
        assert!(resp.applied_plugins.contains(&"java".to_string()));
        assert!(resp.applied_plugins.contains(&"idea".to_string()));
    }

    #[tokio::test]
    async fn test_get_project_info_not_found() {
        let svc = ConfigurationServiceImpl::new();
        let result = svc
            .get_project_info(Request::new(GetProjectInfoRequest {
                project_path: ":nonexistent".to_string(),
            }))
            .await;
        assert!(result.is_err());
    }
}
