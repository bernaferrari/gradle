//! Strict public/internal API boundary enforcement.
//!
//! This module documents and enforces the visibility boundaries between
//! public, internal, and private API items, mirroring Gradle's stable
//! package documentation and `.internal` convention.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Visibility levels
// ---------------------------------------------------------------------------

/// Visibility level of an API item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisibilityLevel {
    /// Stable API — backward compatible, documented for external consumers.
    Public,
    /// Unstable API — may change without notice, not for external use.
    Internal,
    /// Implementation detail — not exposed outside the defining module.
    Private,
}

// ---------------------------------------------------------------------------
// Stability levels
// ---------------------------------------------------------------------------

/// Stability guarantee of an API item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiStability {
    /// Guaranteed backward compatible.
    Stable,
    /// Mostly stable, may have breaking changes.
    Beta,
    /// Experimental, no guarantees.
    Alpha,
    /// Will be removed in the next major version.
    Deprecated,
}

// ---------------------------------------------------------------------------
// API kind
// ---------------------------------------------------------------------------

/// Category of an API item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiKind {
    /// gRPC service implementation.
    Service,
    /// Proto message type.
    Message,
    /// Public function.
    Function,
    /// Public type (struct/enum).
    Type,
    /// Public constant.
    Constant,
    /// Public trait.
    Trait,
}

// ---------------------------------------------------------------------------
// API item
// ---------------------------------------------------------------------------

/// Documents a public API item in the substrate.
#[derive(Debug, Clone, Copy)]
pub struct ApiItem {
    pub name: &'static str,
    pub kind: ApiKind,
    pub visibility: VisibilityLevel,
    pub stability: ApiStability,
    pub since_version: &'static str,
    pub description: &'static str,
}

// ---------------------------------------------------------------------------
// PUBLIC_API registry
// ---------------------------------------------------------------------------

/// Static registry of all public API items.
///
/// This is the authoritative source of truth for what constitutes the
/// public surface of the substrate. Items not listed here are considered
/// internal or private.
pub const PUBLIC_API: &[ApiItem] = &[
    // === Services ===
    ApiItem {
        name: "HashService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "File hashing service with multiple algorithm support (MD5, SHA-256, SHA-512)",
    },
    ApiItem {
        name: "CacheService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Build cache operations — store, fetch, and query cached artifacts",
    },
    ApiItem {
        name: "ExecService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Process execution service — spawn and manage external processes",
    },
    ApiItem {
        name: "WorkService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Work item management — enqueue, dequeue, and track work units",
    },
    ApiItem {
        name: "ExecutionPlanService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Execution plan construction and querying",
    },
    ApiItem {
        name: "ExecutionHistoryService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Historical execution data — previous runs, outcomes, and durations",
    },
    ApiItem {
        name: "BuildCacheOrchestrationService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Coordinates local and remote build caches with eviction policies",
    },
    ApiItem {
        name: "FileFingerprintService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "File fingerprinting for incremental build — content hash and metadata",
    },
    ApiItem {
        name: "ValueSnapshotService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description:
            "Value snapshotting for up-to-date checks — serializes and compares input/output values",
    },
    ApiItem {
        name: "TaskGraphService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Task graph construction, traversal, and dependency resolution",
    },
    ApiItem {
        name: "ConfigurationService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Build configuration — project model, dependencies, and variants",
    },
    ApiItem {
        name: "ConfigurationCacheService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Configuration cache — serialize and reuse configuration phase results",
    },
    ApiItem {
        name: "BootstrapService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Daemon bootstrap — initialization, classpath resolution, and JVM startup",
    },
    ApiItem {
        name: "ControlService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Daemon lifecycle control — stop, status, and health checks",
    },
    ApiItem {
        name: "BuildEventStreamService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Build event streaming — progress, lifecycle, and outcome events",
    },
    ApiItem {
        name: "DagExecutorService",
        kind: ApiKind::Service,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "DAG-based task executor — parallel execution with dependency ordering",
    },
    // === Build Plan IR types ===
    ApiItem {
        name: "CanonicalBuildPlan",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Canonical intermediate representation of a build plan",
    },
    ApiItem {
        name: "BuildPlanNode",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "A node in the build plan IR — represents a task or action",
    },
    ApiItem {
        name: "BuildPlanEdge",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "A dependency edge between build plan nodes",
    },
    // === Scope types ===
    ApiItem {
        name: "BuildId",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Unique identifier for a build invocation",
    },
    ApiItem {
        name: "SessionId",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Unique identifier for a daemon session",
    },
    ApiItem {
        name: "TreeId",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Unique identifier for a project tree",
    },
    ApiItem {
        name: "ProjectPath",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Path to a project within a multi-project build",
    },
    // === Protocol constants ===
    ApiItem {
        name: "PROTOCOL_VERSION",
        kind: ApiKind::Constant,
        visibility: VisibilityLevel::Public,
        stability: ApiStability::Stable,
        since_version: "0.1.0",
        description: "Current gRPC protocol version between Rust substrate and JVM host",
    },
    // === Internal items (documented but marked internal) ===
    ApiItem {
        name: "InternalCacheStore",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Internal,
        stability: ApiStability::Beta,
        since_version: "0.1.0",
        description: "Internal cache storage implementation — may change without notice",
    },
    ApiItem {
        name: "ShadowValidator",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Internal,
        stability: ApiStability::Alpha,
        since_version: "0.2.0",
        description: "Shadow mode validator for comparing Rust vs JVM outputs",
    },
    ApiItem {
        name: "JvmHostClient",
        kind: ApiKind::Type,
        visibility: VisibilityLevel::Internal,
        stability: ApiStability::Beta,
        since_version: "0.1.0",
        description: "Internal gRPC client for JVM host callbacks",
    },
];

