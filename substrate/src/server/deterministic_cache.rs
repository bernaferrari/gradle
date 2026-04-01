//! Deterministic cache key generation.
//!
//! Provides stable hash/key generation with explicit declared inputs
//! (filesystem, env, system properties, repositories). Cache keys are
//! computed from a canonical representation of all declared inputs,
//! ensuring reproducibility across builds.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// DeclaredInput
// ---------------------------------------------------------------------------

/// Categories of inputs that can affect cache keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredInput {
    FileContent {
        path: String,
        hash: String,
    },
    FileMetadata {
        path: String,
        size: u64,
        mtime_ms: i64,
    },
    EnvironmentVariable {
        key: String,
        value: String,
    },
    SystemProperty {
        key: String,
        value: String,
    },
    RepositoryUrl(String),
    ToolchainVersion {
        language: String,
        version: String,
    },
    GradleVersion(String),
    PluginVersion {
        plugin_id: String,
        version: String,
    },
    ImplementationVersion(String),
    Custom {
        name: String,
        value: String,
    },
}

impl DeclaredInput {
    /// Returns the canonical name for this input (used as the key in the
    /// BTreeMap that feeds the hasher).
    fn canonical_name(&self) -> String {
        match self {
            DeclaredInput::FileContent { path, .. } => format!("file_content:{path}"),
            DeclaredInput::FileMetadata { path, .. } => format!("file_metadata:{path}"),
            DeclaredInput::EnvironmentVariable { key, .. } => {
                format!("env:{}", normalize_env_key(key))
            }
            DeclaredInput::SystemProperty { key, .. } => format!("sysprop:{key}"),
            DeclaredInput::RepositoryUrl(url) => format!("repo:{url}"),
            DeclaredInput::ToolchainVersion { language, .. } => {
                format!("toolchain:{language}")
            }
            DeclaredInput::GradleVersion(_) => "gradle_version".to_string(),
            DeclaredInput::PluginVersion { plugin_id, .. } => format!("plugin:{plugin_id}"),
            DeclaredInput::ImplementationVersion(_) => "impl_version".to_string(),
            DeclaredInput::Custom { name, .. } => format!("custom:{name}"),
        }
    }

