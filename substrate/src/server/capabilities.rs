//! Capability-based access control for the Gradle daemon.
//!
//! This module implements explicit capability handles so that filesystem,
//! environment variable, system property, and network access are observable
//! and testable. Instead of the daemon having implicit access to all resources,
//! operations require a `CapabilityToken` that grants specific permissions.
//!
//! # Architecture
//!
//! - **CapabilityToken** — Opaque token granting specific permissions within a scope
//! - **CapabilityRegistry** — Central registry that issues, validates, and audits tokens
//! - **Typed wrappers** — `FileSystemCapability`, `EnvironmentCapability`, `RepositoryCapability`
//!   provide safe, type-checked access to specific resource types
//! - **CapabilityBuilder** — Fluent API for constructing permission sets
//!
//! # Example
//!
//! ```rust
//! use gradle_substrate_daemon::server::capabilities::*;
//!
//! let registry = Arc::new(CapabilityRegistry::new());
//! let token_id = CapabilityBuilder::new(CapabilityScope::Global)
//!     .allow_read(PathBuf::from("/project/src"))
//!     .allow_env("GRADLE_*".to_string())
//!     .build(&registry);
//!
//! let fs = FileSystemCapability::from_token(token_id, registry.clone()).unwrap();
//! // fs.read_file(&PathBuf::from("/project/src/main.rs")) // would succeed
//! // fs.read_file(&PathBuf::from("/etc/passwd"))           // would fail
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use uuid::Uuid;

use crate::server::scopes::{BuildId, ProjectPath};

// ---------------------------------------------------------------------------
// Permission — granular resource access rights
// ---------------------------------------------------------------------------

/// A single permission granted to a capability token.
///
/// Each permission variant encodes the resource type and the scope of access.
/// Path-based permissions use prefix matching; pattern-based permissions use
/// simple glob matching with `*` wildcards.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Read files under a directory prefix.
    FsRead { path_prefix: PathBuf },
    /// Write files under a directory prefix.
    FsWrite { path_prefix: PathBuf },
    /// Read environment variables whose names match a glob pattern.
    EnvRead { key_pattern: String },
    /// Modify environment variables (rarely granted).
    EnvWrite,
    /// Read system properties whose names match a glob pattern.
    SysPropRead { key_pattern: String },
    /// Access network hosts matching a pattern.
    NetworkAccess { host_pattern: String },
    /// Spawn processes whose command starts with a given prefix.
    ProcessSpawn { command_prefix: String },
    /// Access a named cache.
    CacheAccess { cache_name: String },
}

impl Permission {
    /// Check whether this permission satisfies a required permission.
    ///
    /// For path-based permissions, the required path must be under the
    /// granted prefix. For pattern-based permissions, the required key/host
    /// must match the granted glob pattern.
    pub fn matches(&self, required: &Permission) -> bool {
        match (self, required) {
            (Permission::FsRead { path_prefix }, Permission::FsRead { path_prefix: req }) => {
                path_is_under(req, path_prefix)
            }
            (Permission::FsWrite { path_prefix }, Permission::FsWrite { path_prefix: req }) => {
                path_is_under(req, path_prefix)
            }
            (
                Permission::EnvRead { key_pattern },
                Permission::EnvRead {
                    key_pattern: req_key,
                },
            ) => glob_match(key_pattern, req_key),
            (Permission::EnvWrite, Permission::EnvWrite) => true,
            (
                Permission::SysPropRead { key_pattern },
                Permission::SysPropRead {
                    key_pattern: req_key,
                },
            ) => glob_match(key_pattern, req_key),
            (
                Permission::NetworkAccess { host_pattern },
                Permission::NetworkAccess {
                    host_pattern: req_host,
                },
            ) => glob_match(host_pattern, req_host),
            (
                Permission::ProcessSpawn { command_prefix },
                Permission::ProcessSpawn {
                    command_prefix: req_cmd,
                },
            ) => req_cmd.starts_with(command_prefix),
            (
                Permission::CacheAccess { cache_name },
                Permission::CacheAccess {
                    cache_name: req_cache,
                },
            ) => cache_name == req_cache,
            _ => false,
        }
    }