// ---------------------------------------------------------------------------
// ApiRegistry — runtime lookup
// ---------------------------------------------------------------------------

/// Runtime registry for API lookups, built from `PUBLIC_API`.
pub struct ApiRegistry {
    items_by_name: HashMap<String, &'static ApiItem>,
    items_by_visibility: HashMap<VisibilityLevel, Vec<&'static ApiItem>>,
    items_by_stability: HashMap<ApiStability, Vec<&'static ApiItem>>,
}

impl ApiRegistry {
    /// Builds indexes from `PUBLIC_API`.
    pub fn new() -> Self {
        let mut items_by_name = HashMap::new();
        let mut items_by_visibility: HashMap<VisibilityLevel, Vec<&'static ApiItem>> =
            HashMap::new();
        let mut items_by_stability: HashMap<ApiStability, Vec<&'static ApiItem>> = HashMap::new();

        for item in PUBLIC_API.iter() {
            items_by_name.insert(item.name.to_string(), item);
            items_by_visibility
                .entry(item.visibility)
                .or_default()
                .push(item);
            items_by_stability
                .entry(item.stability)
                .or_default()
                .push(item);
        }

        Self {
            items_by_name,
            items_by_visibility,
            items_by_stability,
        }
    }

    /// Look up an API item by name.
    pub fn get(&self, name: &str) -> Option<&'static ApiItem> {
        self.items_by_name.get(name).copied()
    }

    /// Get all items with the given visibility level.
    pub fn by_visibility(&self, level: VisibilityLevel) -> Vec<&'static ApiItem> {
        self.items_by_visibility
            .get(&level)
            .cloned()
            .unwrap_or_default()
    }

    /// Get all items with the given stability level.
    pub fn by_stability(&self, stability: ApiStability) -> Vec<&'static ApiItem> {
        self.items_by_stability
            .get(&stability)
            .cloned()
            .unwrap_or_default()
    }

    /// Get all public items that are stable.
    pub fn stable_public_items(&self) -> Vec<&'static ApiItem> {
        PUBLIC_API
            .iter()
            .filter(|item| {
                item.visibility == VisibilityLevel::Public && item.stability == ApiStability::Stable
            })
            .collect()
    }
}

impl Default for ApiRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ApiViolation
// ---------------------------------------------------------------------------

/// Reports a boundary violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiViolation {
    pub source_module: String,
    pub target_item: String,
    pub expected_visibility: VisibilityLevel,
    pub actual_visibility: VisibilityLevel,
}