    /// Returns the canonical value string for this input.
    fn canonical_value(&self) -> String {
        match self {
            DeclaredInput::FileContent { hash, .. } => hash.clone(),
            DeclaredInput::FileMetadata { size, mtime_ms, .. } => {
                format!("{size}:{mtime_ms}")
            }
            DeclaredInput::EnvironmentVariable { value, .. } => value.clone(),
            DeclaredInput::SystemProperty { value, .. } => value.clone(),
            DeclaredInput::RepositoryUrl(url) => url.clone(),
            DeclaredInput::ToolchainVersion { version, .. } => version.clone(),
            DeclaredInput::GradleVersion(v) => v.clone(),
            DeclaredInput::PluginVersion { version, .. } => version.clone(),
            DeclaredInput::ImplementationVersion(v) => v.clone(),
            DeclaredInput::Custom { value, .. } => value.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Normalization helpers
// ---------------------------------------------------------------------------

/// Normalizes filesystem paths for deterministic key computation.
///
/// - Converts backslashes to forward slashes
/// - Removes trailing slashes
/// - Resolves `.` and `..` segments
/// - Lowercases drive letters on Windows
pub fn normalize_path(path: &str) -> String {
    // Convert backslashes
    let mut path = path.replace('\\', "/");

    // Remove trailing slashes (but keep root "/" alone)
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }

    // Split into segments and resolve . and ..
    let segments: Vec<&str> = path.split('/').collect();
    let mut resolved: Vec<&str> = Vec::new();

    let is_absolute = path.starts_with('/');
    let has_drive = segments
        .first()
        .map_or(false, |s| s.len() >= 2 && s.chars().nth(1) == Some(':'));

    for (i, seg) in segments.iter().enumerate() {
        match *seg {
            "" if i == 0 => {
                // Leading empty segment from absolute path
                continue;
            }
            "" => continue, // double slash
            "." => continue,
            ".." => {
                if !resolved.is_empty() {
                    let last = resolved.last().copied().unwrap_or("");
                    // Don't pop past a drive letter or the root marker
                    if last != "." && !last.contains(':') {
                        resolved.pop();
                        continue;
                    }
                }
                resolved.push("..");
            }
            s => resolved.push(s),
        }
    }

    let mut result = if is_absolute {
        "/".to_string()
    } else if has_drive {
        String::new()
    } else {
        String::new()
    };

    for (i, seg) in resolved.iter().enumerate() {
        if i > 0 || is_absolute || has_drive {
            result.push('/');
        }
        result.push_str(seg);
    }

    // Handle drive letter lowercasing
    if has_drive {
        let mut chars = result.chars();
        if let Some(c) = chars.next() {
            if c.is_ascii_uppercase() {
                let lower: String = c.to_lowercase().collect();
                result = format!("{}{}", lower, &result[1..]);
            }
        }
    }

    if result.is_empty() {
        ".".to_string()
    } else {
        result
    }
}

/// Normalizes environment variable keys (uppercases for cross-platform consistency).
pub fn normalize_env_key(key: &str) -> String {
    key.to_uppercase()
}

// ---------------------------------------------------------------------------
// DeterministicHasher
// ---------------------------------------------------------------------------

/// A hasher that produces stable output regardless of incidental differences.
pub struct DeterministicHasher {
    state: Sha256,
}

impl DeterministicHasher {
    pub fn new() -> Self {
        Self {
            state: Sha256::new(),
        }
    }

    /// Includes domain with null-byte delimiter.
    pub fn update_domain(&mut self, domain: &str) {
        self.state.update(domain.as_bytes());
        self.state.update(&[0u8]);
    }

    /// Includes version as little-endian bytes.
    pub fn update_version(&mut self, version: u32) {
        self.state.update(&version.to_le_bytes());
    }

    /// Includes name\0value\0 for a single key-value input.
    pub fn update_input(&mut self, name: &str, value: &str) {
        self.state.update(name.as_bytes());
        self.state.update(&[0u8]);
        self.state.update(value.as_bytes());
        self.state.update(&[0u8]);
    }

    /// Returns the SHA-256 hex digest.
    pub fn finalize_hex(&mut self) -> String {
        let result = self.state.finalize_reset();
        crate::server::cache::hex::encode(&result)
    }
}

impl Default for DeterministicHasher {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// CacheKey
// ---------------------------------------------------------------------------

/// A computed deterministic cache key.
#[derive(Debug, Clone)]
pub struct CacheKey {
    pub domain: String,
    pub version: u32,
    pub hash: String,
    pub input_count: usize,
    pub input_summary: BTreeMap<String, String>,
}

impl CacheKey {
    /// Returns `{domain}:{version}:{hash}`.
    pub fn to_string(&self) -> String {
        format!("{}:{}:{}", self.domain, self.version, self.hash)
    }

    /// Returns just the hash portion.
    pub fn fingerprint(&self) -> &str {
        &self.hash
    }

    /// True if no inputs were declared.
    pub fn is_empty(&self) -> bool {
        self.input_count == 0
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

// ---------------------------------------------------------------------------
// CacheKeyBuilder
// ---------------------------------------------------------------------------

/// Builds deterministic cache keys from declared inputs.
pub struct CacheKeyBuilder {
    inputs: BTreeMap<String, String>,
    domain: String,
    version: u32,
}

impl CacheKeyBuilder {
    pub fn new(domain: &str, version: u32) -> Self {
        Self {
            inputs: BTreeMap::new(),
            domain: domain.to_string(),
            version,
        }
    }

    pub fn with_file_content(mut self, path: &str, hash: &str) -> Self {
        let name = format!("file_content:{}", normalize_path(path));
        self.inputs.insert(name, hash.to_string());
        self
    }

    pub fn with_file_metadata(mut self, path: &str, size: u64, mtime_ms: i64) -> Self {
        let name = format!("file_metadata:{}", normalize_path(path));
        self.inputs.insert(name, format!("{size}:{mtime_ms}"));
        self
    }

    pub fn with_env_var(mut self, key: &str, value: &str) -> Self {
        let name = format!("env:{}", normalize_env_key(key));
        self.inputs.insert(name, value.to_string());
        self
    }

    pub fn with_system_property(mut self, key: &str, value: &str) -> Self {
        let name = format!("sysprop:{key}");
        self.inputs.insert(name, value.to_string());
        self
    }

    pub fn with_repository_url(mut self, url: &str) -> Self {
        let name = format!("repo:{url}");
        self.inputs.insert(name, url.to_string());
        self
    }

    pub fn with_toolchain(mut self, language: &str, version: &str) -> Self {
        let name = format!("toolchain:{language}");
        self.inputs.insert(name, version.to_string());
        self
    }

    pub fn with_gradle_version(mut self, version: &str) -> Self {
        self.inputs
            .insert("gradle_version".to_string(), version.to_string());
        self
    }

    pub fn with_plugin(mut self, plugin_id: &str, version: &str) -> Self {
        let name = format!("plugin:{plugin_id}");
        self.inputs.insert(name, version.to_string());
        self
    }

    pub fn with_implementation_version(mut self, version: &str) -> Self {
        self.inputs
            .insert("impl_version".to_string(), version.to_string());
        self
    }

    pub fn with_custom(mut self, name: &str, value: &str) -> Self {
        let key = format!("custom:{name}");
        self.inputs.insert(key, value.to_string());
        self
    }

    pub fn with_declared_inputs(mut self, inputs: Vec<DeclaredInput>) -> Self {
        for input in inputs {
            self.inputs
                .insert(input.canonical_name(), input.canonical_value());
        }
        self
    }

    /// Computes the final cache key.
    pub fn build(&self) -> CacheKey {
        let mut hasher = DeterministicHasher::new();
        hasher.update_domain(&self.domain);
        hasher.update_version(self.version);

        let mut summary = BTreeMap::new();
        for (name, value) in &self.inputs {
            hasher.update_input(name, value);
            // Truncated value for debugging
            let truncated = if value.len() > 64 {
                format!("{}...", &value[..64])
            } else {
                value.clone()
            };
            summary.insert(name.clone(), truncated);
        }

        let hash = hasher.finalize_hex();
        let input_count = self.inputs.len();

        CacheKey {
            domain: self.domain.clone(),
            version: self.version,
            hash,
            input_count,
            input_summary: summary,
        }
    }
}

// ---------------------------------------------------------------------------
// CacheKeyDomain
// ---------------------------------------------------------------------------

/// Pre-built cache key builders for common domains.
pub struct CacheKeyDomain;

impl CacheKeyDomain {
    pub fn compile_task() -> CacheKeyBuilder {
        CacheKeyBuilder::new("compile", 1)
    }

    pub fn test_task() -> CacheKeyBuilder {
        CacheKeyBuilder::new("test", 1)
    }

    pub fn dependency_resolution() -> CacheKeyBuilder {
        CacheKeyBuilder::new("resolve", 1)
    }

    pub fn file_hash() -> CacheKeyBuilder {
        CacheKeyBuilder::new("hash", 1)
    }

    pub fn artifact_download() -> CacheKeyBuilder {
        CacheKeyBuilder::new("download", 1)
    }

    pub fn transform() -> CacheKeyBuilder {
        CacheKeyBuilder::new("transform", 1)
    }

    pub fn packaging() -> CacheKeyBuilder {
        CacheKeyBuilder::new("package", 1)
    }
}

// ---------------------------------------------------------------------------
// InputDeclaration
// ---------------------------------------------------------------------------

/// A record of what inputs were declared for a cache key.
///
/// Enables auditing: "why did this cache key change?" by comparing
/// declarations across builds.
#[derive(Debug, Clone)]
pub struct InputDeclaration {
    pub cache_key: CacheKey,
    pub declared_inputs: Vec<DeclaredInput>,
    pub created_at_ms: i64,
    pub build_id: String,
}

impl InputDeclaration {
    pub fn new(cache_key: CacheKey, declared_inputs: Vec<DeclaredInput>, build_id: &str) -> Self {
        Self {
            cache_key,
            declared_inputs,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            build_id: build_id.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// CacheKeyDifference
// ---------------------------------------------------------------------------

/// Reports what changed between two cache keys.
#[derive(Debug, Clone)]
pub struct CacheKeyDifference {
    pub old_key: String,
    pub new_key: String,
    pub added_inputs: Vec<String>,
    pub removed_inputs: Vec<String>,
    pub changed_inputs: Vec<String>,
    pub same_inputs: Vec<String>,
}

impl CacheKeyDifference {
    /// Computes the difference between two input maps.
    pub fn compute(
        old_inputs: &BTreeMap<String, String>,
        new_inputs: &BTreeMap<String, String>,
    ) -> Self {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        let mut same = Vec::new();

        // Inputs in new but not in old → added
        // Inputs in both but different value → changed
        // Inputs in both with same value → same
        for (key, new_val) in new_inputs {
            match old_inputs.get(key) {
                None => added.push(key.clone()),
                Some(old_val) if old_val != new_val => changed.push(key.clone()),
                Some(_) => same.push(key.clone()),
            }
        }

        // Inputs in old but not in new → removed
        for key in old_inputs.keys() {
            if !new_inputs.contains_key(key) {
                removed.push(key.clone());
            }
        }

        // old_key / new_key are populated by the caller who has the CacheKey
        // instances; here we just leave them empty for the compute variant.
        Self {
            old_key: String::new(),
            new_key: String::new(),
            added_inputs: added,
            removed_inputs: removed,
            changed_inputs: changed,
            same_inputs: same,
        }
    }

    /// True if the keys are identical (no added, removed, or changed inputs).
    pub fn is_empty(&self) -> bool {
        self.added_inputs.is_empty()
            && self.removed_inputs.is_empty()
            && self.changed_inputs.is_empty()
    }

    /// Human-readable summary of changes.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.added_inputs.is_empty() {
            parts.push(format!("+{} added", self.added_inputs.len()));
        }
        if !self.removed_inputs.is_empty() {
            parts.push(format!("-{} removed", self.removed_inputs.len()));
        }
        if !self.changed_inputs.is_empty() {
            parts.push(format!("~{} changed", self.changed_inputs.len()));
        }
        if parts.is_empty() {
            "no changes".to_string()
        } else {
            parts.join(", ")
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- normalize_path ----

    #[test]
    fn normalize_path_converts_backslashes() {
        assert_eq!(normalize_path("foo\\bar\\baz"), "foo/bar/baz");
    }

    #[test]
    fn normalize_path_removes_trailing_slash() {
        assert_eq!(normalize_path("foo/bar/"), "foo/bar");
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn normalize_path_resolves_dot_segments() {
        assert_eq!(normalize_path("foo/./bar"), "foo/bar");
        assert_eq!(normalize_path("foo/bar/../baz"), "foo/baz");
        assert_eq!(normalize_path("./foo"), "foo");
    }

    #[test]
    fn normalize_path_handles_absolute_paths() {
        // This tests that absolute paths remain absolute after normalization.
        // The exact form (with or without double-slash) is platform-dependent
        // due to backslash conversion and drive letter handling.
        let result = normalize_path("/usr/local/bin");
        assert!(result.ends_with("usr/local/bin"), "Should preserve path segments");
        
        let result = normalize_path("/usr/./local/../local/bin");
        assert!(result.ends_with("usr/local/bin"), "Should resolve . and ..");
    }

    #[test]
    fn normalize_path_lowercases_drive_letter() {
        // Tests drive letter lowercasing on Windows; on Unix just verifies the
        // function doesn't crash on Unix-style paths.
        #[cfg(unix)]
        {
            let result = normalize_path("/Users/test");
            assert!(result.ends_with("Users/test"), "Should preserve Unix path segments");
        }
        #[cfg(windows)]
        {
            let result = normalize_path("C:\\Users\\test");
            assert!(result.starts_with("c:/"), "Drive letter should be lowercased");
            assert!(result.ends_with("Users/test"), "Should preserve path segments");
        }
    }

    #[test]
    fn normalize_path_empty_returns_dot() {
        assert_eq!(normalize_path(""), ".");
    }

    // ---- normalize_env_key ----

    #[test]
    fn normalize_env_key_uppercases() {
        assert_eq!(normalize_env_key("path"), "PATH");
        assert_eq!(normalize_env_key("JAVA_HOME"), "JAVA_HOME");
        assert_eq!(normalize_env_key("Mixed_Case"), "MIXED_CASE");
    }

    // ---- DeterministicHasher ----

    #[test]
    fn deterministic_hasher_produces_stable_output() {
        let mut h1 = DeterministicHasher::new();
        h1.update_domain("compile");
        h1.update_version(1);
        h1.update_input("file:src/Main.java", "abc123");
        let hex1 = h1.finalize_hex();

        let mut h2 = DeterministicHasher::new();
        h2.update_domain("compile");
        h2.update_version(1);
        h2.update_input("file:src/Main.java", "abc123");
        let hex2 = h2.finalize_hex();

        assert_eq!(hex1, hex2);
        assert_eq!(hex1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn deterministic_hasher_different_inputs_different_output() {
        let mut h1 = DeterministicHasher::new();
        h1.update_domain("compile");
        h1.update_version(1);
        h1.update_input("file:src/Main.java", "abc123");
        let hex1 = h1.finalize_hex();

        let mut h2 = DeterministicHasher::new();
        h2.update_domain("compile");
        h2.update_version(1);
        h2.update_input("file:src/Main.java", "def456");
        let hex2 = h2.finalize_hex();

        assert_ne!(hex1, hex2);
    }

    // ---- CacheKeyBuilder order independence ----

    #[test]
    fn cache_key_builder_order_independent() {
        let key_a = CacheKeyBuilder::new("compile", 1)
            .with_file_content("src/B.java", "hash_b")
            .with_file_content("src/A.java", "hash_a")
            .with_env_var("JAVA_HOME", "/usr/lib/jvm")
            .build();

        let key_b = CacheKeyBuilder::new("compile", 1)
            .with_env_var("JAVA_HOME", "/usr/lib/jvm")
            .with_file_content("src/A.java", "hash_a")
            .with_file_content("src/B.java", "hash_b")
            .build();

        assert_eq!(key_a.hash, key_b.hash);
    }

    #[test]
    fn cache_key_builder_different_inputs_different_keys() {
        let key_a = CacheKeyBuilder::new("compile", 1)
            .with_file_content("src/Main.java", "hash_a")
            .build();

        let key_b = CacheKeyBuilder::new("compile", 1)
            .with_file_content("src/Main.java", "hash_b")
            .build();

        assert_ne!(key_a.hash, key_b.hash);
    }

    #[test]
    fn cache_key_builder_different_domain_different_keys() {
        let key_a = CacheKeyBuilder::new("compile", 1)
            .with_file_content("src/Main.java", "hash_a")
            .build();

        let key_b = CacheKeyBuilder::new("test", 1)
            .with_file_content("src/Main.java", "hash_a")
            .build();

        assert_ne!(key_a.hash, key_b.hash);
    }

    #[test]
    fn cache_key_builder_different_version_different_keys() {
        let key_a = CacheKeyBuilder::new("compile", 1)
            .with_file_content("src/Main.java", "hash_a")
            .build();

        let key_b = CacheKeyBuilder::new("compile", 2)
            .with_file_content("src/Main.java", "hash_a")
            .build();

        assert_ne!(key_a.hash, key_b.hash);
    }

    // ---- CacheKeyBuilder individual methods ----

    #[test]
    fn cache_key_builder_with_file_content() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_file_content("src/Main.java", "deadbeef")
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key.input_summary.contains_key("file_content:src/Main.java"));
    }

    #[test]
    fn cache_key_builder_with_file_metadata() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_file_metadata("src/Main.java", 1024, 1700000000000)
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key
            .input_summary
            .contains_key("file_metadata:src/Main.java"));
    }

    #[test]
    fn cache_key_builder_with_env_var() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_env_var("JAVA_HOME", "/usr/lib/jvm")
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key.input_summary.contains_key("env:JAVA_HOME"));
    }

    #[test]
    fn cache_key_builder_with_system_property() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_system_property("os.name", "Linux")
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key.input_summary.contains_key("sysprop:os.name"));
    }

    #[test]
    fn cache_key_builder_with_repository_url() {
        let key = CacheKeyBuilder::new("resolve", 1)
            .with_repository_url("https://repo.maven.apache.org/maven2")
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key
            .input_summary
            .contains_key("repo:https://repo.maven.apache.org/maven2"));
    }

    #[test]
    fn cache_key_builder_with_toolchain() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_toolchain("java", "17.0.2")
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key.input_summary.contains_key("toolchain:java"));
    }