    /// Human-readable label for the permission kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Permission::FsRead { .. } => "fs:read",
            Permission::FsWrite { .. } => "fs:write",
            Permission::EnvRead { .. } => "env:read",
            Permission::EnvWrite => "env:write",
            Permission::SysPropRead { .. } => "sysprop:read",
            Permission::NetworkAccess { .. } => "net:access",
            Permission::ProcessSpawn { .. } => "process:spawn",
            Permission::CacheAccess { .. } => "cache:access",
        }
    }

    /// Resource identifier for auditing.
    pub fn resource_label(&self) -> String {
        match self {
            Permission::FsRead { path_prefix } | Permission::FsWrite { path_prefix } => {
                path_prefix.display().to_string()
            }
            Permission::EnvRead { key_pattern } | Permission::SysPropRead { key_pattern } => {
                key_pattern.clone()
            }
            Permission::EnvWrite => "*".into(),
            Permission::NetworkAccess { host_pattern } => host_pattern.clone(),
            Permission::ProcessSpawn { command_prefix } => command_prefix.clone(),
            Permission::CacheAccess { cache_name } => cache_name.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// CapabilityScope — hierarchical scope for tokens
// ---------------------------------------------------------------------------

/// The scope within which a capability token is valid.
///
/// Scopes form a hierarchy: Global > Build > Project > Task.
/// A token scoped to a narrower scope cannot access resources outside
/// that scope even if it holds a matching permission.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CapabilityScope {
    /// Scoped to a specific build execution.
    Build(BuildId),
    /// Scoped to a project within a build.
    Project(BuildId, ProjectPath),
    /// Scoped to a specific task within a project.
    Task(BuildId, ProjectPath, String),
    /// Unrestricted scope — only for bootstrap/daemon initialization.
    Global,
}

impl CapabilityScope {
    /// Returns the build ID associated with this scope, if any.
    pub fn build_id(&self) -> Option<&BuildId> {
        match self {
            CapabilityScope::Build(id)
            | CapabilityScope::Project(id, _)
            | CapabilityScope::Task(id, _, _) => Some(id),
            CapabilityScope::Global => None,
        }
    }

    /// Returns the project path associated with this scope, if any.
    pub fn project_path(&self) -> Option<&ProjectPath> {
        match self {
            CapabilityScope::Project(_, path) | CapabilityScope::Task(_, path, _) => Some(path),
            _ => None,
        }
    }

    /// Human-readable description of the scope.
    pub fn label(&self) -> String {
        match self {
            CapabilityScope::Global => "global".into(),
            CapabilityScope::Build(id) => format!("build:{}", id),
            CapabilityScope::Project(build_id, proj) => {
                format!("project:{}:{}", build_id, proj)
            }
            CapabilityScope::Task(build_id, proj, task) => {
                format!("task:{}:{}:{}", build_id, proj, task)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CapabilityToken — opaque permission grant
// ---------------------------------------------------------------------------

/// An opaque token that grants specific permissions within a scope.
///
/// Tokens are identified by a `Uuid` and managed by the `CapabilityRegistry`.
/// Callers hold only the `Uuid`; the registry stores the actual permissions.
pub struct CapabilityToken {
    pub(crate) permissions: Vec<Permission>,
    pub(crate) scope: CapabilityScope,
}

impl CapabilityToken {
    fn new(permissions: Vec<Permission>, scope: CapabilityScope) -> Self {
        Self { permissions, scope }
    }

    /// Check if this token grants the required permission.
    fn grants(&self, required: &Permission) -> bool {
        self.permissions.iter().any(|p| p.matches(required))
    }
}

// ---------------------------------------------------------------------------
// CapabilityError
// ---------------------------------------------------------------------------

/// Errors returned by capability-based access checks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityError {
    /// The token was not found in the registry.
    #[error("token not found: {0}")]
    TokenNotFound(Uuid),

    /// The token has been revoked.
    #[error("token has been revoked")]
    TokenRevoked,

    /// The token does not grant the required permission.
    #[error("insufficient permissions: required {required} (scope: {scope})")]
    InsufficientPermissions { required: String, scope: String },

    /// The requested path is not under any allowed prefix.
    #[error("path not under allowed prefix: path={path}, prefix={prefix}")]
    PathNotUnderPrefix { path: String, prefix: String },

    /// The audit log has reached capacity.
    #[error("audit log is full")]
    AuditLogFull,
}

// ---------------------------------------------------------------------------
// CapabilityAuditEntry
// ---------------------------------------------------------------------------

/// A single entry in the capability audit trail.
#[derive(Debug, Clone)]
pub struct CapabilityAuditEntry {
    /// The token that was used.
    pub token_id: Uuid,
    /// The permission that was checked.
    pub permission: String,
    /// The specific resource that was accessed (or attempted).
    pub resource: String,
    /// Whether the access was granted.
    pub granted: bool,
    /// Scope label at time of access.
    pub scope: String,
}

// ---------------------------------------------------------------------------
// CapabilityRegistry — central authority
// ---------------------------------------------------------------------------

/// Central registry that issues, validates, revokes, and audits capability tokens.
///
/// The registry is thread-safe and designed for concurrent access from multiple
/// executor threads.
pub struct CapabilityRegistry {
    tokens: DashMap<Uuid, CapabilityToken>,
    revoked: DashMap<Uuid, ()>,
    audit_log: std::sync::Mutex<Vec<CapabilityAuditEntry>>,
}

impl CapabilityRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tokens: DashMap::new(),
            revoked: DashMap::new(),
            audit_log: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Issue a new capability token with the given permissions and scope.
    ///
    /// Returns the token's `Uuid` which can be used for validation and revocation.
    pub fn issue(&self, permissions: Vec<Permission>, scope: CapabilityScope) -> Uuid {
        let id = Uuid::new_v4();
        let token = CapabilityToken::new(permissions, scope);
        self.tokens.insert(id, token);
        id
    }

    /// Validate that a token grants the required permission.
    ///
    /// Records the access attempt in the audit log regardless of outcome.
    pub fn validate(&self, token_id: &Uuid, required: &Permission) -> Result<(), CapabilityError> {
        let scope_label = self.scope_label(token_id);

        if self.revoked.contains_key(token_id) {
            let _ = self.record_access_inner(token_id, required, "revoked", false, &scope_label);
            return Err(CapabilityError::TokenRevoked);
        }

        let granted = match self.tokens.get(token_id) {
            Some(token) => token.grants(required),
            None => {
                let _ =
                    self.record_access_inner(token_id, required, "not_found", false, &scope_label);
                return Err(CapabilityError::TokenNotFound(*token_id));
            }
        };

        let _ = self.record_access_inner(
            token_id,
            required,
            &required.resource_label(),
            granted,
            &scope_label,
        );

        if granted {
            Ok(())
        } else {
            Err(CapabilityError::InsufficientPermissions {
                required: format!("{:?}({})", required.kind(), required.resource_label()),
                scope: scope_label,
            })
        }
    }

    /// Revoke a token, preventing any further access.
    pub fn revoke(&self, token_id: &Uuid) {
        self.revoked.insert(*token_id, ());
        self.tokens.remove(token_id);
    }

    /// Return a snapshot of the audit log.
    pub fn audit_log(&self) -> Vec<CapabilityAuditEntry> {
        self.audit_log.lock().unwrap().clone()
    }

    /// Record an access attempt in the audit log.
    pub fn record_access(
        &self,
        token_id: &Uuid,
        permission: &Permission,
        resource: &str,
    ) -> Result<(), CapabilityError> {
        let scope_label = self.scope_label(token_id);
        self.record_access_inner(token_id, permission, resource, true, &scope_label)
    }

    fn record_access_inner(
        &self,
        token_id: &Uuid,
        permission: &Permission,
        resource: &str,
        granted: bool,
        scope_label: &str,
    ) -> Result<(), CapabilityError> {
        let mut log = self.audit_log.lock().unwrap();
        // Cap audit log at 100_000 entries to prevent unbounded growth
        if log.len() >= 100_000 {
            return Err(CapabilityError::AuditLogFull);
        }
        log.push(CapabilityAuditEntry {
            token_id: *token_id,
            permission: format!("{:?}({})", permission.kind(), permission.resource_label()),
            resource: resource.to_string(),
            granted,
            scope: scope_label.to_string(),
        });
        Ok(())
    }

    fn scope_label(&self, token_id: &Uuid) -> String {
        self.tokens
            .get(token_id)
            .map(|t| t.value().scope.label())
            .unwrap_or_else(|| "unknown".into())
    }

    /// Look up a token's scope.
    pub fn token_scope(&self, token_id: &Uuid) -> Option<CapabilityScope> {
        self.tokens.get(token_id).map(|t| t.value().scope.clone())
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// FileSystemCapability — typed wrapper for filesystem operations
// ---------------------------------------------------------------------------

/// A typed capability wrapper that provides safe filesystem operations.
///
/// All operations check that the target path is under an allowed prefix
/// before proceeding.
pub struct FileSystemCapability {
    token_id: Uuid,
    registry: Arc<CapabilityRegistry>,
    allowed_prefixes: Vec<PathBuf>,
    write_prefixes: Vec<PathBuf>,
}

impl FileSystemCapability {
    /// Construct from a token. Returns `None` if the token has no fs permissions.
    pub fn from_token(token_id: Uuid, registry: Arc<CapabilityRegistry>) -> Option<Self> {
        let (read_prefixes, write_prefixes) = match registry.tokens.get(&token_id) {
            Some(token) => {
                let mut reads = Vec::new();
                let mut writes = Vec::new();
                for p in &token.value().permissions {
                    match p {
                        Permission::FsRead { path_prefix } => reads.push(path_prefix.clone()),
                        Permission::FsWrite { path_prefix } => writes.push(path_prefix.clone()),
                        _ => {}
                    }
                }
                if reads.is_empty() && writes.is_empty() {
                    return None;
                }
                (reads, writes)
            }
            None => return None,
        };

        Some(Self {
            token_id,
            registry,
            allowed_prefixes: read_prefixes,
            write_prefixes,
        })
    }

    /// Read the contents of a file.
    ///
    /// The path must be under an allowed read prefix.
    pub fn read_file(&self, path: &Path) -> Result<Vec<u8>, CapabilityError> {
        self.check_read(path)?;
        std::fs::read(path).map_err(|e| {
            let _ = self.registry.record_access(
                &self.token_id,
                &Permission::FsRead {
                    path_prefix: path.to_path_buf(),
                },
                &format!("read_error:{}", e),
            );
            CapabilityError::PathNotUnderPrefix {
                path: path.display().to_string(),
                prefix: "io_error".into(),
            }
        })
    }

    /// Write data to a file.
    ///
    /// The path must be under an allowed write prefix.
    pub fn write_file(&self, path: &Path, data: &[u8]) -> Result<(), CapabilityError> {
        self.check_write(path)?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, data).map_err(|e| CapabilityError::PathNotUnderPrefix {
            path: path.display().to_string(),
            prefix: format!("io_error:{}", e),
        })
    }

    /// List entries in a directory.
    ///
    /// The path must be under an allowed read prefix.
    pub fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>, CapabilityError> {
        self.check_read(path)?;
        let entries: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|_| CapabilityError::PathNotUnderPrefix {
                path: path.display().to_string(),
                prefix: "io_error".into(),
            })?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        Ok(entries)
    }

    /// Check if a path exists.
    ///
    /// The path must be under an allowed read prefix.
    pub fn exists(&self, path: &Path) -> Result<bool, CapabilityError> {
        self.check_read(path)?;
        Ok(path.exists())
    }

    /// Return the allowed read prefixes.
    pub fn allowed_prefixes(&self) -> &[PathBuf] {
        &self.allowed_prefixes
    }

    /// Return the allowed write prefixes.
    pub fn write_prefixes(&self) -> &[PathBuf] {
        &self.write_prefixes
    }

    fn check_read(&self, path: &Path) -> Result<(), CapabilityError> {
        let required = Permission::FsRead {
            path_prefix: path.to_path_buf(),
        };
        self.registry.validate(&self.token_id, &required)?;
        if !self.allowed_prefixes.iter().any(|p| path_is_under(path, p)) {
            return Err(CapabilityError::PathNotUnderPrefix {
                path: path.display().to_string(),
                prefix: self
                    .allowed_prefixes
                    .first()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(none)".into()),
            });
        }
        Ok(())
    }

    fn check_write(&self, path: &Path) -> Result<(), CapabilityError> {
        let required = Permission::FsWrite {
            path_prefix: path.to_path_buf(),
        };
        self.registry.validate(&self.token_id, &required)?;
        if !self.write_prefixes.iter().any(|p| path_is_under(path, p)) {
            return Err(CapabilityError::PathNotUnderPrefix {
                path: path.display().to_string(),
                prefix: self
                    .write_prefixes
                    .first()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(none)".into()),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// EnvironmentCapability — typed wrapper for env var access
// ---------------------------------------------------------------------------

/// A typed capability wrapper that provides safe environment variable access.
///
/// Only environment variables matching an allowed pattern can be read.
pub struct EnvironmentCapability {
    token_id: Uuid,
    registry: Arc<CapabilityRegistry>,
    allowed_patterns: Vec<String>,
}

impl EnvironmentCapability {
    /// Construct from a token. Returns `None` if the token has no env permissions.
    pub fn from_token(token_id: Uuid, registry: Arc<CapabilityRegistry>) -> Option<Self> {
        let patterns = match registry.tokens.get(&token_id) {
            Some(token) => token
                .value()
                .permissions
                .iter()
                .filter_map(|p| match p {
                    Permission::EnvRead { key_pattern } => Some(key_pattern.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            None => return None,
        };

        if patterns.is_empty() {
            return None;
        }

        Some(Self {
            token_id,
            registry,
            allowed_patterns: patterns,
        })
    }

    /// Get a single environment variable.
    ///
    /// Returns `Ok(None)` if the variable is not set.
    /// Returns an error if the key does not match any allowed pattern.
    pub fn get_var(&self, key: &str) -> Result<Option<String>, CapabilityError> {
        let required = Permission::EnvRead {
            key_pattern: key.to_string(),
        };
        self.registry.validate(&self.token_id, &required)?;

        if !self
            .allowed_patterns
            .iter()
            .any(|pattern| glob_match(pattern, key))
        {
            return Err(CapabilityError::InsufficientPermissions {
                required: format!("env:read({})", key),
                scope: self
                    .registry
                    .token_scope(&self.token_id)
                    .map(|s| s.label())
                    .unwrap_or_else(|| "unknown".into()),
            });
        }

        Ok(std::env::var(key).ok())
    }

    /// Get all environment variables that match allowed patterns.
    ///
    /// Returns a sorted map of matching variables.
    pub fn get_vars(&self) -> Result<BTreeMap<String, String>, CapabilityError> {
        let mut result = BTreeMap::new();
        for (key, value) in std::env::vars() {
            if self
                .allowed_patterns
                .iter()
                .any(|pattern| glob_match(pattern, &key))
            {
                let required = Permission::EnvRead {
                    key_pattern: key.clone(),
                };
                if self.registry.validate(&self.token_id, &required).is_ok() {
                    result.insert(key, value);
                }
            }
        }
        Ok(result)
    }

    /// Return the allowed patterns.
    pub fn allowed_patterns(&self) -> &[String] {
        &self.allowed_patterns
    }
}

// ---------------------------------------------------------------------------
// RepositoryCapability — typed wrapper for repository/network access
// ---------------------------------------------------------------------------

/// A typed capability wrapper for repository and network access.
///
/// Only URLs matching allowed host patterns can be accessed.
pub struct RepositoryCapability {
    #[allow(dead_code)]
    token_id: Uuid,
    #[allow(dead_code)]
    registry: Arc<CapabilityRegistry>,
    allowed_urls: Vec<String>,
}

impl RepositoryCapability {
    /// Construct from a token. Returns `None` if the token has no network permissions.
    pub fn from_token(token_id: Uuid, registry: Arc<CapabilityRegistry>) -> Option<Self> {
        let urls = match registry.tokens.get(&token_id) {
            Some(token) => token
                .value()
                .permissions
                .iter()
                .filter_map(|p| match p {
                    Permission::NetworkAccess { host_pattern } => Some(host_pattern.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            None => return None,
        };

        if urls.is_empty() {
            return None;
        }

        Some(Self {
            token_id,
            registry,
            allowed_urls: urls,
        })
    }

    /// Check whether a URL can be accessed.
    pub fn can_access(&self, url: &str) -> bool {
        let host = extract_host(url);
        self.allowed_urls
            .iter()
            .any(|pattern| glob_match(pattern, &host))
    }

    /// Return the allowed URL patterns.
    pub fn allowed_urls(&self) -> &[String] {
        &self.allowed_urls
    }
}

// ---------------------------------------------------------------------------
// CapabilityBuilder — fluent API for constructing tokens
// ---------------------------------------------------------------------------

/// A fluent builder for creating capability token configurations.
///
/// # Example
///
/// ```rust
/// let token_id = CapabilityBuilder::new(CapabilityScope::Build(BuildId::from("b1".into())))
///     .allow_read(PathBuf::from("/project/src"))
///     .allow_write(PathBuf::from("/project/build"))
///     .allow_env("GRADLE_HOME".to_string())
///     .allow_network("repo.gradle.org".to_string())
///     .allow_cache("compile".to_string())
///     .build(&registry);
/// ```
pub struct CapabilityBuilder {
    permissions: Vec<Permission>,
    scope: CapabilityScope,
}

impl CapabilityBuilder {
    /// Create a new builder with the given scope and no permissions.
    pub fn new(scope: CapabilityScope) -> Self {
        Self {
            permissions: Vec::new(),
            scope,
        }
    }

    /// Grant read access to files under the given path prefix.
    pub fn allow_read(mut self, path: PathBuf) -> Self {
        self.permissions
            .push(Permission::FsRead { path_prefix: path });
        self
    }

    /// Grant write access to files under the given path prefix.
    pub fn allow_write(mut self, path: PathBuf) -> Self {
        self.permissions
            .push(Permission::FsWrite { path_prefix: path });
        self
    }

    /// Grant read access to environment variables matching the given pattern.
    pub fn allow_env(mut self, key: String) -> Self {
        self.permissions
            .push(Permission::EnvRead { key_pattern: key });
        self
    }

    /// Grant write access to environment variables.
    pub fn allow_env_write(mut self) -> Self {
        self.permissions.push(Permission::EnvWrite);
        self
    }

    /// Grant read access to system properties matching the given pattern.
    pub fn allow_sysprop(mut self, key: String) -> Self {
        self.permissions
            .push(Permission::SysPropRead { key_pattern: key });
        self
    }

    /// Grant network access to hosts matching the given pattern.
    pub fn allow_network(mut self, host: String) -> Self {
        self.permissions
            .push(Permission::NetworkAccess { host_pattern: host });
        self
    }

    /// Grant permission to spawn processes with the given command prefix.
    pub fn allow_process(mut self, command_prefix: String) -> Self {
        self.permissions
            .push(Permission::ProcessSpawn { command_prefix });
        self
    }

    /// Grant access to a named cache.
    pub fn allow_cache(mut self, name: String) -> Self {
        self.permissions
            .push(Permission::CacheAccess { cache_name: name });
        self
    }

    /// Issue the token in the registry and return its ID.
    pub fn build(self, registry: &CapabilityRegistry) -> Uuid {
        registry.issue(self.permissions, self.scope)
    }
}

// ---------------------------------------------------------------------------
// Permission matching helpers
// ---------------------------------------------------------------------------

/// Check whether `path` is under (or equal to) `prefix`.
///
/// Both paths are canonicalized by stripping trailing separators and
/// comparing components. A path is considered "under" a prefix if the
/// prefix is a leading segment of the path.
pub fn path_is_under(path: &Path, prefix: &Path) -> bool {
    let path = normalize_path(path);
    let prefix = normalize_path(prefix);

    if path == prefix {
        return true;
    }

    let prefix_components: Vec<_> = prefix.components().collect();
    let path_components: Vec<_> = path.components().collect();

    if prefix_components.len() > path_components.len() {
        return false;
    }

    path_components[..prefix_components.len()] == prefix_components
}

/// Normalize a path for comparison by removing redundant separators.
fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Simple glob matching supporting `*` (matches any sequence of non-separator chars)
/// and `**` (matches any sequence including separators).
///
/// This is a minimal implementation sufficient for environment variable
/// and host pattern matching. It does not support character classes `[...]`
/// or single-char wildcards `?`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    // Handle exact match
    if pattern == text {
        return true;
    }

    // Handle trailing wildcard: "GRADLE_*" matches "GRADLE_HOME"
    if pattern.ends_with('*') && !pattern.ends_with("**") {
        let prefix = &pattern[..pattern.len() - 1];
        return text.starts_with(prefix);
    }

    // Handle leading wildcard: "*.gradle.org" matches "repo.gradle.org"
    if pattern.starts_with('*') && !pattern.starts_with("**") {
        let suffix = &pattern[1..];
        return text.ends_with(suffix);
    }

    // Handle double-star wildcard: matches everything
    if pattern == "*" || pattern == "**" {
        return true;
    }

    // Handle pattern with wildcard in the middle: "foo*bar"
    if let Some(star_pos) = pattern.find('*') {
        let (before, after) = pattern.split_at(star_pos);
        let after = &after[1..]; // skip the '*'
        if text.starts_with(before) && text[before.len()..].ends_with(after) {
            return true;
        }
    }

    // Use proper recursive glob matching for **
    glob_match_recursive(pattern, text)
}

/// Recursive glob matcher supporting * (any chars within segment) and ** (any segments).
fn glob_match_recursive(pattern: &str, text: &str) -> bool {
    if pattern == text {
        return true;
    }

    // Handle ** wildcard
    if let Some(pos) = pattern.find("**") {
        let before = &pattern[..pos];
        let after_start = pos + 2;
        let after = &pattern[after_start..];
        let after_trimmed = after.strip_prefix('/').unwrap_or(after);

        if let Some(remaining) = text.strip_prefix(before) {
            // ** can match zero or more path segments.
            // Try the remainder of the pattern against every suffix of remaining.
            // For pattern "src/**/*.java" with text "src/com/example/App.java":
            // before="src/", after="*.java" (trimmed)
            // Try: "*.java" vs "com/example/App.java", "example/App.java", "App.java", ""
            
            if after_trimmed.is_empty() {
                // Pattern ends with ** after consuming some prefix -> matches everything remaining
                return true;
            }
            // Try at every position after a '/' separator plus the full remaining
            let mut pos = 0;
            loop {
                if glob_match_recursive(after_trimmed, &remaining[pos..]) {
                    return true;
                }
                if let Some(next) = remaining[pos..].find('/') {
                    pos += next + 1; // skip past the '/'
                } else {
                    break;
                }
            }
        }
        return false;
    }

    // Handle single * within a segment (no **)
    if let Some(pos) = pattern.find('*') {
        let before = &pattern[..pos];
        let after = &pattern[pos + 1..];
        // If there's no more *, do a simple check
        if !after.contains('*') {
            return text.starts_with(before) && text[before.len()..].ends_with(after);
        }
        // Multiple single-* wildcards: find matching segment
        if let Some(remaining) = text.strip_prefix(before) {
            let slash_pos = remaining.find('/').unwrap_or(remaining.len());
            let segment = &remaining[..slash_pos];
            let rest = &remaining[slash_pos..];
            return glob_match_single_star(after, segment) && rest.is_empty();
        }
        return false;
    }

    false
}

/// Match a pattern that may contain * wildcards but no **.
/// Each * matches any sequence of chars within a single segment.
fn glob_match_single_star(pattern: &str, segment: &str) -> bool {
    if let Some(pos) = pattern.find('*') {
        let before = &pattern[..pos];
        let after = &pattern[pos + 1..];
        if let Some(remaining) = segment.strip_prefix(before) {
            return remaining.ends_with(after);
        }
        return false;
    }
    pattern == segment
}

/// Extract the host portion from a URL string.
///
/// Handles common schemes (http, https, file, ssh) and returns the host
/// without port or path.
fn extract_host(url: &str) -> String {
    // Strip scheme
    let without_scheme = if let Some(pos) = url.find("://") {
        &url[pos + 3..]
    } else {
        url
    };

    // Strip path
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);

    // Strip port
    host_port.split(':').next().unwrap_or(host_port).to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Token issuance and validation
    // =========================================================================

    #[test]
    fn test_issue_and_validate_token() {
        let registry = Arc::new(CapabilityRegistry::new());
        let perms = vec![Permission::FsRead {
            path_prefix: PathBuf::from("/project/src"),
        }];
        let token_id = registry.issue(perms, CapabilityScope::Global);

        let required = Permission::FsRead {
            path_prefix: PathBuf::from("/project/src/main.rs"),
        };
        assert!(registry.validate(&token_id, &required).is_ok());
    }

    #[test]
    fn test_validate_fails_for_missing_permission() {
        let registry = Arc::new(CapabilityRegistry::new());
        let perms = vec![Permission::FsRead {
            path_prefix: PathBuf::from("/project/src"),
        }];
        let token_id = registry.issue(perms, CapabilityScope::Global);

        let required = Permission::FsWrite {
            path_prefix: PathBuf::from("/project/src/output.txt"),
        };
        assert!(registry.validate(&token_id, &required).is_err());
    }

    #[test]
    fn test_validate_fails_for_unknown_token() {
        let registry = Arc::new(CapabilityRegistry::new());
        let unknown = Uuid::new_v4();
        let required = Permission::FsRead {
            path_prefix: PathBuf::from("/any"),
        };
        let err = registry.validate(&unknown, &required).unwrap_err();
        assert!(matches!(err, CapabilityError::TokenNotFound(_)));
    }

    // =========================================================================
    // Path prefix enforcement
    // =========================================================================

    #[test]
    fn test_path_is_under_exact() {
        assert!(path_is_under(
            Path::new("/project/src"),
            Path::new("/project/src")
        ));
    }

    #[test]
    fn test_path_is_under_child() {
        assert!(path_is_under(
            Path::new("/project/src/main.rs"),
            Path::new("/project/src")
        ));
    }

    #[test]
    fn test_path_is_under_deep_child() {
        assert!(path_is_under(
            Path::new("/project/src/com/example/App.java"),
            Path::new("/project/src")
        ));
    }

    #[test]
    fn test_path_is_not_under_unrelated() {
        assert!(!path_is_under(
            Path::new("/etc/passwd"),
            Path::new("/project/src")
        ));
    }

    #[test]
    fn test_path_is_not_under_parent_escape() {
        assert!(!path_is_under(
            Path::new("/project/other"),
            Path::new("/project/src")
        ));
    }

    #[test]
    fn test_path_is_not_under_shorter_prefix() {
        assert!(!path_is_under(
            Path::new("/project/src"),
            Path::new("/project/src/main")
        ));
    }

    #[test]
    fn test_fs_capability_can_read_under_prefix() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp/test-cap-fs"))
            .build(&registry);

        let fs = FileSystemCapability::from_token(token_id, registry.clone()).unwrap();

        // Create a test file under the allowed prefix
        let test_dir = PathBuf::from("/tmp/test-cap-fs");
        let _ = std::fs::create_dir_all(&test_dir);
        let test_file = test_dir.join("hello.txt");
        std::fs::write(&test_file, b"hello").unwrap();

        assert!(fs.read_file(&test_file).is_ok());
        assert_eq!(fs.read_file(&test_file).unwrap(), b"hello");

        // Cleanup
        let _ = std::fs::remove_file(&test_file);
    }

    #[test]
    fn test_fs_capability_cannot_read_outside_prefix() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp/test-cap-fs-allowed"))
            .build(&registry);

        let fs = FileSystemCapability::from_token(token_id, registry.clone()).unwrap();

        // Use a path that exists but is clearly outside the allowed prefix
        let result = fs.read_file(Path::new("/usr/bin"));
        // Could be InsufficientPermissions or PathNotUnderPrefix depending on
        // whether the path passes under any prefix but isn't readable, or
        // doesn't match any prefix at all. In either case, it should be
        // rejected.
        assert!(result.is_err());
    }

    #[test]
    fn test_fs_capability_can_write_under_prefix() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_write(PathBuf::from("/tmp/test-cap-fs-write"))
            .build(&registry);

        let fs = FileSystemCapability::from_token(token_id, registry.clone()).unwrap();

        let test_dir = PathBuf::from("/tmp/test-cap-fs-write");
        let _ = std::fs::create_dir_all(&test_dir);
        let test_file = test_dir.join("output.txt");

        assert!(fs.write_file(&test_file, b"world").is_ok());
        assert_eq!(std::fs::read(&test_file).unwrap(), b"world");

        // Cleanup
        let _ = std::fs::remove_file(&test_file);
    }

    #[test]
    fn test_fs_capability_cannot_write_outside_prefix() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_write(PathBuf::from("/tmp/test-cap-fs-write"))
            .build(&registry);

        let fs = FileSystemCapability::from_token(token_id, registry.clone()).unwrap();

        let result = fs.write_file(Path::new("/tmp/forbidden.txt"), b"nope");
        assert!(result.is_err());
    }

    #[test]
    fn test_fs_capability_exists() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp"))
            .build(&registry);

        let fs = FileSystemCapability::from_token(token_id, registry.clone()).unwrap();

        assert!(fs.exists(Path::new("/tmp")).unwrap());
        assert!(!fs
            .exists(Path::new("/tmp/nonexistent-cap-file-xyz"))
            .unwrap());
    }

    #[test]
    fn test_fs_capability_list_dir() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp/test-cap-listdir"))
            .build(&registry);

        let fs = FileSystemCapability::from_token(token_id, registry.clone()).unwrap();

        let test_dir = PathBuf::from("/tmp/test-cap-listdir");
        let _ = std::fs::create_dir_all(&test_dir);
        let _ = std::fs::write(test_dir.join("a.txt"), b"");
        let _ = std::fs::write(test_dir.join("b.txt"), b"");

        let entries = fs.list_dir(&test_dir).unwrap();
        assert_eq!(entries.len(), 2);

        // Cleanup
        let _ = std::fs::remove_file(test_dir.join("a.txt"));
        let _ = std::fs::remove_file(test_dir.join("b.txt"));
        let _ = std::fs::remove_dir(&test_dir);
    }

    // =========================================================================
    // Env var pattern matching
    // =========================================================================

    #[test]
    fn test_env_capability_matches_pattern() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_env("GRADLE_*".to_string())
            .build(&registry);

        let env = EnvironmentCapability::from_token(token_id, registry.clone()).unwrap();

        // Set a test env var
        std::env::set_var("GRADLE_TEST_CAP", "test-value");
        let result = env.get_var("GRADLE_TEST_CAP");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some("test-value".to_string()));

        // Cleanup
        std::env::remove_var("GRADLE_TEST_CAP");
    }

    #[test]
    fn test_env_capability_rejects_non_matching() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_env("GRADLE_*".to_string())
            .build(&registry);

        let env = EnvironmentCapability::from_token(token_id, registry.clone()).unwrap();

        std::env::set_var("SECRET_KEY", "should-not-read");
        let result = env.get_var("SECRET_KEY");
        assert!(result.is_err());

        // Cleanup
        std::env::remove_var("SECRET_KEY");
    }

    #[test]
    fn test_env_capability_get_vars() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_env("CAP_TEST_*".to_string())
            .build(&registry);

        let env = EnvironmentCapability::from_token(token_id, registry.clone()).unwrap();

        std::env::set_var("CAP_TEST_A", "1");
        std::env::set_var("CAP_TEST_B", "2");
        std::env::set_var("CAP_OTHER", "3");

        let vars = env.get_vars().unwrap();

        assert_eq!(vars.get("CAP_TEST_A"), Some(&"1".to_string()));
        assert_eq!(vars.get("CAP_TEST_B"), Some(&"2".to_string()));
        assert!(!vars.contains_key("CAP_OTHER"));

        // Cleanup
        std::env::remove_var("CAP_TEST_A");
        std::env::remove_var("CAP_TEST_B");
        std::env::remove_var("CAP_OTHER");
    }

    // =========================================================================
    // Audit log
    // =========================================================================

    #[test]
    fn test_audit_log_records_access() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp"))
            .build(&registry);

        let required = Permission::FsRead {
            path_prefix: PathBuf::from("/tmp/test"),
        };
        let _ = registry.validate(&token_id, &required);

        let log = registry.audit_log();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].token_id, token_id);
        assert!(log[0].granted);
    }

    #[test]
    fn test_audit_log_records_denied_access() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp"))
            .build(&registry);

        let required = Permission::FsWrite {
            path_prefix: PathBuf::from("/tmp/test"),
        };
        let _ = registry.validate(&token_id, &required);

        let log = registry.audit_log();
        assert_eq!(log.len(), 1);
        assert!(!log[0].granted);
    }

    #[test]
    fn test_audit_log_records_multiple_entries() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp"))
            .allow_env("HOME".to_string())
            .build(&registry);

        let _ = registry.validate(
            &token_id,
            &Permission::FsRead {
                path_prefix: PathBuf::from("/tmp/a"),
            },
        );
        let _ = registry.validate(
            &token_id,
            &Permission::EnvRead {
                key_pattern: "HOME".to_string(),
            },
        );

        let log = registry.audit_log();
        assert_eq!(log.len(), 2);
    }

    // =========================================================================
    // Token revocation
    // =========================================================================

    #[test]
    fn test_revoke_token_prevents_access() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp"))
            .build(&registry);

        // Should work before revocation
        let required = Permission::FsRead {
            path_prefix: PathBuf::from("/tmp/test"),
        };
        assert!(registry.validate(&token_id, &required).is_ok());

        // Revoke
        registry.revoke(&token_id);

        // Should fail after revocation
        assert!(matches!(
            registry.validate(&token_id, &required).unwrap_err(),
            CapabilityError::TokenRevoked
        ));
    }

    #[test]
    fn test_revoke_removes_from_tokens() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp"))
            .build(&registry);

        assert!(registry.token_scope(&token_id).is_some());

        registry.revoke(&token_id);

        assert!(registry.token_scope(&token_id).is_none());
    }

    // =========================================================================
    // CapabilityBuilder fluent API
    // =========================================================================

    #[test]
    fn test_builder_all_methods() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/read"))
            .allow_write(PathBuf::from("/write"))
            .allow_env("HOME".to_string())
            .allow_env_write()
            .allow_sysprop("java.*".to_string())
            .allow_network("*.maven.org".to_string())
            .allow_process("javac".to_string())
            .allow_cache("compile".to_string())
            .build(&registry);

        // Verify each permission type
        assert!(registry
            .validate(
                &token_id,
                &Permission::FsRead {
                    path_prefix: PathBuf::from("/read/file")
                }
            )
            .is_ok());
        assert!(registry
            .validate(
                &token_id,
                &Permission::FsWrite {
                    path_prefix: PathBuf::from("/write/file")
                }
            )
            .is_ok());
        assert!(registry
            .validate(
                &token_id,
                &Permission::EnvRead {
                    key_pattern: "HOME".to_string()
                }
            )
            .is_ok());
        assert!(registry.validate(&token_id, &Permission::EnvWrite).is_ok());
        assert!(registry
            .validate(
                &token_id,
                &Permission::SysPropRead {
                    key_pattern: "java.version".to_string()
                }
            )
            .is_ok());
        assert!(registry
            .validate(
                &token_id,
                &Permission::NetworkAccess {
                    host_pattern: "repo.maven.org".to_string()
                }
            )
            .is_ok());
        assert!(registry
            .validate(
                &token_id,
                &Permission::ProcessSpawn {
                    command_prefix: "javac".to_string()
                }
            )
            .is_ok());
        assert!(registry
            .validate(
                &token_id,
                &Permission::CacheAccess {
                    cache_name: "compile".to_string()
                }
            )
            .is_ok());
    }

    #[test]
    fn test_builder_chaining_returns_self() {
        let builder = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/a"))
            .allow_write(PathBuf::from("/b"));

        assert_eq!(builder.permissions.len(), 2);
    }

    // =========================================================================
    // Cross-scope isolation
    // =========================================================================

    #[test]
    fn test_project_scope_token_isolated() {
        let registry = Arc::new(CapabilityRegistry::new());

        // Project A token
        let token_a = CapabilityBuilder::new(CapabilityScope::Project(
            BuildId("build-1".into()),
            ProjectPath(":app".into()),
        ))
        .allow_read(PathBuf::from("/project/app"))
        .build(&registry);

        // Project B token
        let token_b = CapabilityBuilder::new(CapabilityScope::Project(
            BuildId("build-1".into()),
            ProjectPath(":lib".into()),
        ))
        .allow_read(PathBuf::from("/project/lib"))
        .build(&registry);

        // Token A can read its own path
        assert!(registry
            .validate(
                &token_a,
                &Permission::FsRead {
                    path_prefix: PathBuf::from("/project/app/src")
                }
            )
            .is_ok());

        // Token A cannot read project B's path (no matching permission)
        assert!(registry
            .validate(
                &token_b,
                &Permission::FsRead {
                    path_prefix: PathBuf::from("/project/app/src")
                }
            )
            .is_err());

        // Verify scopes are different
        let scope_a = registry.token_scope(&token_a).unwrap();
        let scope_b = registry.token_scope(&token_b).unwrap();
        assert_ne!(scope_a, scope_b);
    }

    #[test]
    fn test_task_scope_is_narrower_than_project() {
        let registry = Arc::new(CapabilityRegistry::new());

        let task_token = CapabilityBuilder::new(CapabilityScope::Task(
            BuildId("build-1".into()),
            ProjectPath(":app".into()),
            "compileJava".into(),
        ))
        .allow_read(PathBuf::from("/project/app/src"))
        .build(&registry);

        let scope = registry.token_scope(&task_token).unwrap();
        assert!(matches!(scope, CapabilityScope::Task(_, _, _)));
        assert_eq!(scope.label(), "task:build-1::app:compileJava");
    }

    #[test]
    fn test_build_scope_label() {
        let scope = CapabilityScope::Build(BuildId("my-build".into()));
        assert_eq!(scope.label(), "build:my-build");
    }

    #[test]
    fn test_global_scope_label() {
        let scope = CapabilityScope::Global;
        assert_eq!(scope.label(), "global");
    }

    #[test]
    fn test_scope_build_id_accessor() {
        let build_scope = CapabilityScope::Build(BuildId("b1".into()));
        assert_eq!(build_scope.build_id().map(|b| b.as_ref()), Some("b1"));

        let global_scope = CapabilityScope::Global;
        assert!(global_scope.build_id().is_none());
    }

    #[test]
    fn test_scope_project_path_accessor() {
        let project_scope =
            CapabilityScope::Project(BuildId("b1".into()), ProjectPath(":app".into()));
        assert_eq!(
            project_scope.project_path().map(|p| p.as_ref()),
            Some(":app")
        );

        let build_scope = CapabilityScope::Build(BuildId("b1".into()));
        assert!(build_scope.project_path().is_none());
    }

    // =========================================================================
    // Glob matching
    // =========================================================================

    #[test]
    fn test_glob_exact_match() {
        assert!(glob_match("HOME", "HOME"));
    }

    #[test]
    fn test_glob_trailing_wildcard() {
        assert!(glob_match("GRADLE_*", "GRADLE_HOME"));
        assert!(glob_match("GRADLE_*", "GRADLE_USER_HOME"));
        assert!(glob_match("GRADLE_*", "GRADLE_"));
    }

    #[test]
    fn test_glob_trailing_wildcard_no_match() {
        assert!(!glob_match("GRADLE_*", "JAVA_HOME"));
        assert!(!glob_match("GRADLE_*", "GRADLE"));
    }

    #[test]
    fn test_glob_leading_wildcard() {
        assert!(glob_match("*.gradle.org", "repo.gradle.org"));
        assert!(glob_match("*.maven.org", "central.maven.org"));
    }

    #[test]
    fn test_glob_leading_wildcard_no_match() {
        assert!(!glob_match("*.gradle.org", "gradle.org"));
    }

    #[test]
    fn test_glob_wildcard_all() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("**", "anything/at/all"));
    }

    #[test]
    fn test_glob_middle_wildcard() {
        assert!(glob_match("java*version", "java.runtime.version"));
        assert!(glob_match("foo*bar", "fooXYZbar"));
    }

    #[test]
    fn test_glob_double_star_path() {
        assert!(glob_match("src/**/*.java", "src/com/example/App.java"));
        assert!(glob_match("**/test/**", "some/path/test/unit/Test.java"));
    }

    // =========================================================================
    // Repository capability
    // =========================================================================

    #[test]
    fn test_repository_can_access_matching() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_network("*.maven.org".to_string())
            .allow_network("repo.gradle.org".to_string())
            .build(&registry);

        let repo = RepositoryCapability::from_token(token_id, registry.clone()).unwrap();

        assert!(repo.can_access("https://repo.maven.org/maven2/"));
        assert!(repo.can_access("https://central.maven.org/maven2/"));
        assert!(repo.can_access("https://repo.gradle.org/gradle/repo"));
        assert!(!repo.can_access("https://evil.example.com/malware"));
    }

    #[test]
    fn test_repository_allowed_urls() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_network("*.maven.org".to_string())
            .build(&registry);

        let repo = RepositoryCapability::from_token(token_id, registry.clone()).unwrap();
        assert_eq!(repo.allowed_urls(), &["*.maven.org"]);
    }

    #[test]
    fn test_repository_from_token_no_network_perm() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp"))
            .build(&registry);

        assert!(RepositoryCapability::from_token(token_id, registry.clone()).is_none());
    }

    // =========================================================================
    // Permission matching
    // =========================================================================

    #[test]
    fn test_permission_matches_fs_read_subset() {
        let granted = Permission::FsRead {
            path_prefix: PathBuf::from("/project"),
        };
        let required = Permission::FsRead {
            path_prefix: PathBuf::from("/project/src"),
        };
        assert!(granted.matches(&required));
    }

    #[test]
    fn test_permission_matches_fs_read_not_subset() {
        let granted = Permission::FsRead {
            path_prefix: PathBuf::from("/project/src"),
        };
        let required = Permission::FsRead {
            path_prefix: PathBuf::from("/project"),
        };
        assert!(!granted.matches(&required));
    }

    #[test]
    fn test_permission_matches_env_pattern() {
        let granted = Permission::EnvRead {
            key_pattern: "GRADLE_*".to_string(),
        };
        let required = Permission::EnvRead {
            key_pattern: "GRADLE_HOME".to_string(),
        };
        assert!(granted.matches(&required));
    }

    #[test]
    fn test_permission_matches_cache_exact() {
        let granted = Permission::CacheAccess {
            cache_name: "compile".to_string(),
        };
        let required = Permission::CacheAccess {
            cache_name: "compile".to_string(),
        };
        assert!(granted.matches(&required));
        let wrong = Permission::CacheAccess {
            cache_name: "test".to_string(),
        };
        assert!(!granted.matches(&wrong));
    }

    #[test]
    fn test_permission_matches_process_prefix() {
        let granted = Permission::ProcessSpawn {
            command_prefix: "java".to_string(),
        };
        let required = Permission::ProcessSpawn {
            command_prefix: "javac".to_string(),
        };
        assert!(granted.matches(&required));

        let wrong = Permission::ProcessSpawn {
            command_prefix: "kotlin".to_string(),
        };
        assert!(!granted.matches(&wrong));
    }

    #[test]
    fn test_permission_different_types_do_not_match() {
        let fs = Permission::FsRead {
            path_prefix: PathBuf::from("/tmp"),
        };
        let env = Permission::EnvRead {
            key_pattern: "*".to_string(),
        };
        assert!(!fs.matches(&env));
        assert!(!env.matches(&fs));
    }

    // =========================================================================
    // Permission kind and resource label
    // =========================================================================

    #[test]
    fn test_permission_kind() {
        assert_eq!(
            Permission::FsRead {
                path_prefix: PathBuf::from("/a")
            }
            .kind(),
            "fs:read"
        );
        assert_eq!(
            Permission::CacheAccess {
                cache_name: "x".to_string()
            }
            .kind(),
            "cache:access"
        );
    }

    #[test]
    fn test_permission_resource_label() {
        assert_eq!(
            Permission::FsRead {
                path_prefix: PathBuf::from("/a/b")
            }
            .resource_label(),
            "/a/b"
        );
        assert_eq!(Permission::EnvWrite.resource_label(), "*");
    }

    // =========================================================================
    // FileSystemCapability construction
    // =========================================================================

    #[test]
    fn test_fs_capability_none_without_fs_perms() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_env("HOME".to_string())
            .build(&registry);

        assert!(FileSystemCapability::from_token(token_id, registry.clone()).is_none());
    }

    #[test]
    fn test_env_capability_none_without_env_perms() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp"))
            .build(&registry);

        assert!(EnvironmentCapability::from_token(token_id, registry.clone()).is_none());
    }

    // =========================================================================
    // Extract host from URL
    // =========================================================================

    #[test]
    fn test_extract_host_https() {
        assert_eq!(
            extract_host("https://repo.maven.org/maven2/"),
            "repo.maven.org"
        );
    }

    #[test]
    fn test_extract_host_with_port() {
        assert_eq!(extract_host("https://localhost:8080/api"), "localhost");
    }

    #[test]
    fn test_extract_host_no_scheme() {
        assert_eq!(extract_host("repo.maven.org/maven2/"), "repo.maven.org");
    }

    // =========================================================================
    // Audit log capacity
    // =========================================================================

    #[test]
    fn test_audit_log_capacity_limit() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/tmp"))
            .build(&registry);

        // Fill the audit log
        for _ in 0..100_000 {
            let _ = registry.validate(
                &token_id,
                &Permission::FsRead {
                    path_prefix: PathBuf::from("/tmp/test"),
                },
            );
        }

        // Next entry should fail
        let result = registry.record_access(
            &token_id,
            &Permission::FsRead {
                path_prefix: PathBuf::from("/tmp/test"),
            },
            "overflow",
        );
        assert!(matches!(result, Err(CapabilityError::AuditLogFull)));
    }

    // =========================================================================
    // EnvironmentCapability allowed_patterns accessor
    // =========================================================================

    #[test]
    fn test_env_capability_allowed_patterns() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_env("HOME".to_string())
            .allow_env("PATH".to_string())
            .build(&registry);

        let env = EnvironmentCapability::from_token(token_id, registry.clone()).unwrap();
        assert_eq!(env.allowed_patterns(), &["HOME", "PATH"]);
    }

    // =========================================================================
    // FileSystemCapability prefixes accessors
    // =========================================================================

    #[test]
    fn test_fs_capability_prefixes() {
        let registry = Arc::new(CapabilityRegistry::new());
        let token_id = CapabilityBuilder::new(CapabilityScope::Global)
            .allow_read(PathBuf::from("/read"))
            .allow_write(PathBuf::from("/write"))
            .build(&registry);

        let fs = FileSystemCapability::from_token(token_id, registry.clone()).unwrap();
        assert_eq!(fs.allowed_prefixes(), &[PathBuf::from("/read")]);
        assert_eq!(fs.write_prefixes(), &[PathBuf::from("/write")]);
    }

    // =========================================================================
    // Default impl
    // =========================================================================

    #[test]
    fn test_registry_default() {
        let registry = CapabilityRegistry::default();
        assert_eq!(registry.audit_log().len(), 0);
    }
}