impl std::fmt::Display for ApiViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "API boundary violation: module '{}' accessed '{}' (expected {:?}, got {:?})",
            self.source_module, self.target_item, self.expected_visibility, self.actual_visibility
        )
    }
}

impl std::error::Error for ApiViolation {}

// ---------------------------------------------------------------------------
// validate_api_boundary
// ---------------------------------------------------------------------------

/// Checks if a module is allowed to access a target item based on visibility rules.
///
/// Rules:
/// - Public items can be accessed by any module.
/// - Internal items can only be accessed by modules in the same platform
///   (i.e., the source module path must contain the same top-level platform prefix).
/// - Private items can only be accessed within the same module
///   (i.e., the source module path must start with the target module path).
pub fn validate_api_boundary(source: &str, target: &str) -> Result<(), ApiViolation> {
    let registry = ApiRegistry::new();

    match registry.get(target) {
        Some(item) => match item.visibility {
            VisibilityLevel::Public => Ok(()),
            VisibilityLevel::Internal => {
                // Internal items: only accessible from within the substrate platform.
                if source.starts_with("substrate::") {
                    Ok(())
                } else {
                    Err(ApiViolation {
                        source_module: source.to_string(),
                        target_item: target.to_string(),
                        expected_visibility: VisibilityLevel::Public,
                        actual_visibility: VisibilityLevel::Internal,
                    })
                }
            }
            VisibilityLevel::Private => {
                // Private items: source must be within the same module hierarchy.
                let target_with_sep = format!("{}::", target);
                if source.starts_with(&target_with_sep) {
                    Ok(())
                } else {
                    Err(ApiViolation {
                        source_module: source.to_string(),
                        target_item: target.to_string(),
                        expected_visibility: VisibilityLevel::Private,
                        actual_visibility: VisibilityLevel::Private,
                    })
                }
            }
        },
        None => {
            // Unknown target: treat as a module path and check hierarchy.
            let target_with_sep = format!("{}::", target);
            if source.starts_with(&target_with_sep) {
                Ok(())
            } else {
                Err(ApiViolation {
                    source_module: source.to_string(),
                    target_item: target.to_string(),
                    expected_visibility: VisibilityLevel::Public,
                    actual_visibility: VisibilityLevel::Private,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// InternalModuleGuard
// ---------------------------------------------------------------------------

/// A marker type that documents a module as internal.
///
/// Place a `const` of this type at the top of an internal module to
/// make its internal status explicit and searchable.
pub struct InternalModuleGuard {
    pub module_path: &'static str,
    pub reason: &'static str,
}

impl InternalModuleGuard {
    pub const fn new(module_path: &'static str, reason: &'static str) -> Self {
        Self {
            module_path,
            reason,
        }
    }
}

// ---------------------------------------------------------------------------
// Macros
// ---------------------------------------------------------------------------

/// Marks a module as internal.
///
/// # Example
///
/// ```ignore
/// declare_internal_module! {
///     "server::cache::internal",
///     "Internal cache implementation details — do not use directly"
/// }
/// ```
#[macro_export]
macro_rules! declare_internal_module {
    ($module_path:expr, $reason:expr) => {
        #[allow(dead_code)]
        pub const INTERNAL_MODULE_GUARD: $crate::server::api_boundary::InternalModuleGuard =
            $crate::server::api_boundary::InternalModuleGuard::new($module_path, $reason);
    };
}

/// Documents a public API item at its definition site.
///
/// # Example
///
/// ```ignore
/// declare_public_api! {
///     HashService,
///     Service,
///     Stable,
///     "0.1.0",
///     "File hashing service"
/// }
/// ```
#[macro_export]
macro_rules! declare_public_api {
    ($name:ident, $kind:ident, $stability:ident, $since_version:expr, $description:expr) => {
        #[allow(dead_code)]
        pub const $name: $crate::server::api_boundary::ApiItem =
            $crate::server::api_boundary::ApiItem {
                name: stringify!($name),
                kind: $crate::server::api_boundary::ApiKind::$kind,
                visibility: $crate::server::api_boundary::VisibilityLevel::Public,
                stability: $crate::server::api_boundary::ApiStability::$stability,
                since_version: $since_version,
                description: $description,
            };
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- ApiRegistry indexing ---

    #[test]
    fn registry_indexing_works() {
        let registry = ApiRegistry::new();
        let item = registry.get("HashService");
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item.kind, ApiKind::Service);
        assert_eq!(item.visibility, VisibilityLevel::Public);
        assert_eq!(item.stability, ApiStability::Stable);
    }

    #[test]
    fn registry_returns_none_for_unknown() {
        let registry = ApiRegistry::new();
        assert!(registry.get("NonExistentService").is_none());
    }

    // --- by_visibility filtering ---

    #[test]
    fn by_visibility_filters_public_items() {
        let registry = ApiRegistry::new();
        let public_items = registry.by_visibility(VisibilityLevel::Public);
        assert!(!public_items.is_empty());
        for item in &public_items {
            assert_eq!(item.visibility, VisibilityLevel::Public);
        }
    }

    #[test]
    fn by_visibility_filters_internal_items() {
        let registry = ApiRegistry::new();
        let internal_items = registry.by_visibility(VisibilityLevel::Internal);
        assert!(!internal_items.is_empty());
        for item in &internal_items {
            assert_eq!(item.visibility, VisibilityLevel::Internal);
        }
    }

    #[test]
    fn by_visibility_returns_empty_for_private() {
        let registry = ApiRegistry::new();
        let private_items = registry.by_visibility(VisibilityLevel::Private);
        assert!(private_items.is_empty());
    }

    // --- by_stability filtering ---

    #[test]
    fn by_stability_filters_stable_items() {
        let registry = ApiRegistry::new();
        let stable_items = registry.by_stability(ApiStability::Stable);
        assert!(!stable_items.is_empty());
        for item in &stable_items {
            assert_eq!(item.stability, ApiStability::Stable);
        }
    }

    #[test]
    fn by_stability_filters_alpha_items() {
        let registry = ApiRegistry::new();
        let alpha_items = registry.by_stability(ApiStability::Alpha);
        assert!(!alpha_items.is_empty());
        for item in &alpha_items {
            assert_eq!(item.stability, ApiStability::Alpha);
        }
    }

    #[test]
    fn by_stability_filters_beta_items() {
        let registry = ApiRegistry::new();
        let beta_items = registry.by_stability(ApiStability::Beta);
        assert!(!beta_items.is_empty());
        for item in &beta_items {
            assert_eq!(item.stability, ApiStability::Beta);
        }
    }

    // --- stable_public_items ---

    #[test]
    fn stable_public_items_returns_only_stable_public() {
        let registry = ApiRegistry::new();
        let items = registry.stable_public_items();
        assert!(!items.is_empty());
        for item in &items {
            assert_eq!(item.visibility, VisibilityLevel::Public);
            assert_eq!(item.stability, ApiStability::Stable);
        }
    }

    // --- validate_api_boundary — public access ---

    #[test]
    fn validate_allows_public_access_from_any_module() {
        let result = validate_api_boundary("external::module::consumer", "HashService");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_allows_public_access_from_different_platform() {
        let result = validate_api_boundary("client::jvm_host", "BuildId");
        assert!(result.is_ok());
    }

    // --- validate_api_boundary — internal access ---

    #[test]
    fn validate_blocks_internal_access_from_wrong_platform() {
        let result = validate_api_boundary("client::jvm_host", "InternalCacheStore");
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.actual_visibility, VisibilityLevel::Internal);
    }

    #[test]
    fn validate_allows_internal_access_from_same_platform() {
        let result = validate_api_boundary("substrate::server::cache::store", "InternalCacheStore");
        assert!(result.is_ok());
    }

    // --- validate_api_boundary — private access ---

    #[test]
    fn validate_blocks_private_access_from_outside_module() {
        let result = validate_api_boundary(
            "substrate::server::other",
            "substrate::server::cache::internal::PrivateDetail",
        );
        // Unknown item treated as private, source doesn't match target prefix
        assert!(result.is_err());
    }

    #[test]
    fn validate_allows_private_access_from_same_module() {
        let result = validate_api_boundary(
            "substrate::server::cache::internal",
            "substrate::server::cache",
        );
        // source starts with target's prefix
        assert!(result.is_ok());
    }

    // --- PUBLIC_API completeness ---

    #[test]
    fn public_api_has_at_least_20_items() {
        assert!(
            PUBLIC_API.len() >= 20,
            "PUBLIC_API should have at least 20 items, has {}",
            PUBLIC_API.len()
        );
    }

    #[test]
    fn all_items_have_non_empty_descriptions() {
        for item in PUBLIC_API.iter() {
            assert!(
                !item.description.is_empty(),
                "API item '{}' has an empty description",
                item.name
            );
        }
    }

    #[test]
    fn all_items_have_valid_since_version() {
        for item in PUBLIC_API.iter() {
            assert!(
                !item.since_version.is_empty(),
                "API item '{}' has an empty since_version",
                item.name
            );
            // Basic semver-like check
            assert!(
                item.since_version.contains('.'),
                "API item '{}' has invalid since_version '{}'",
                item.name,
                item.since_version
            );
        }
    }

    // --- Macro expansion ---

    #[test]
    fn declare_internal_module_macro_expands() {
        declare_internal_module!("test::module", "Test module for macro expansion");
        assert_eq!(INTERNAL_MODULE_GUARD.module_path, "test::module");
        assert_eq!(
            INTERNAL_MODULE_GUARD.reason,
            "Test module for macro expansion"
        );
    }

    #[test]
    #[allow(non_upper_case_globals)]
    fn declare_public_api_macro_expands() {
        declare_public_api!(
            TestService,
            Service,
            Stable,
            "1.0.0",
            "Test service for macro expansion"
        );
        assert_eq!(TestService.name, "TestService");
        assert_eq!(TestService.kind, ApiKind::Service);
        assert_eq!(TestService.visibility, VisibilityLevel::Public);
        assert_eq!(TestService.stability, ApiStability::Stable);
        assert_eq!(TestService.since_version, "1.0.0");
        assert_eq!(TestService.description, "Test service for macro expansion");
    }

    // --- ApiViolation display ---

    #[test]
    fn api_violation_display() {
        let violation = ApiViolation {
            source_module: "client::foo".to_string(),
            target_item: "InternalCacheStore".to_string(),
            expected_visibility: VisibilityLevel::Public,
            actual_visibility: VisibilityLevel::Internal,
        };
        let msg = format!("{}", violation);
        assert!(msg.contains("client::foo"));
        assert!(msg.contains("InternalCacheStore"));
    }

    // --- Coverage: all services in PUBLIC_API ---

    #[test]
    fn all_expected_services_present() {
        let registry = ApiRegistry::new();
        let expected_services = [
            "HashService",
            "CacheService",
            "ExecService",
            "WorkService",
            "ExecutionPlanService",
            "ExecutionHistoryService",
            "BuildCacheOrchestrationService",
            "FileFingerprintService",
            "ValueSnapshotService",
            "TaskGraphService",
            "ConfigurationService",
            "ConfigurationCacheService",
            "BootstrapService",
            "ControlService",
            "BuildEventStreamService",
            "DagExecutorService",
        ];
        for name in expected_services {
            assert!(
                registry.get(name).is_some(),
                "Expected service '{}' not found in PUBLIC_API",
                name
            );
        }
    }

    #[test]
    fn all_expected_types_present() {
        let registry = ApiRegistry::new();
        let expected_types = [
            "CanonicalBuildPlan",
            "BuildPlanNode",
            "BuildPlanEdge",
            "BuildId",
            "SessionId",
            "TreeId",
            "ProjectPath",
        ];
        for name in expected_types {
            assert!(
                registry.get(name).is_some(),
                "Expected type '{}' not found in PUBLIC_API",
                name
            );
        }
    }

    #[test]
    fn protocol_version_constant_present() {
        let registry = ApiRegistry::new();
        let item = registry.get("PROTOCOL_VERSION");
        assert!(item.is_some());
        assert_eq!(item.unwrap().kind, ApiKind::Constant);
    }
}