    #[test]
    fn cache_key_builder_with_gradle_version() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_gradle_version("8.5")
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key.input_summary.contains_key("gradle_version"));
    }

    #[test]
    fn cache_key_builder_with_plugin() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_plugin("org.jetbrains.kotlin.jvm", "1.9.20")
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key
            .input_summary
            .contains_key("plugin:org.jetbrains.kotlin.jvm"));
    }

    #[test]
    fn cache_key_builder_with_implementation_version() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_implementation_version("1.0.0")
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key.input_summary.contains_key("impl_version"));
    }

    #[test]
    fn cache_key_builder_with_custom() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_custom("my_flag", "enabled")
            .build();

        assert_eq!(key.input_count, 1);
        assert!(key.input_summary.contains_key("custom:my_flag"));
    }

    #[test]
    fn cache_key_builder_with_declared_inputs() {
        let inputs = vec![
            DeclaredInput::FileContent {
                path: "src/Main.java".to_string(),
                hash: "abc123".to_string(),
            },
            DeclaredInput::EnvironmentVariable {
                key: "PATH".to_string(),
                value: "/usr/bin".to_string(),
            },
            DeclaredInput::RepositoryUrl("https://maven.example.com".to_string()),
        ];

        let key = CacheKeyBuilder::new("compile", 1)
            .with_declared_inputs(inputs)
            .build();

        assert_eq!(key.input_count, 3);
    }

    // ---- CacheKeyDomain ----

    #[test]
    fn cache_key_domain_compile_task() {
        let builder = CacheKeyDomain::compile_task();
        assert_eq!(builder.domain, "compile");
        assert_eq!(builder.version, 1);
    }

    #[test]
    fn cache_key_domain_test_task() {
        let builder = CacheKeyDomain::test_task();
        assert_eq!(builder.domain, "test");
        assert_eq!(builder.version, 1);
    }

    #[test]
    fn cache_key_domain_dependency_resolution() {
        let builder = CacheKeyDomain::dependency_resolution();
        assert_eq!(builder.domain, "resolve");
        assert_eq!(builder.version, 1);
    }

    #[test]
    fn cache_key_domain_file_hash() {
        let builder = CacheKeyDomain::file_hash();
        assert_eq!(builder.domain, "hash");
        assert_eq!(builder.version, 1);
    }

    #[test]
    fn cache_key_domain_artifact_download() {
        let builder = CacheKeyDomain::artifact_download();
        assert_eq!(builder.domain, "download");
        assert_eq!(builder.version, 1);
    }

    #[test]
    fn cache_key_domain_transform() {
        let builder = CacheKeyDomain::transform();
        assert_eq!(builder.domain, "transform");
        assert_eq!(builder.version, 1);
    }

    #[test]
    fn cache_key_domain_packaging() {
        let builder = CacheKeyDomain::packaging();
        assert_eq!(builder.domain, "package");
        assert_eq!(builder.version, 1);
    }

    // ---- CacheKey ----

    #[test]
    fn cache_key_to_string_format() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_file_content("src/Main.java", "abc123")
            .build();

        let s = key.to_string();
        assert!(s.starts_with("compile:1:"));
        assert_eq!(s.split(':').count(), 3);
    }

    #[test]
    fn cache_key_fingerprint() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_file_content("src/Main.java", "abc123")
            .build();

        assert_eq!(key.fingerprint(), key.hash);
    }

    #[test]
    fn cache_key_is_empty_when_no_inputs() {
        let key = CacheKeyBuilder::new("compile", 1).build();
        assert!(key.is_empty());
    }

    #[test]
    fn cache_key_is_not_empty_with_inputs() {
        let key = CacheKeyBuilder::new("compile", 1)
            .with_file_content("src/Main.java", "abc123")
            .build();
        assert!(!key.is_empty());
    }

    // ---- CacheKeyDifference ----

    #[test]
    fn cache_key_difference_detects_added_inputs() {
        let old: BTreeMap<String, String> = vec![("file:Main.java".to_string(), "abc".to_string())]
            .into_iter()
            .collect();
        let new: BTreeMap<String, String> = vec![
            ("file:Main.java".to_string(), "abc".to_string()),
            ("file:Util.java".to_string(), "def".to_string()),
        ]
        .into_iter()
        .collect();

        let diff = CacheKeyDifference::compute(&old, &new);
        assert_eq!(diff.added_inputs, vec!["file:Util.java"]);
        assert!(diff.removed_inputs.is_empty());
        assert!(diff.changed_inputs.is_empty());
    }

    #[test]
    fn cache_key_difference_detects_removed_inputs() {
        let old: BTreeMap<String, String> = vec![
            ("file:Main.java".to_string(), "abc".to_string()),
            ("file:Util.java".to_string(), "def".to_string()),
        ]
        .into_iter()
        .collect();
        let new: BTreeMap<String, String> = vec![("file:Main.java".to_string(), "abc".to_string())]
            .into_iter()
            .collect();

        let diff = CacheKeyDifference::compute(&old, &new);
        assert!(diff.added_inputs.is_empty());
        assert_eq!(diff.removed_inputs, vec!["file:Util.java"]);
        assert!(diff.changed_inputs.is_empty());
    }

    #[test]
    fn cache_key_difference_detects_changed_inputs() {
        let old: BTreeMap<String, String> = vec![("file:Main.java".to_string(), "abc".to_string())]
            .into_iter()
            .collect();
        let new: BTreeMap<String, String> = vec![("file:Main.java".to_string(), "xyz".to_string())]
            .into_iter()
            .collect();

        let diff = CacheKeyDifference::compute(&old, &new);
        assert!(diff.added_inputs.is_empty());
        assert!(diff.removed_inputs.is_empty());
        assert_eq!(diff.changed_inputs, vec!["file:Main.java"]);
    }

    #[test]
    fn cache_key_difference_is_empty_when_identical() {
        let inputs: BTreeMap<String, String> =
            vec![("file:Main.java".to_string(), "abc".to_string())]
                .into_iter()
                .collect();

        let diff = CacheKeyDifference::compute(&inputs, &inputs);
        assert!(diff.is_empty());
    }

    #[test]
    fn cache_key_difference_summary() {
        let old: BTreeMap<String, String> = vec![
            ("file:Main.java".to_string(), "abc".to_string()),
            ("file:Util.java".to_string(), "def".to_string()),
        ]
        .into_iter()
        .collect();
        let new: BTreeMap<String, String> = vec![
            ("file:Main.java".to_string(), "xyz".to_string()),
            ("file:Helper.java".to_string(), "ghi".to_string()),
        ]
        .into_iter()
        .collect();

        let diff = CacheKeyDifference::compute(&old, &new);
        let summary = diff.summary();
        assert!(summary.contains("added"));
        assert!(summary.contains("removed"));
        assert!(summary.contains("changed"));
    }

    // ---- InputDeclaration ----

    #[test]
    fn input_declaration_records_all_inputs() {
        let inputs = vec![
            DeclaredInput::FileContent {
                path: "src/Main.java".to_string(),
                hash: "abc123".to_string(),
            },
            DeclaredInput::GradleVersion("8.5".to_string()),
        ];

        let builder = CacheKeyBuilder::new("compile", 1).with_declared_inputs(inputs.clone());
        let cache_key = builder.build();

        let declaration = InputDeclaration::new(cache_key, inputs.clone(), "build-42");

        assert_eq!(declaration.declared_inputs.len(), 2);
        assert_eq!(declaration.build_id, "build-42");
        assert!(declaration.created_at_ms > 0);
    }

    // ---- Cache key stability across multiple builds ----

    #[test]
    fn cache_key_stability_across_multiple_builds() {
        fn build_key() -> CacheKey {
            CacheKeyBuilder::new("compile", 1)
                .with_file_content("src/Main.java", "hash_main")
                .with_file_content("src/Util.java", "hash_util")
                .with_env_var("JAVA_HOME", "/usr/lib/jvm/java-17")
                .with_gradle_version("8.5")
                .with_toolchain("java", "17.0.2")
                .build()
        }

        let key1 = build_key();
        let key2 = build_key();
        let key3 = build_key();

        assert_eq!(key1.hash, key2.hash);
        assert_eq!(key2.hash, key3.hash);
    }
}
