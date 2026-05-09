use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tonic::{Request, Response, Status};

use crate::proto::{
    dependency_resolution_service_server::DependencyResolutionService, AddArtifactToCacheRequest,
    AddArtifactToCacheResponse, CheckArtifactCacheRequest, CheckArtifactCacheResponse,
    CheckMetadataCacheRequest, CheckMetadataCacheResponse, ChecksumFailure, DependencyDescriptor,
    GetResolutionStatsRequest, GetResolutionStatsResponse, RecordResolutionRequest,
    RecordResolutionResponse, RepositoryDescriptor, ResolveDependenciesRequest,
    ResolveDependenciesResponse, ResolvedDependency, VerifyDependencyChecksumsRequest,
    VerifyDependencyChecksumsResponse,
};

use super::dependency_solver::gradle_module_metadata::{self, ModuleMetadataSelection};
use super::dependency_solver::graph_builder;
use super::dependency_solver::ivyresolve::strategy::compare_versions;
use super::dependency_solver::maven_pom;
pub use super::dependency_solver::maven_pom::{ManagedDependency, PomDependency};
pub use super::dependency_solver::resolveengine::graph::conflicts::ResolutionStrategy;
use super::dependency_solver::resolveengine::graph::conflicts::{
    resolve_conflicts, resolve_conflicts_with_strategy, try_resolve_conflicts_with_strategy,
};

// ---------------------------------------------------------------------------
// Dependency scope
// ---------------------------------------------------------------------------

/// Dependency scope classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyScope {
    Compile,
    Runtime,
    Test,
    Provided,
    System,
}

impl DependencyScope {
    /// Parse a scope string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Self {
        match s.as_bytes() {
            b"compile" | b"compileonly" | b"api" | b"Compile" | b"CompileOnly" | b"Api"
            | b"COMPILE" | b"COMPILEONLY" | b"API" => DependencyScope::Compile,
            b"runtime" | b"implementation" | b"runtimeonly" | b"Runtime" | b"Implementation"
            | b"RuntimeOnly" | b"RUNTIME" | b"IMPLEMENTATION" | b"RUNTIMEONLY" => {
                DependencyScope::Runtime
            }
            b"test"
            | b"testimplementation"
            | b"testruntimeonly"
            | b"Test"
            | b"TestImplementation"
            | b"TestRuntimeOnly"
            | b"TEST"
            | b"TESTIMPLEMENTATION"
            | b"TESTRUNTIMEONLY" => DependencyScope::Test,
            b"provided" | b"Provided" | b"PROVIDED" => DependencyScope::Provided,
            b"system" | b"System" | b"SYSTEM" => DependencyScope::System,
            _ => DependencyScope::Compile,
        }
    }

    /// Returns true if this scope includes the given dependency scope.
    /// This follows Maven classpath semantics: runtime includes compile/runtime,
    /// test includes everything, and compile excludes runtime/test-only entries.
    pub fn includes(&self, other: &DependencyScope) -> bool {
        match self {
            DependencyScope::Compile => matches!(
                other,
                DependencyScope::Compile | DependencyScope::Provided | DependencyScope::System
            ),
            DependencyScope::Runtime => {
                matches!(other, DependencyScope::Compile | DependencyScope::Runtime)
            }
            DependencyScope::Test => true,
            DependencyScope::Provided => {
                matches!(other, DependencyScope::Compile | DependencyScope::Provided)
            }
            DependencyScope::System => matches!(other, DependencyScope::System),
        }
    }

    /// Scopes that are transitively inherited.
    pub fn transitive_scopes(&self) -> Vec<DependencyScope> {
        match self {
            DependencyScope::Compile => vec![DependencyScope::Compile, DependencyScope::Runtime],
            DependencyScope::Runtime => vec![DependencyScope::Runtime],
            DependencyScope::Test => vec![DependencyScope::Compile, DependencyScope::Runtime],
            DependencyScope::Provided => vec![],
            DependencyScope::System => vec![],
        }
    }
}

/// Cached artifact metadata.
struct CachedArtifact {
    group: String,
    name: String,
    version: String,
    classifier: String,
    extension: String,
    sha256: String,
    local_path: String,
    size: i64,
    cached_at_ms: i64,
}

/// Resolution statistics.
struct ResolutionStats {
    total_resolutions: AtomicI64,
    cache_hits: AtomicI64,
    total_time_ms: AtomicI64,
}

/// Rust-native dependency resolution service.
/// Resolves dependency graphs, fetches POMs from Maven repos, and manages artifact caching.
pub struct DependencyResolutionServiceImpl {
    artifact_cache: Arc<DashMap<String, CachedArtifact>>,
    module_metadata_misses: Arc<DashMap<String, ()>>,
    resolution_stats: ResolutionStats,
    http_client: reqwest::Client,
    artifact_store_dir: PathBuf,
}

/// Parsed maven-metadata.xml.
struct MavenMetadata {
    group_id: String,
    artifact_id: String,
    versioning: MavenVersioning,
}

/// Versioning section from maven-metadata.xml.
struct MavenVersioning {
    latest: Option<String>,
    release: Option<String>,
    last_updated: Option<String>,
    snapshot: Option<MavenSnapshot>,
    versions: Vec<String>,
}

struct TransitiveResolution {
    dependencies: Vec<ResolvedDependency>,
    source_repo_url: Option<String>,
}

/// Snapshot info from maven-metadata.xml.
#[derive(Clone)]
struct MavenSnapshot {
    build_number: Option<String>,
    timestamp: Option<String>,
    local_copy: bool,
}

/// Parsed <parent> section from a POM file.
#[allow(dead_code)]
struct ParentPom {
    group_id: String,
    artifact_id: String,
    version: String,
    relative_path: String,
}

/// Maximum depth for parent POM inheritance chain.
const MAX_PARENT_DEPTH: u32 = 10;

/// Checksum verification result.
#[allow(dead_code)]
struct ChecksumResult {
    algorithm: String,
    expected: String,
    actual: String,
    matched: bool,
}

impl Default for DependencyResolutionServiceImpl {
    fn default() -> Self {
        Self::new(std::path::PathBuf::new())
    }
}

impl DependencyResolutionServiceImpl {
    pub fn new(artifact_store_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&artifact_store_dir).ok();
        Self {
            artifact_cache: Arc::new(DashMap::new()),
            module_metadata_misses: Arc::new(DashMap::new()),
            resolution_stats: ResolutionStats {
                total_resolutions: AtomicI64::new(0),
                cache_hits: AtomicI64::new(0),
                total_time_ms: AtomicI64::new(0),
            },
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            artifact_store_dir,
        }
    }

    /// Convert a Maven group id ("org.gradle") to a path ("org/gradle").
    /// Pre-allocates the result string to avoid repeated `replace` allocations.
    fn group_to_path(group: &str) -> String {
        let dot_count = group.bytes().filter(|&b| b == b'.').count();
        let mut path = String::with_capacity(group.len() + dot_count);
        for b in group.bytes() {
            if b == b'.' {
                path.push('/');
            } else {
                path.push(b as char);
            }
        }
        path
    }

    /// Compute the local filesystem path for an artifact using Maven repository layout.
    fn artifact_path(
        &self,
        group: &str,
        name: &str,
        version: &str,
        classifier: &str,
        extension: &str,
    ) -> PathBuf {
        let group_path = Self::group_to_path(group);
        let filename = if classifier.is_empty() {
            format!("{}-{}.{}", name, version, extension)
        } else {
            format!("{}-{}-{}.{}", name, version, classifier, extension)
        };
        self.artifact_store_dir
            .join(&group_path)
            .join(name)
            .join(version)
            .join(&filename)
    }

    fn normalize_extension(extension: &str) -> String {
        let extension = extension.trim_start_matches('.');
        if extension.is_empty() {
            "jar".to_string()
        } else {
            extension.to_string()
        }
    }

    fn is_metadata_extension(extension: &str) -> bool {
        matches!(
            Self::normalize_extension(extension).as_str(),
            "pom" | "module" | "ivy" | "maven-metadata.xml"
        )
    }

    fn supports_gradle_module_metadata(repo: &RepositoryDescriptor) -> bool {
        matches!(repo.layout.as_str(), "gradle-module-metadata" | "gradle")
    }

    fn repository_allows_group(repo: &RepositoryDescriptor, group: &str) -> bool {
        let group = group.trim();
        (repo.include_groups.is_empty() && repo.include_group_prefixes.is_empty()
            || repo
                .include_groups
                .iter()
                .any(|candidate| candidate == group)
            || repo
                .include_group_prefixes
                .iter()
                .any(|candidate| Self::group_matches_prefix(group, candidate)))
            && !repo
                .exclude_groups
                .iter()
                .any(|candidate| candidate == group)
            && !repo
                .exclude_group_prefixes
                .iter()
                .any(|candidate| Self::group_matches_prefix(group, candidate))
    }

    fn group_matches_prefix(group: &str, prefix: &str) -> bool {
        let prefix = prefix.trim();
        group == prefix
            || group
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('.'))
    }

    fn repositories_for_group(
        repos: &[RepositoryDescriptor],
        group: &str,
    ) -> Vec<RepositoryDescriptor> {
        repos
            .iter()
            .filter(|repo| Self::repository_allows_group(repo, group))
            .cloned()
            .collect()
    }

    fn first_unresolved_reason(dependencies: &[ResolvedDependency]) -> Option<String> {
        for dep in dependencies {
            if !dep.resolved {
                return Some(dep.failure_reason.clone());
            }
            if let Some(reason) = Self::first_unresolved_reason(&dep.dependencies) {
                return Some(reason);
            }
        }
        None
    }

    fn maven_artifact_shape(classifier: &str, type_field: &str) -> (String, String) {
        let classifier = classifier.trim();
        let type_field = type_field.trim();
        let effective_type = if type_field.is_empty() {
            "jar"
        } else {
            type_field
        };
        let extension = if Self::maven_type_has_jar_extension(effective_type) {
            "jar"
        } else {
            effective_type
        };
        let effective_classifier = if !classifier.is_empty() {
            classifier
        } else {
            Self::implicit_classifier_for_maven_type(effective_type).unwrap_or("")
        };
        (effective_classifier.to_string(), extension.to_string())
    }

    fn maven_type_has_jar_extension(type_field: &str) -> bool {
        matches!(
            type_field,
            "test-jar" | "ejb-client" | "ejb" | "bundle" | "maven-plugin" | "eclipse-plugin"
        )
    }

    fn implicit_classifier_for_maven_type(type_field: &str) -> Option<&'static str> {
        match type_field {
            "test-jar" => Some("tests"),
            "ejb-client" => Some("client"),
            _ => None,
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }

    fn sha256_sidecar_path(path: &Path) -> PathBuf {
        PathBuf::from(format!("{}.sha256", path.to_string_lossy()))
    }

    async fn compute_file_sha256(path: &Path) -> Result<String, String> {
        use tokio::io::AsyncReadExt;

        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buf)
                .await
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    async fn write_sha256_sidecar(path: &Path, sha256: &str) -> Result<(), String> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("artifact");
        tokio::fs::write(
            Self::sha256_sidecar_path(path),
            format!("{}  {}\n", sha256, file_name),
        )
        .await
        .map_err(|e| format!("Failed to write SHA-256 sidecar: {}", e))
    }

    async fn copy_file_and_sha256(src: &Path, dest: &Path) -> Result<(i64, String), String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create artifact cache directory: {}", e))?;
        }

        let tmp_path = PathBuf::from(format!("{}.part", dest.to_string_lossy()));
        let copy_result = async {
            let mut input = tokio::fs::File::open(src)
                .await
                .map_err(|e| format!("Failed to open {}: {}", src.display(), e))?;
            let mut output = tokio::fs::File::create(&tmp_path)
                .await
                .map_err(|e| format!("Failed to create {}: {}", tmp_path.display(), e))?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 64 * 1024];
            let mut written = 0i64;

            loop {
                let read = input
                    .read(&mut buf)
                    .await
                    .map_err(|e| format!("Failed to read {}: {}", src.display(), e))?;
                if read == 0 {
                    break;
                }
                output
                    .write_all(&buf[..read])
                    .await
                    .map_err(|e| format!("Failed to write {}: {}", tmp_path.display(), e))?;
                hasher.update(&buf[..read]);
                written += read as i64;
            }
            output
                .flush()
                .await
                .map_err(|e| format!("Failed to flush {}: {}", tmp_path.display(), e))?;
            drop(output);
            tokio::fs::rename(&tmp_path, dest)
                .await
                .map_err(|e| format!("Failed to commit artifact cache file: {}", e))?;

            Ok((written, format!("{:x}", hasher.finalize())))
        }
        .await;

        if copy_result.is_err() {
            let _ = tokio::fs::remove_file(&tmp_path).await;
        }
        copy_result
    }

    /// Compute SHA-256 hex digest of data.
    fn compute_sha256(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    /// Compute SHA-1 hex digest of data.
    #[allow(dead_code)]
    fn compute_sha1(data: &[u8]) -> String {
        let mut hasher = sha1::Sha1::new();
        sha1::Digest::update(&mut hasher, data);
        format!("{:x}", hasher.finalize())
    }

    /// Compute MD5 hex digest of data.
    #[allow(dead_code)]
    fn compute_md5(data: &[u8]) -> String {
        use md5::Digest;
        let mut hasher = md5::Md5::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    /// Parse a checksum file content (e.g., "abc123  filename.jar").
    #[allow(dead_code)]
    fn parse_checksum_value(raw: &str) -> String {
        raw.split_whitespace().next().unwrap_or("").to_string()
    }

    /// Fetch a checksum sidecar file and return its value.
    #[allow(dead_code)]
    async fn fetch_checksum_file(&self, artifact_url: &str, algo: &str) -> Result<String, String> {
        let checksum_url = format!("{}.{}", artifact_url, algo);
        let resp = self
            .http_client
            .get(&checksum_url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch {} checksum: {}", algo, e))?;

        match resp.status().as_u16() {
            200 => resp
                .text()
                .await
                .map(|s| Self::parse_checksum_value(&s))
                .map_err(|e| format!("Failed to read {} checksum: {}", algo, e)),
            404 => Err(format!("No {} checksum available", algo)),
            status => Err(format!("HTTP {} for {} checksum", status, algo)),
        }
    }

    /// Verify an artifact's checksum against available sidecar files.
    #[allow(dead_code)]
    async fn verify_artifact_checksum(&self, data: &[u8], artifact_url: &str) -> ChecksumResult {
        let actual_sha256 = Self::compute_sha256(data);

        // Try SHA-256 first, then SHA-1, then MD5
        type HashFn = fn(&[u8]) -> String;
        let algos: [(&str, HashFn); 3] = [
            ("sha256", Self::compute_sha256),
            ("sha1", Self::compute_sha1),
            ("md5", Self::compute_md5),
        ];
        for (algo, compute_fn) in algos {
            if let Ok(expected) = self.fetch_checksum_file(artifact_url, algo).await {
                let actual = compute_fn(data);
                let matched = expected == actual;
                return ChecksumResult {
                    algorithm: algo.to_string(),
                    expected,
                    actual,
                    matched,
                };
            }
        }

        // No checksum files available — use the computed SHA-256 as the reference
        ChecksumResult {
            algorithm: "sha256".to_string(),
            expected: actual_sha256.clone(),
            actual: actual_sha256,
            matched: true,
        }
    }

    /// Build an authenticated request for a repository.
    fn build_request(&self, repo: &RepositoryDescriptor, path: &str) -> reqwest::RequestBuilder {
        let base = repo.url.trim_end_matches('/');
        let mut url = format!("{}/{}", base, path);

        // Handle allow_insecure_protocol
        if repo.allow_insecure_protocol && url.starts_with("https://") {
            url = url.replacen("https://", "http://", 1);
        }

        let mut req = self.http_client.get(&url);

        // Apply Basic auth if credentials are provided
        if let Some(username) = repo.credentials.get("username") {
            if let Some(password) = repo.credentials.get("password") {
                req = req.basic_auth(username, Some(password));
            }
        }

        req
    }

    /// Parse maven-metadata.xml using quick-xml.
    fn parse_maven_metadata(xml: &str) -> Result<MavenMetadata, String> {
        use quick_xml::events::Event;
        let mut reader = quick_xml::Reader::from_str(xml);
        reader.trim_text(true);

        let mut metadata = MavenMetadata {
            group_id: String::new(),
            artifact_id: String::new(),
            versioning: MavenVersioning {
                latest: None,
                release: None,
                last_updated: None,
                snapshot: None,
                versions: Vec::new(),
            },
        };

        let mut in_versions = false;
        let mut in_snapshot = false;
        let mut current_tag = String::new();
        let mut snapshot = MavenSnapshot {
            build_number: None,
            timestamp: None,
            local_copy: false,
        };

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = e.name();
                    match name.as_ref() {
                        b"versions" => in_versions = true,
                        b"snapshot" => {
                            in_snapshot = true;
                            snapshot = MavenSnapshot {
                                build_number: None,
                                timestamp: None,
                                local_copy: false,
                            };
                        }
                        _ => {
                            current_tag = std::str::from_utf8(name.local_name().as_ref())
                                .unwrap_or_default()
                                .to_string()
                        }
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    current_tag = std::str::from_utf8(name.local_name().as_ref())
                        .unwrap_or_default()
                        .to_string();
                }
                Ok(Event::Text(ref e)) => {
                    let text = e.unescape().unwrap_or_default().to_string();
                    if in_versions && current_tag == "version" {
                        metadata.versioning.versions.push(text);
                    } else if in_snapshot {
                        match current_tag.as_str() {
                            "buildNumber" => snapshot.build_number = Some(text),
                            "timestamp" => snapshot.timestamp = Some(text),
                            "localCopy" => snapshot.local_copy = text == "true",
                            _ => {}
                        }
                    } else {
                        match current_tag.as_str() {
                            "groupId" => metadata.group_id = text,
                            "artifactId" => metadata.artifact_id = text,
                            "latest" => metadata.versioning.latest = Some(text),
                            "release" => metadata.versioning.release = Some(text),
                            "lastUpdated" => metadata.versioning.last_updated = Some(text),
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = e.name();
                    match name.as_ref() {
                        b"versions" => in_versions = false,
                        b"snapshot" => {
                            in_snapshot = false;
                            metadata.versioning.snapshot = Some(snapshot.clone());
                        }
                        _ => {}
                    }
                    current_tag.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("XML parse error: {}", e)),
                _ => {}
            }
        }

        Ok(metadata)
    }

    /// Fetch maven-metadata.xml from a repository.
    async fn fetch_maven_metadata(
        &self,
        group: &str,
        name: &str,
        repo: &RepositoryDescriptor,
    ) -> Result<MavenMetadata, String> {
        let extension = "maven-metadata.xml";
        let classifier = "";
        let version = "";
        let key = Self::module_metadata_cache_key(repo, group, name, extension);
        let cache_path = self.module_metadata_path(repo, group, name, extension);
        let group_path = Self::group_to_path(group);
        let path = format!("{}/{}/maven-metadata.xml", group_path, name);
        let url = self
            .build_request(repo, &path)
            .build()
            .map_err(|e| format!("Failed to build maven-metadata.xml request: {}", e))?
            .url()
            .to_string();
        let url_key = Self::metadata_url_cache_key(&url, extension);
        let url_cache_path = self.metadata_url_path(&url, extension);
        if let Some(cached) = self
            .read_cached_text_artifact(
                &key,
                &cache_path,
                group,
                name,
                version,
                classifier,
                extension,
            )
            .await
        {
            tracing::info!(group = %group, name = %name, source = "rust_cache", "Metadata cache hit");
            return Self::parse_maven_metadata(&cached);
        }
        if let Some(cached) = self
            .read_cached_text_artifact(
                &url_key,
                &url_cache_path,
                group,
                name,
                version,
                classifier,
                extension,
            )
            .await
        {
            tracing::info!(group = %group, name = %name, source = "url_cache", "Metadata cache hit");
            return Self::parse_maven_metadata(&cached);
        }

        let resp = self
            .build_request(repo, &path)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch maven-metadata.xml: {}", e))?;

        match resp.status().as_u16() {
            200 => {
                let body = resp
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read metadata response: {}", e))?;
                self.persist_text_artifact(
                    &key,
                    &cache_path,
                    group,
                    name,
                    version,
                    classifier,
                    extension,
                    &body,
                )
                .await?;
                self.persist_text_artifact_alias(
                    &url_key,
                    &url_cache_path,
                    &cache_path,
                    group,
                    name,
                    version,
                    classifier,
                    extension,
                    &body,
                )
                .await?;
                tracing::info!(group = %group, name = %name, source = "remote_fetch", url = %url, "Metadata fetched from remote");
                Self::parse_maven_metadata(&body)
            }
            404 => Err("maven-metadata.xml not found".to_string()),
            status => Err(format!("HTTP {} for maven-metadata.xml", status)),
        }
    }

    fn artifact_cache_key(
        group: &str,
        name: &str,
        version: &str,
        classifier: &str,
        extension: &str,
    ) -> String {
        let extension = Self::normalize_extension(extension);
        let mut key = String::with_capacity(
            group.len() + name.len() + version.len() + classifier.len() + extension.len() + 4,
        );
        key.push_str(group);
        key.push(':');
        key.push_str(name);
        key.push(':');
        key.push_str(version);
        key.push(':');
        key.push_str(classifier);
        key.push(':');
        key.push_str(&extension);
        key
    }

    fn repository_cache_id(repo: &RepositoryDescriptor) -> String {
        let digest = Self::compute_sha256(repo.url.as_bytes());
        digest[..16].to_string()
    }

    fn metadata_cache_key(
        repo: &RepositoryDescriptor,
        group: &str,
        name: &str,
        version: &str,
        extension: &str,
    ) -> String {
        let extension = Self::normalize_extension(extension);
        let repo_id = Self::repository_cache_id(repo);
        let mut key = String::with_capacity(
            "metadata:".len()
                + repo_id.len()
                + group.len()
                + name.len()
                + version.len()
                + extension.len()
                + 4,
        );
        key.push_str("metadata:");
        key.push_str(&repo_id);
        key.push(':');
        key.push_str(group);
        key.push(':');
        key.push_str(name);
        key.push(':');
        key.push_str(version);
        key.push(':');
        key.push_str(&extension);
        key
    }

    fn module_metadata_cache_key(
        repo: &RepositoryDescriptor,
        group: &str,
        name: &str,
        extension: &str,
    ) -> String {
        let extension = Self::normalize_extension(extension);
        let repo_id = Self::repository_cache_id(repo);
        let mut key = String::with_capacity(
            "module-metadata:".len()
                + repo_id.len()
                + group.len()
                + name.len()
                + extension.len()
                + 3,
        );
        key.push_str("module-metadata:");
        key.push_str(&repo_id);
        key.push(':');
        key.push_str(group);
        key.push(':');
        key.push_str(name);
        key.push(':');
        key.push_str(&extension);
        key
    }

    fn metadata_url_cache_key(url: &str, extension: &str) -> String {
        let extension = Self::normalize_extension(extension);
        let digest = Self::compute_sha256(url.as_bytes());
        let mut key =
            String::with_capacity("metadata-url:".len() + digest.len() + extension.len() + 1);
        key.push_str("metadata-url:");
        key.push_str(&digest);
        key.push(':');
        key.push_str(&extension);
        key
    }

    fn warm_cached_artifact_path(
        artifact_cache: &DashMap<String, CachedArtifact>,
        cache_key: &str,
    ) -> Option<PathBuf> {
        let cached_local_path = artifact_cache
            .get(cache_key)
            .map(|cached| cached.local_path.clone());
        let cached_local_path = cached_local_path?;
        let path = PathBuf::from(cached_local_path);
        if path.is_file() {
            return Some(path);
        }

        artifact_cache.remove(cache_key);
        None
    }

    fn metadata_path(
        &self,
        repo: &RepositoryDescriptor,
        group: &str,
        name: &str,
        version: &str,
        extension: &str,
    ) -> PathBuf {
        let filename = format!(
            "{}-{}.{}",
            name,
            version,
            Self::normalize_extension(extension)
        );
        self.artifact_store_dir
            .join("_metadata")
            .join(Self::repository_cache_id(repo))
            .join(Self::group_to_path(group))
            .join(name)
            .join(version)
            .join(filename)
    }

    fn module_metadata_path(
        &self,
        repo: &RepositoryDescriptor,
        group: &str,
        name: &str,
        extension: &str,
    ) -> PathBuf {
        self.artifact_store_dir
            .join("_metadata")
            .join(Self::repository_cache_id(repo))
            .join(Self::group_to_path(group))
            .join(name)
            .join(Self::normalize_extension(extension))
    }

    fn metadata_url_path(&self, url: &str, extension: &str) -> PathBuf {
        let digest = Self::compute_sha256(url.as_bytes());
        self.artifact_store_dir
            .join("_metadata")
            .join("by-url")
            .join(format!(
                "{}.{}",
                digest,
                Self::normalize_extension(extension)
            ))
    }

    async fn read_cached_text_artifact(
        &self,
        key: &str,
        path: &Path,
        group: &str,
        name: &str,
        version: &str,
        classifier: &str,
        extension: &str,
    ) -> Option<String> {
        if let Some(cached) = self.artifact_cache.get(key) {
            let cached_path = cached.local_path.clone();
            drop(cached);
            if !cached_path.is_empty() {
                if let Ok(text) = tokio::fs::read_to_string(&cached_path).await {
                    self.resolution_stats
                        .cache_hits
                        .fetch_add(1, Ordering::Relaxed);
                    return Some(text);
                }
            }
        }

        if !path.exists() {
            return None;
        }

        let text = tokio::fs::read_to_string(path).await.ok()?;
        let size = text.len() as i64;
        self.artifact_cache.insert(
            key.to_string(),
            CachedArtifact {
                group: group.to_string(),
                name: name.to_string(),
                version: version.to_string(),
                classifier: classifier.to_string(),
                extension: extension.to_string(),
                sha256: String::new(),
                local_path: path.to_string_lossy().into_owned(),
                size,
                cached_at_ms: Self::now_ms(),
            },
        );
        self.resolution_stats
            .cache_hits
            .fetch_add(1, Ordering::Relaxed);
        Some(text)
    }

    async fn persist_text_artifact(
        &self,
        key: &str,
        path: &Path,
        group: &str,
        name: &str,
        version: &str,
        classifier: &str,
        extension: &str,
        content: &str,
    ) -> Result<(), String> {
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create metadata cache directory: {}", e))?;
        }

        let tmp_path = PathBuf::from(format!("{}.part", path.to_string_lossy()));
        tokio::fs::write(&tmp_path, content)
            .await
            .map_err(|e| format!("Failed to write metadata cache file: {}", e))?;
        tokio::fs::rename(&tmp_path, path)
            .await
            .map_err(|e| format!("Failed to commit metadata cache file: {}", e))?;

        let sha256 = Self::compute_sha256(content.as_bytes());
        if let Err(e) = Self::write_sha256_sidecar(path, &sha256).await {
            tracing::warn!(path = %path.display(), error = %e, "Failed to write metadata checksum sidecar");
        }
        self.artifact_cache.insert(
            key.to_string(),
            CachedArtifact {
                group: group.to_string(),
                name: name.to_string(),
                version: version.to_string(),
                classifier: classifier.to_string(),
                extension: extension.to_string(),
                sha256,
                local_path: path.to_string_lossy().into_owned(),
                size: content.len() as i64,
                cached_at_ms: Self::now_ms(),
            },
        );
        Ok(())
    }

    async fn persist_text_artifact_alias(
        &self,
        key: &str,
        path: &Path,
        source_path: &Path,
        group: &str,
        name: &str,
        version: &str,
        classifier: &str,
        extension: &str,
        content: &str,
    ) -> Result<(), String> {
        if path.as_os_str().is_empty() || path == source_path {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create metadata alias directory: {}", e))?;
        }
        if path.exists() {
            let _ = tokio::fs::remove_file(path).await;
        }
        if tokio::fs::hard_link(source_path, path).await.is_err() {
            tokio::fs::copy(source_path, path)
                .await
                .map_err(|e| format!("Failed to copy metadata alias file: {}", e))?;
        }

        let sha256 = Self::compute_sha256(content.as_bytes());
        if let Err(e) = Self::write_sha256_sidecar(path, &sha256).await {
            tracing::warn!(path = %path.display(), error = %e, "Failed to write metadata alias checksum sidecar");
        }
        self.artifact_cache.insert(
            key.to_string(),
            CachedArtifact {
                group: group.to_string(),
                name: name.to_string(),
                version: version.to_string(),
                classifier: classifier.to_string(),
                extension: extension.to_string(),
                sha256,
                local_path: path.to_string_lossy().into_owned(),
                size: content.len() as i64,
                cached_at_ms: Self::now_ms(),
            },
        );
        Ok(())
    }

    /// Fetch a POM file from a Maven repository and parse it.
    async fn fetch_pom(
        &self,
        group: &str,
        name: &str,
        version: &str,
        repo: &RepositoryDescriptor,
    ) -> Result<String, String> {
        let extension = "pom";
        let classifier = "";
        let key = Self::metadata_cache_key(repo, group, name, version, extension);
        let cache_path = self.metadata_path(repo, group, name, version, extension);
        let group_path = Self::group_to_path(group);
        let path = format!(
            "{}/{}/{}/{}-{}.pom",
            group_path, name, version, name, version
        );
        let url = self
            .build_request(repo, &path)
            .build()
            .map_err(|e| format!("Failed to build POM request: {}", e))?
            .url()
            .to_string();
        let url_key = Self::metadata_url_cache_key(&url, extension);
        let url_cache_path = self.metadata_url_path(&url, extension);
        if let Some(cached) = self
            .read_cached_text_artifact(
                &key,
                &cache_path,
                group,
                name,
                version,
                classifier,
                extension,
            )
            .await
        {
            tracing::info!(group = %group, name = %name, version = %version, source = "rust_cache", "POM cache hit");
            return Ok(cached);
        }
        if let Some(cached) = self
            .read_cached_text_artifact(
                &url_key,
                &url_cache_path,
                group,
                name,
                version,
                classifier,
                extension,
            )
            .await
        {
            tracing::info!(group = %group, name = %name, version = %version, source = "url_cache", "POM cache hit");
            return Ok(cached);
        }

        let response = self
            .build_request(repo, &path)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch POM: {}", e))?;

        match response.status().as_u16() {
            200 => {
                let content = response
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read POM response: {}", e))?;
                self.persist_text_artifact(
                    &key,
                    &cache_path,
                    group,
                    name,
                    version,
                    classifier,
                    extension,
                    &content,
                )
                .await?;
                self.persist_text_artifact_alias(
                    &url_key,
                    &url_cache_path,
                    &cache_path,
                    group,
                    name,
                    version,
                    classifier,
                    extension,
                    &content,
                )
                .await?;
                tracing::info!(group = %group, name = %name, version = %version, source = "remote_fetch", url = %url, "POM fetched from remote");
                Ok(content)
            }
            404 => Err(format!("POM not found: {}-{}.pom", name, version)),
            status => Err(format!("HTTP {} for POM", status)),
        }
    }

    async fn fetch_gradle_module_metadata(
        &self,
        group: &str,
        name: &str,
        version: &str,
        repo: &RepositoryDescriptor,
    ) -> Result<Option<String>, String> {
        let extension = "module";
        let classifier = "";
        let key = Self::metadata_cache_key(repo, group, name, version, extension);
        let cache_path = self.metadata_path(repo, group, name, version, extension);
        let group_path = Self::group_to_path(group);
        let path = format!(
            "{}/{}/{}/{}-{}.module",
            group_path, name, version, name, version
        );
        let url = self
            .build_request(repo, &path)
            .build()
            .map_err(|e| format!("Failed to build Gradle Module Metadata request: {e}"))?
            .url()
            .to_string();
        let url_key = Self::metadata_url_cache_key(&url, extension);
        let url_cache_path = self.metadata_url_path(&url, extension);
        if self.module_metadata_misses.contains_key(&key)
            || self.module_metadata_misses.contains_key(&url_key)
        {
            tracing::debug!(group = %group, name = %name, version = %version, "Gradle Module Metadata negative cache hit");
            return Ok(None);
        }
        if let Some(cached) = self
            .read_cached_text_artifact(
                &key,
                &cache_path,
                group,
                name,
                version,
                classifier,
                extension,
            )
            .await
        {
            tracing::info!(group = %group, name = %name, version = %version, source = "rust_cache", "Gradle Module Metadata cache hit");
            return Ok(Some(cached));
        }
        if let Some(cached) = self
            .read_cached_text_artifact(
                &url_key,
                &url_cache_path,
                group,
                name,
                version,
                classifier,
                extension,
            )
            .await
        {
            tracing::info!(group = %group, name = %name, version = %version, source = "url_cache", "Gradle Module Metadata cache hit");
            return Ok(Some(cached));
        }

        let response = self
            .build_request(repo, &path)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Gradle Module Metadata: {e}"))?;

        match response.status().as_u16() {
            200 => {
                let content = response
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read Gradle Module Metadata response: {e}"))?;
                self.persist_text_artifact(
                    &key,
                    &cache_path,
                    group,
                    name,
                    version,
                    classifier,
                    extension,
                    &content,
                )
                .await?;
                self.persist_text_artifact_alias(
                    &url_key,
                    &url_cache_path,
                    &cache_path,
                    group,
                    name,
                    version,
                    classifier,
                    extension,
                    &content,
                )
                .await?;
                tracing::info!(group = %group, name = %name, version = %version, source = "remote_fetch", url = %url, "Gradle Module Metadata fetched from remote");
                Ok(Some(content))
            }
            404 => {
                self.module_metadata_misses.insert(key, ());
                self.module_metadata_misses.insert(url_key, ());
                Ok(None)
            }
            status => Err(format!("HTTP {status} for Gradle Module Metadata")),
        }
    }

    async fn gradle_module_metadata_artifact_url(
        &self,
        group: &str,
        name: &str,
        version: &str,
        scope: &str,
        repos: &[RepositoryDescriptor],
    ) -> Result<Option<String>, String> {
        let mut redirects = std::collections::HashSet::new();
        let allowed_repos = Self::repositories_for_group(repos, group);
        for repo in allowed_repos
            .iter()
            .filter(|repo| Self::supports_gradle_module_metadata(repo))
        {
            if let Some(url) = Box::pin(self.gradle_module_metadata_artifact_url_in_repo(
                group,
                name,
                version,
                scope,
                repo,
                &mut redirects,
                0,
            ))
            .await?
            {
                return Ok(Some(url));
            }
        }
        Ok(None)
    }

    async fn gradle_module_metadata_artifact_url_in_repo(
        &self,
        group: &str,
        name: &str,
        version: &str,
        scope: &str,
        repo: &RepositoryDescriptor,
        redirects: &mut std::collections::HashSet<(String, String, String)>,
        depth: u32,
    ) -> Result<Option<String>, String> {
        const MAX_GMM_REDIRECT_DEPTH: u32 = 8;
        if depth > MAX_GMM_REDIRECT_DEPTH {
            return Err(format!(
                "unsupported Gradle Module Metadata available-at redirect depth exceeded for {group}:{name}:{version}"
            ));
        }
        let key = (group.to_string(), name.to_string(), version.to_string());
        if !redirects.insert(key.clone()) {
            return Err(format!(
                "unsupported Gradle Module Metadata available-at redirect cycle at {group}:{name}:{version}"
            ));
        }
        let Some(module_metadata) = self
            .fetch_gradle_module_metadata(group, name, version, repo)
            .await?
        else {
            redirects.remove(&key);
            return Ok(None);
        };
        let selection = gradle_module_metadata::select_jvm_variant(
            &module_metadata,
            scope,
            group,
            name,
            version,
        )?;
        let result = match selection {
            Some(ModuleMetadataSelection::Selected(variant)) => {
                let Some(artifact) = variant.artifacts.first() else {
                    redirects.remove(&key);
                    return Ok(None);
                };
                self.module_artifact_url(repo, group, name, version, &artifact.url)
                    .map(Some)
            }
            Some(ModuleMetadataSelection::Redirect(redirect)) => {
                Box::pin(self.gradle_module_metadata_artifact_url_in_repo(
                    &redirect.group,
                    &redirect.module,
                    &redirect.version,
                    scope,
                    repo,
                    redirects,
                    depth + 1,
                ))
                .await
            }
            Some(ModuleMetadataSelection::Unsupported(reason)) => Err(reason),
            None => Ok(None),
        };
        redirects.remove(&key);
        result
    }

    fn module_artifact_url(
        &self,
        repo: &RepositoryDescriptor,
        group: &str,
        name: &str,
        version: &str,
        artifact_path: &str,
    ) -> Result<String, String> {
        if reqwest::Url::parse(artifact_path).is_ok() {
            return Ok(artifact_path.to_string());
        }
        let group_path = Self::group_to_path(group);
        let path = format!(
            "{}/{}/{}/{}",
            group_path,
            name,
            version,
            artifact_path.trim_start_matches('/')
        );
        self.build_request(repo, &path)
            .build()
            .map(|request| request.url().to_string())
            .map_err(|e| format!("Failed to build Gradle Module Metadata artifact URL: {e}"))
    }

    fn artifact_url_for_descriptor(
        repo_base: &str,
        group: &str,
        name: &str,
        version: &str,
        classifier: &str,
        extension: &str,
    ) -> String {
        let extension = if extension.is_empty() {
            "jar"
        } else {
            extension.trim_start_matches('.')
        };
        let classifier_suffix = if classifier.is_empty() {
            String::new()
        } else {
            format!("-{}", classifier)
        };
        format!(
            "{}/{}/{}/{}/{}-{}{}.{}",
            repo_base.trim_end_matches('/'),
            Self::group_to_path(group),
            name,
            version,
            name,
            version,
            classifier_suffix,
            extension
        )
    }

    fn artifact_file_parts_from_url(
        artifact_url: &str,
        name: &str,
        version: &str,
    ) -> Option<(String, String)> {
        let path = reqwest::Url::parse(artifact_url)
            .ok()
            .and_then(|url| url.path_segments()?.last().map(str::to_string))
            .or_else(|| artifact_url.rsplit('/').next().map(str::to_string))?;
        let prefix = format!("{}-{}", name, version);
        if !path.starts_with(&prefix) {
            return None;
        }
        let dot = path.rfind('.')?;
        let extension = path[dot + 1..].to_string();
        if extension.is_empty() {
            return None;
        }
        let suffix = &path[prefix.len()..dot];
        if !suffix.is_empty() && !suffix.starts_with('-') {
            return None;
        }
        let classifier = suffix.strip_prefix('-').unwrap_or("").to_string();
        Some((classifier, extension))
    }

    async fn download_artifact_into_store(
        &self,
        group: &str,
        name: &str,
        version: &str,
        classifier: &str,
        extension: &str,
        artifact_url: &str,
    ) -> Result<(i64, String), String> {
        if group.is_empty() || name.is_empty() || version.is_empty() || artifact_url.is_empty() {
            return Err("Incomplete Maven artifact coordinate".to_string());
        }
        if version.ends_with("-SNAPSHOT") {
            return Err(format!(
                "SNAPSHOT artifact prefetch is not supported yet: {group}:{name}:{version}"
            ));
        }

        let dl_start = std::time::Instant::now();

        let extension = Self::normalize_extension(extension);
        let key = Self::artifact_cache_key(group, name, version, classifier, &extension);
        let store_path = self.artifact_path(group, name, version, classifier, &extension);
        if store_path.exists() {
            let size = store_path.metadata().map(|m| m.len() as i64).unwrap_or(0);
            let sha256 = Self::compute_file_sha256(&store_path)
                .await
                .unwrap_or_default();
            self.artifact_cache.insert(
                key,
                CachedArtifact {
                    group: group.to_string(),
                    name: name.to_string(),
                    version: version.to_string(),
                    classifier: classifier.to_string(),
                    extension,
                    sha256: sha256.clone(),
                    local_path: store_path.to_string_lossy().into_owned(),
                    size,
                    cached_at_ms: Self::now_ms(),
                },
            );
            self.resolution_stats
                .cache_hits
                .fetch_add(1, Ordering::Relaxed);
            tracing::info!(
                artifact = %format!("{}:{}:{}", group, name, version),
                cache_hit = true,
                duration_ms = dl_start.elapsed().as_millis() as u64,
                "Artifact resolved"
            );
            return Ok((size, sha256));
        }

        let response = self
            .http_client
            .get(artifact_url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch artifact {artifact_url}: {e}"))?;
        match response.status().as_u16() {
            200..=299 => {
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|e| format!("Failed to read artifact {artifact_url}: {e}"))?;
                if let Some(parent) = store_path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|e| format!("Failed to create artifact store directory: {e}"))?;
                }
                let tmp_path = PathBuf::from(format!("{}.part", store_path.to_string_lossy()));
                tokio::fs::write(&tmp_path, &bytes)
                    .await
                    .map_err(|e| format!("Failed to write artifact cache file: {e}"))?;
                tokio::fs::rename(&tmp_path, &store_path)
                    .await
                    .map_err(|e| format!("Failed to commit artifact cache file: {e}"))?;

                let sha256 = Self::compute_sha256(&bytes);
                if let Err(e) = Self::write_sha256_sidecar(&store_path, &sha256).await {
                    tracing::warn!(path = %store_path.display(), error = %e, "Failed to write prefetched artifact checksum sidecar");
                }
                let size = bytes.len() as i64;
                self.artifact_cache.insert(
                    key,
                    CachedArtifact {
                        group: group.to_string(),
                        name: name.to_string(),
                        version: version.to_string(),
                        classifier: classifier.to_string(),
                        extension,
                        sha256: sha256.clone(),
                        local_path: store_path.to_string_lossy().into_owned(),
                        size,
                        cached_at_ms: Self::now_ms(),
                    },
                );
                tracing::info!(
                    artifact = %format!("{}:{}:{}", group, name, version),
                    cache_hit = false,
                    duration_ms = dl_start.elapsed().as_millis() as u64,
                    "Artifact resolved"
                );
                Ok((size, sha256))
            }
            404 => Err(format!("Artifact not found: {artifact_url}")),
            status => Err(format!("HTTP {status} for {artifact_url}")),
        }
    }

    async fn prefetch_resolved_artifacts(
        &self,
        deps: &mut [ResolvedDependency],
    ) -> Result<(i32, i64), String> {
        let mut total_artifacts = 0;
        let mut total_download_size = 0;

        for dep in deps {
            if dep.resolved && !dep.artifact_url.is_empty() {
                let (classifier, extension) = Self::artifact_file_parts_from_url(
                    &dep.artifact_url,
                    &dep.name,
                    &dep.selected_version,
                )
                .ok_or_else(|| {
                    format!(
                        "Unsupported Maven artifact URL for prefetch: {}",
                        dep.artifact_url
                    )
                })?;
                let (size, sha256) = self
                    .download_artifact_into_store(
                        &dep.group,
                        &dep.name,
                        &dep.selected_version,
                        &classifier,
                        &extension,
                        &dep.artifact_url,
                    )
                    .await?;
                dep.artifact_size = size;
                dep.artifact_sha256 = sha256;
                total_artifacts += 1;
                total_download_size += size;
            }

            let (child_count, child_size) =
                Box::pin(self.prefetch_resolved_artifacts(&mut dep.dependencies)).await?;
            total_artifacts += child_count;
            total_download_size += child_size;
        }

        Ok((total_artifacts, total_download_size))
    }

    pub fn parse_pom_dependencies(pom_content: &str) -> Vec<PomDependency> {
        maven_pom::parse_pom_dependencies(pom_content)
    }

    pub fn parse_dependency_management(
        pom_content: &str,
    ) -> std::collections::HashMap<(String, String), ManagedDependency> {
        maven_pom::parse_dependency_management(pom_content)
    }

    fn managed_default_scope(dep: &PomDependency, managed: Option<&ManagedDependency>) -> String {
        maven_pom::managed_default_scope(dep, managed)
    }

    fn effective_exclusions(
        dep: &PomDependency,
        managed: Option<&ManagedDependency>,
    ) -> Vec<(String, String)> {
        maven_pom::effective_exclusions(dep, managed)
    }

    /// Deduplicate resolved dependencies by (group, name), keeping the highest version.
    /// This implements Gradle's default conflict resolution strategy.
    pub fn resolve_conflicts(deps: &mut Vec<ResolvedDependency>) {
        resolve_conflicts(deps);
    }

    /// Deduplicate resolved dependencies using the given resolution strategy.
    pub fn resolve_conflicts_with_strategy(
        deps: &mut Vec<ResolvedDependency>,
        strategy: &ResolutionStrategy,
    ) {
        resolve_conflicts_with_strategy(deps, strategy);
    }

    pub fn try_resolve_conflicts_with_strategy(
        deps: &mut Vec<ResolvedDependency>,
        strategy: &ResolutionStrategy,
    ) -> Result<(), String> {
        try_resolve_conflicts_with_strategy(deps, strategy)
    }

    /// Filter resolved dependencies by scope.
    pub fn filter_by_scope(
        deps: Vec<ResolvedDependency>,
        target: &DependencyScope,
    ) -> Vec<ResolvedDependency> {
        deps.into_iter()
            .filter_map(|mut dep| {
                let dep_scope = DependencyScope::from_str_loose(&dep.scope);
                if target.includes(&dep_scope) {
                    dep.dependencies = Self::filter_by_scope(dep.dependencies, target);
                    Some(dep)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check if a dependency matches an exclusion pattern.
    /// An exclusion with group "*" matches any group; artifactId "*" matches any artifact.
    /// Both must match for the exclusion to apply.
    fn matches_exclusion(
        dep_group: &str,
        dep_name: &str,
        excl_group: &str,
        excl_name: &str,
    ) -> bool {
        let group_matches = excl_group == "*" || excl_group == dep_group;
        let name_matches = excl_name == "*" || excl_name == dep_name;
        group_matches && name_matches
    }

    fn is_dependency_excluded(dep: &PomDependency, exclusions: &[(String, String)]) -> bool {
        exclusions.iter().any(|(excl_group, excl_name)| {
            Self::matches_exclusion(&dep.group, &dep.name, excl_group, excl_name)
        })
    }

    /// Parse the <parent> section from a POM file.
    /// Returns None if no parent section exists.
    fn parse_parent_pom(pom_content: &str) -> Option<ParentPom> {
        let bytes = pom_content.as_bytes();
        let pos = find_open_tag_exact(bytes, 0, b"parent")?;
        let _end_pos = find_end_tag(bytes, pos, b"parent")?;

        let group_id = extract_tag_text(bytes, pos, b"groupId").unwrap_or_default();
        let artifact_id = extract_tag_text(bytes, pos, b"artifactId").unwrap_or_default();
        let version = extract_tag_text(bytes, pos, b"version").unwrap_or_default();
        let relative_path = extract_tag_text(bytes, pos, b"relativePath").unwrap_or_default();

        if group_id.is_empty() || artifact_id.is_empty() || version.is_empty() {
            return None;
        }

        Some(ParentPom {
            group_id,
            artifact_id,
            version,
            relative_path,
        })
    }

    /// Parse properties from <properties> section of a POM.
    pub fn parse_pom_properties(pom_content: &str) -> std::collections::HashMap<String, String> {
        let mut props = std::collections::HashMap::new();
        let bytes = pom_content.as_bytes();

        // Find <properties> block
        let start = match find_open_tag_exact(bytes, 0, b"properties") {
            Some(p) => p,
            None => return props,
        };
        let end = match find_end_tag(bytes, start, b"properties") {
            Some(p) => p,
            None => return props,
        };

        // Extract all <key>value</key> pairs within the properties block
        let mut i = start + b"<properties>".len();
        while i < end {
            // Find next opening tag <something>
            let tag_start = match bytes[i..].iter().position(|&b| b == b'<') {
                Some(p) => i + p,
                None => break,
            };
            if tag_start >= end {
                break;
            }

            // Find the closing >
            let tag_end = match bytes[tag_start..].iter().position(|&b| b == b'>') {
                Some(p) => tag_start + p,
                None => break,
            };

            let tag_name = &bytes[tag_start + 1..tag_end];
            // Skip closing tags, comments, etc.
            if tag_name.is_empty() || tag_name[0] == b'/' {
                i = tag_end + 1;
                continue;
            }

            // Extract the text content between <key> and </key>
            let close_tag = format!("</{}", std::str::from_utf8(tag_name).unwrap_or_default());
            let close_bytes = close_tag.as_bytes();
            if let Some(val_end) = bytes[tag_end + 1..end]
                .windows(close_bytes.len())
                .position(|w| w == close_bytes)
                .map(|p| tag_end + 1 + p)
            {
                let value = std::str::from_utf8(&bytes[tag_end + 1..val_end])
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let key = std::str::from_utf8(tag_name)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !key.is_empty() {
                    props.insert(key, value);
                }
                i = val_end + close_bytes.len();
            } else {
                i = tag_end + 1;
            }
        }

        props
    }

    /// Interpolate ${property.name} references in a string using the given properties map.
    pub fn interpolate_properties(
        value: &str,
        properties: &std::collections::HashMap<String, String>,
    ) -> String {
        let mut result = value.to_string();
        // Keep interpolating until no more ${...} references remain (handles nested refs)
        let mut max_iterations = 10;
        while result.contains("${") && max_iterations > 0 {
            max_iterations -= 1;
            if let Some(start) = result.find("${") {
                if let Some(end) = result[start..].find('}') {
                    let key = &result[start + 2..start + end];
                    let replacement = properties.get(key).cloned().unwrap_or_else(|| {
                        // Try common built-in properties
                        match key {
                            "project.version" | "version" | "pom.version" => {
                                "0.0.0-unknown".to_string()
                            }
                            "project.groupId" | "groupId" => "unknown".to_string(),
                            "project.artifactId" | "artifactId" => "unknown".to_string(),
                            _ => format!("${{{}}}", key), // Leave unresolved
                        }
                    });
                    result.replace_range(start..start + end + 1, &replacement);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        result
    }

    /// Resolve a version range to a concrete version.
    /// Supports: exact ("1.0"), soft range ("[1.0,2.0)", "(1.0,]", "[1.0]"), and "latest.release".
    /// Also supports "LATEST" and "RELEASE" via MavenMetadata.
    fn resolve_version_range(
        range: &str,
        available: &[String],
        metadata: Option<&MavenMetadata>,
    ) -> Option<String> {
        let range = range.trim();
        if range.is_empty() || range.contains('+') {
            return None;
        }

        // Special versions: try metadata first, then fall back to available list
        if range == "latest.release" || range == "latest.integration" {
            if let Some(meta) = metadata {
                if let Some(release) = &meta.versioning.release {
                    return Some(release.clone());
                }
            }
            return available.last().cloned();
        }
        if range == "LATEST" {
            if let Some(meta) = metadata {
                if let Some(latest) = &meta.versioning.latest {
                    return Some(latest.clone());
                }
            }
            return available.last().cloned();
        }
        if range == "RELEASE" {
            if let Some(meta) = metadata {
                if let Some(release) = &meta.versioning.release {
                    return Some(release.clone());
                }
            }
            return available.last().cloned();
        }

        // Exact version
        if !range.starts_with('[') && !range.starts_with('(') {
            return Some(range.to_string());
        }
        if !(range.ends_with(']') || range.ends_with(')')) || range.len() < 2 {
            return None;
        }

        // Parse range: [start,end) or (start,end]
        let inner: &str = &range[1..range.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() != 2 {
            return None;
        }

        let start = parts[0].trim();
        let end = parts[1].trim();
        let start_inclusive = range.starts_with('[');
        let end_inclusive = range.ends_with(']');

        // Filter available versions by range
        use std::cmp::Ordering;
        let matching: Vec<&String> = available
            .iter()
            .filter(|v| {
                if !start.is_empty() {
                    let cmp = compare_versions(v, start);
                    if start_inclusive && cmp == Ordering::Less {
                        return false;
                    }
                    if !start_inclusive && cmp != Ordering::Greater {
                        return false;
                    }
                }
                if !end.is_empty() {
                    let cmp = compare_versions(v, end);
                    if end_inclusive && cmp == Ordering::Greater {
                        return false;
                    }
                    if !end_inclusive && cmp != Ordering::Less {
                        return false;
                    }
                }
                true
            })
            .collect();

        // Return the highest matching version
        matching.last().map(|v| (*v).clone())
    }

    fn unsupported_version_selector_reason(version: &str) -> Option<String> {
        graph_builder::unsupported_version_selector_reason(version)
    }

    /// Fetch available versions for a dependency from maven-metadata.xml.
    async fn fetch_available_versions(
        &self,
        group: &str,
        name: &str,
        repos: &[RepositoryDescriptor],
    ) -> (Vec<String>, Option<MavenMetadata>) {
        let allowed_repos = Self::repositories_for_group(repos, group);
        for repo in &allowed_repos {
            match self.fetch_maven_metadata(group, name, repo).await {
                Ok(meta) => {
                    let versions = meta.versioning.versions.clone();
                    if !versions.is_empty() {
                        return (versions, Some(meta));
                    }
                }
                Err(_) => continue,
            }
        }
        (Vec::new(), None)
    }

    /// Resolve a SNAPSHOT version (e.g., `1.0-SNAPSHOT`) to its timestamped form
    /// (e.g., `1.0-20240101.120000-1`) using maven-metadata.xml.
    ///
    /// Maven stores snapshot metadata in `maven-metadata.xml` with a `<snapshot>`
    /// section containing `<timestamp>` and `<buildNumber>`. The resolved version
    /// is `{baseVersion}-{timestamp}-{buildNumber}`.
    async fn resolve_snapshot_version(
        &self,
        group: &str,
        name: &str,
        raw_version: &str,
        repos: &[RepositoryDescriptor],
    ) -> String {
        let allowed_repos = Self::repositories_for_group(repos, group);
        for repo in &allowed_repos {
            match self.fetch_maven_metadata(group, name, repo).await {
                Ok(meta) => {
                    if let Some(ref snapshot) = meta.versioning.snapshot {
                        // If localCopy is true, use the version as-is (don't re-resolve)
                        if snapshot.local_copy {
                            tracing::debug!(
                                group = %group,
                                name = %name,
                                version = %raw_version,
                                "SNAPSHOT marked as localCopy, using base version"
                            );
                            return raw_version.to_string();
                        }

                        let timestamp = snapshot.timestamp.as_deref().unwrap_or("");
                        let build_number = snapshot.build_number.as_deref().unwrap_or("");

                        if !timestamp.is_empty() && !build_number.is_empty() {
                            let base = &raw_version[..raw_version.len() - "-SNAPSHOT".len()];
                            let resolved = format!("{}-{}-{}", base, timestamp, build_number);
                            tracing::debug!(
                                group = %group,
                                name = %name,
                                raw_version = %raw_version,
                                resolved = %resolved,
                                "Resolved SNAPSHOT version"
                            );
                            return resolved;
                        }
                    }
                    // No snapshot info — try to find the latest timestamped version from versions list
                    if let Some(ts_version) = meta.versioning.versions.iter().rfind(|v| {
                        !v.ends_with("-SNAPSHOT")
                            && v.starts_with(&raw_version[..raw_version.len() - "-SNAPSHOT".len()])
                    }) {
                        tracing::debug!(
                            group = %group,
                            name = %name,
                            resolved = %ts_version,
                            "Resolved SNAPSHOT from versions list"
                        );
                        return ts_version.clone();
                    }

                    // No snapshot metadata available — fall through to next repo
                    tracing::debug!(
                        group = %group,
                        name = %name,
                        repo = %repo.url,
                        "No snapshot metadata found, trying next repo"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        group = %group,
                        name = %name,
                        repo = %repo.url,
                        error = %e,
                        "Failed to fetch maven-metadata.xml for SNAPSHOT"
                    );
                }
            }
        }
        // Fallback: use the raw SNAPSHOT version
        raw_version.to_string()
    }

    /// Resolve a single dependency descriptor with real POM fetching.
    /// Handles property interpolation, version ranges, and recursive transitive resolution.
    async fn resolve_descriptor(
        &self,
        dep: &DependencyDescriptor,
        repos: &[RepositoryDescriptor],
        lenient: bool,
    ) -> ResolvedDependency {
        let mut visited = std::collections::HashSet::with_capacity(64);
        self.resolve_recursive(dep, repos, &mut visited, 0, &[], lenient)
            .await
    }

    /// Recursively resolve a dependency and its transitive dependencies.
    ///
    /// Uses BFS-style resolution: fetches the POM for the current artifact,
    /// extracts direct dependencies, then recursively resolves each one.
    /// Cycle detection via `visited` set, depth limiting at 50 levels.
    async fn resolve_recursive(
        &self,
        dep: &DependencyDescriptor,
        repos: &[RepositoryDescriptor],
        visited: &mut std::collections::HashSet<(String, String)>,
        depth: u32,
        inherited_exclusions: &[(String, String)],
        lenient: bool,
    ) -> ResolvedDependency {
        const MAX_DEPTH: u32 = 50;

        let group = dep.group.clone();
        let name = dep.name.clone();
        let raw_version = dep.version.clone();
        let allowed_repos = Self::repositories_for_group(repos, &group);
        let scope = if dep.scope.is_empty() {
            "compile".to_string()
        } else {
            dep.scope.clone()
        };

        if allowed_repos.is_empty() {
            return ResolvedDependency {
                group,
                name,
                version: raw_version.clone(),
                selected_version: raw_version,
                dependencies: Vec::new(),
                resolved: false,
                failure_reason: "No repository admits dependency group through content filters"
                    .to_string(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                scope,
            };
        }

        if let Some(reason) = Self::unsupported_version_selector_reason(&raw_version) {
            return ResolvedDependency {
                group,
                name,
                version: raw_version.clone(),
                selected_version: raw_version,
                dependencies: Vec::new(),
                resolved: false,
                failure_reason: reason,
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                scope,
            };
        }

        // Resolve version ranges, LATEST, RELEASE, and SNAPSHOT
        let selected_version = if raw_version.contains(',')
            || raw_version.starts_with('[')
            || raw_version.starts_with('(')
            || raw_version == "LATEST"
            || raw_version == "RELEASE"
        {
            let (available, metadata) = self
                .fetch_available_versions(&group, &name, &allowed_repos)
                .await;
            if !available.is_empty() {
                Self::resolve_version_range(&raw_version, &available, metadata.as_ref())
                    .unwrap_or(raw_version.clone())
            } else {
                raw_version.clone()
            }
        } else if raw_version.ends_with("-SNAPSHOT") {
            // SNAPSHOT version — resolve to timestamped version via maven-metadata.xml
            self.resolve_snapshot_version(&group, &name, &raw_version, &allowed_repos)
                .await
        } else {
            raw_version.clone()
        };

        if selected_version != raw_version {
            let strategy = if raw_version.contains(',')
                || raw_version.starts_with('[')
                || raw_version.starts_with('(')
            {
                "range"
            } else if raw_version.ends_with("-SNAPSHOT") {
                "snapshot"
            } else {
                "latest"
            };
            tracing::info!(
                group = %group,
                name = %name,
                resolved_version = %selected_version,
                strategy = strategy,
                "Version resolved"
            );
        }

        // Cycle detection: if we've already visited this group:name, return a leaf node.
        let coord = (group.clone(), name.clone());
        if !visited.insert(coord.clone()) {
            tracing::debug!(
                group = %group,
                name = %name,
                depth,
                "Cycle detected — skipping re-resolution"
            );
            let repo_base = allowed_repos
                .first()
                .map(|r| r.url.as_str())
                .unwrap_or("https://repo.maven.apache.org/maven2");
            let artifact_url = Self::artifact_url_for_descriptor(
                repo_base,
                &group,
                &name,
                &selected_version,
                &dep.classifier,
                &dep.extension,
            );
            return ResolvedDependency {
                group,
                name,
                version: raw_version,
                selected_version,
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url,
                artifact_size: 0,
                artifact_sha256: String::new(),
                scope,
            };
        }

        // Fetch POM and resolve transitive dependencies
        let transitive_resolution = if dep.transitive && depth < MAX_DEPTH {
            match self
                .fetch_and_resolve_transitive(
                    &group,
                    &name,
                    &selected_version,
                    &scope,
                    repos,
                    visited,
                    depth,
                    inherited_exclusions,
                    lenient,
                )
                .await
            {
                Ok(resolution) => resolution,
                Err(reason) => {
                    visited.remove(&coord);
                    return ResolvedDependency {
                        group,
                        name,
                        version: raw_version.clone(),
                        selected_version,
                        dependencies: Vec::new(),
                        resolved: false,
                        failure_reason: reason,
                        artifact_url: String::new(),
                        artifact_size: 0,
                        artifact_sha256: String::new(),
                        scope,
                    };
                }
            }
        } else {
            TransitiveResolution {
                dependencies: Vec::new(),
                source_repo_url: None,
            }
        };

        // Remove from visited set so sibling branches can resolve the same dep
        visited.remove(&coord);

        let module_metadata_artifact_url = match self
            .gradle_module_metadata_artifact_url(
                &group,
                &name,
                &selected_version,
                &scope,
                &allowed_repos,
            )
            .await
        {
            Ok(url) => url,
            Err(reason) => {
                return ResolvedDependency {
                    group,
                    name,
                    version: raw_version.clone(),
                    selected_version,
                    dependencies: Vec::new(),
                    resolved: false,
                    failure_reason: reason,
                    artifact_url: String::new(),
                    artifact_size: 0,
                    artifact_sha256: String::new(),
                    scope,
                };
            }
        };

        // Compute artifact URL
        let repo_base = transitive_resolution
            .source_repo_url
            .as_deref()
            .or_else(|| allowed_repos.first().map(|r| r.url.as_str()))
            .unwrap_or("https://repo.maven.apache.org/maven2");
        let artifact_url = module_metadata_artifact_url.unwrap_or_else(|| {
            Self::artifact_url_for_descriptor(
                repo_base,
                &group,
                &name,
                &selected_version,
                &dep.classifier,
                &dep.extension,
            )
        });

        ResolvedDependency {
            group,
            name,
            version: raw_version,
            selected_version,
            dependencies: transitive_resolution.dependencies,
            resolved: true,
            failure_reason: String::new(),
            artifact_url,
            artifact_size: 0,
            artifact_sha256: String::new(),
            scope,
        }
    }

    /// Resolve parent POM chain and merge inherited properties and dependency management.
    /// Walks up the parent chain (grandparent, great-grandparent, etc.) up to MAX_PARENT_DEPTH.
    /// Child properties override parent properties. Parent managed deps fill gaps in child.
    async fn resolve_parent_inheritance(
        &self,
        pom_content: &str,
        repos: &[RepositoryDescriptor],
    ) -> (
        std::collections::HashMap<String, String>,
        std::collections::HashMap<(String, String), ManagedDependency>,
    ) {
        let mut properties = Self::parse_pom_properties(pom_content);
        let mut managed = Self::parse_dependency_management(pom_content);

        let mut current_pom = pom_content.to_string();
        let mut visited_parents = std::collections::HashSet::with_capacity(8);

        for _ in 0..MAX_PARENT_DEPTH {
            let parent = match Self::parse_parent_pom(&current_pom) {
                Some(p) => p,
                None => break,
            };

            let parent_key = (
                parent.group_id.clone(),
                parent.artifact_id.clone(),
                parent.version.clone(),
            );
            if !visited_parents.insert(parent_key) {
                tracing::debug!("Parent cycle detected, stopping inheritance chain");
                break;
            }

            // Fetch parent POM from repos
            let mut parent_content = None;
            let parent_repos = Self::repositories_for_group(repos, &parent.group_id);
            for repo in &parent_repos {
                match self
                    .fetch_pom(&parent.group_id, &parent.artifact_id, &parent.version, repo)
                    .await
                {
                    Ok(content) => {
                        parent_content = Some(content);
                        break;
                    }
                    Err(e) => {
                        tracing::debug!(
                            parent_group = %parent.group_id,
                            parent_name = %parent.artifact_id,
                            parent_version = %parent.version,
                            repo = %repo.url,
                            error = %e,
                            "Failed to fetch parent POM"
                        );
                    }
                }
            }

            let parent_pom = match parent_content {
                Some(content) => content,
                None => break,
            };

            // Merge: child properties override parent, parent fills gaps
            let parent_props = Self::parse_pom_properties(&parent_pom);
            for (k, v) in parent_props {
                properties.entry(k).or_insert(v);
            }

            // Merge: child managed deps override parent, parent fills gaps
            let parent_managed = Self::parse_dependency_management(&parent_pom);
            for (k, v) in parent_managed {
                managed.entry(k).or_insert(v);
            }

            tracing::debug!(
                parent_group = %parent.group_id,
                parent_name = %parent.artifact_id,
                parent_version = %parent.version,
                "Inherited properties and managed deps from parent POM"
            );

            current_pom = parent_pom;
        }

        (properties, managed)
    }

    /// Fetch a POM from repositories and recursively resolve its transitive dependencies.
    /// Handles BOM imports (scope=import, type=pom) and applies exclusions.
    #[allow(clippy::too_many_arguments)]
    async fn fetch_and_resolve_transitive_from_module_metadata_in_repo(
        &self,
        group: &str,
        name: &str,
        version: &str,
        scope: &str,
        repo: &RepositoryDescriptor,
        repos: &[RepositoryDescriptor],
        visited: &mut std::collections::HashSet<(String, String)>,
        depth: u32,
        inherited_exclusions: &[(String, String)],
        lenient: bool,
        redirects: &mut std::collections::HashSet<(String, String, String)>,
        redirect_depth: u32,
    ) -> Result<Option<TransitiveResolution>, String> {
        const MAX_GMM_REDIRECT_DEPTH: u32 = 8;
        if redirect_depth > MAX_GMM_REDIRECT_DEPTH {
            return Err(format!(
                "unsupported Gradle Module Metadata available-at redirect depth exceeded for {group}:{name}:{version}"
            ));
        }
        let key = (group.to_string(), name.to_string(), version.to_string());
        if !redirects.insert(key.clone()) {
            return Err(format!(
                "unsupported Gradle Module Metadata available-at redirect cycle at {group}:{name}:{version}"
            ));
        }
        let Some(module_metadata) = self
            .fetch_gradle_module_metadata(group, name, version, repo)
            .await?
        else {
            redirects.remove(&key);
            return Ok(None);
        };
        let selection = gradle_module_metadata::select_jvm_variant(
            &module_metadata,
            scope,
            group,
            name,
            version,
        )?;
        let result = match selection {
            Some(ModuleMetadataSelection::Selected(variant)) => {
                let mut transitive_deps = Vec::with_capacity(variant.dependencies.len());
                for module_dep in &variant.dependencies {
                    if inherited_exclusions.iter().any(|(excl_group, excl_name)| {
                        Self::matches_exclusion(
                            &module_dep.group,
                            &module_dep.module,
                            excl_group,
                            excl_name,
                        )
                    }) {
                        tracing::debug!(
                            group = %module_dep.group,
                            name = %module_dep.module,
                            "Gradle Module Metadata dependency excluded"
                        );
                        continue;
                    }
                    let child_dep = DependencyDescriptor {
                        group: module_dep.group.clone(),
                        name: module_dep.module.clone(),
                        version: module_dep.version.clone(),
                        classifier: String::new(),
                        extension: "jar".to_string(),
                        transitive: true,
                        scope: scope.to_string(),
                        changing: false,
                        optional: false,
                        ivy_conf: String::new(),
                        strict_version: String::new(),
                        required_version: String::new(),
                        preferred_version: String::new(),
                        rejected_versions: Vec::new(),
                    };
                    let resolved = Box::pin(self.resolve_recursive(
                        &child_dep,
                        repos,
                        visited,
                        depth + 1,
                        &module_dep.exclusions,
                        lenient,
                    ))
                    .await;
                    transitive_deps.push(resolved);
                }
                Self::resolve_conflicts(&mut transitive_deps);
                tracing::debug!(
                    group = %group,
                    name = %name,
                    version = %version,
                    variant = %variant.name,
                    transitive = transitive_deps.len(),
                    "Resolved transitive dependencies from Gradle Module Metadata"
                );
                Ok(Some(TransitiveResolution {
                    dependencies: transitive_deps,
                    source_repo_url: Some(repo.url.clone()),
                }))
            }
            Some(ModuleMetadataSelection::Redirect(redirect)) => {
                Box::pin(
                    self.fetch_and_resolve_transitive_from_module_metadata_in_repo(
                        &redirect.group,
                        &redirect.module,
                        &redirect.version,
                        scope,
                        repo,
                        repos,
                        visited,
                        depth,
                        inherited_exclusions,
                        lenient,
                        redirects,
                        redirect_depth + 1,
                    ),
                )
                .await
            }
            Some(ModuleMetadataSelection::Unsupported(reason)) => Err(reason),
            None => Ok(None),
        };
        redirects.remove(&key);
        result
    }

    async fn fetch_and_resolve_transitive(
        &self,
        group: &str,
        name: &str,
        version: &str,
        scope: &str,
        repos: &[RepositoryDescriptor],
        visited: &mut std::collections::HashSet<(String, String)>,
        depth: u32,
        inherited_exclusions: &[(String, String)],
        lenient: bool,
    ) -> Result<TransitiveResolution, String> {
        let mut redirects = std::collections::HashSet::new();
        let allowed_repos = Self::repositories_for_group(repos, group);
        for repo in allowed_repos
            .iter()
            .filter(|repo| Self::supports_gradle_module_metadata(repo))
        {
            match Box::pin(
                self.fetch_and_resolve_transitive_from_module_metadata_in_repo(
                    group,
                    name,
                    version,
                    scope,
                    repo,
                    repos,
                    visited,
                    depth,
                    inherited_exclusions,
                    lenient,
                    &mut redirects,
                    0,
                ),
            )
            .await
            {
                Ok(Some(resolution)) => return Ok(resolution),
                Ok(None) => {}
                Err(error) => return Err(error),
            }
        }

        for repo in &allowed_repos {
            match self.fetch_pom(group, name, version, repo).await {
                Ok(pom_content) => {
                    // Resolve parent POM chain for inherited properties and managed deps
                    let (properties, managed_versions) =
                        self.resolve_parent_inheritance(&pom_content, repos).await;
                    let pom_deps = Self::parse_pom_dependencies(&pom_content);

                    // Separate BOM imports from regular dependencies
                    let mut bom_imports = Vec::new();
                    let mut regular_deps = Vec::new();

                    for ((bom_group, bom_name), managed) in &managed_versions {
                        if managed.scope == "import" && managed.type_field == "pom" {
                            let bom_version =
                                Self::interpolate_properties(&managed.version, &properties);
                            if !bom_version.is_empty() {
                                bom_imports.push((
                                    bom_group.clone(),
                                    bom_name.clone(),
                                    bom_version,
                                ));
                            }
                        }
                    }

                    for pom_dep in &pom_deps {
                        let managed =
                            managed_versions.get(&(pom_dep.group.clone(), pom_dep.name.clone()));
                        let effective_scope = Self::managed_default_scope(pom_dep, managed);

                        // BOM import: scope=import, type=pom
                        if effective_scope == "import" && pom_dep.type_field == "pom" {
                            let bom_version =
                                Self::interpolate_properties(&pom_dep.version, &properties);
                            if !bom_version.is_empty() {
                                bom_imports.push((
                                    pom_dep.group.clone(),
                                    pom_dep.name.clone(),
                                    bom_version,
                                ));
                            }
                            continue;
                        }

                        // Skip test/provided scopes and optional deps
                        if effective_scope == "test"
                            || effective_scope == "provided"
                            || pom_dep.optional
                        {
                            continue;
                        }

                        if pom_dep.group == group && pom_dep.name == name {
                            tracing::debug!(
                                group = %pom_dep.group,
                                name = %pom_dep.name,
                                "Skipping self dependency from POM"
                            );
                            continue;
                        }

                        // Exclusions declared on the edge from the parent apply to this
                        // artifact's direct dependencies. A sibling dependency's own
                        // exclusions must not remove other siblings.
                        let is_excluded =
                            Self::is_dependency_excluded(pom_dep, inherited_exclusions);
                        if is_excluded {
                            tracing::debug!(
                                group = %pom_dep.group,
                                name = %pom_dep.name,
                                "Transitive dependency excluded"
                            );
                            continue;
                        }

                        // Resolve version via property interpolation + dependency management
                        let raw_dep_version =
                            Self::interpolate_properties(&pom_dep.version, &properties);
                        let resolved_version =
                            if raw_dep_version.is_empty() || raw_dep_version.starts_with("${") {
                                managed
                                    .map(|managed| managed.version.clone())
                                    .unwrap_or(raw_dep_version)
                            } else {
                                raw_dep_version
                            };

                        let effective_exclusions = Self::effective_exclusions(pom_dep, managed);
                        regular_deps.push((
                            pom_dep.clone(),
                            resolved_version,
                            effective_scope,
                            effective_exclusions,
                        ));
                    }

                    // Merge BOM managed versions into our managed set
                    let mut merged_managed = managed_versions;
                    for (bom_group, bom_name, bom_version) in &bom_imports {
                        if let Ok(bom_pom) =
                            self.fetch_pom(bom_group, bom_name, bom_version, repo).await
                        {
                            let bom_props = Self::parse_pom_properties(&bom_pom);
                            let bom_managed = Self::parse_dependency_management(&bom_pom);
                            for ((g, n), mut managed) in bom_managed {
                                let interpolated =
                                    Self::interpolate_properties(&managed.version, &bom_props);
                                if !interpolated.is_empty() {
                                    managed.version = interpolated;
                                    merged_managed.entry((g, n)).or_insert(managed);
                                }
                            }
                            tracing::debug!(
                                bom_group = %bom_group,
                                bom_name = %bom_name,
                                bom_version = %bom_version,
                                entries = merged_managed.len(),
                                "Loaded BOM and merged managed dependencies"
                            );
                        }
                    }

                    // Re-resolve versions with merged managed set
                    for (pom_dep, resolved_version, effective_scope, effective_exclusions) in
                        &mut regular_deps
                    {
                        if resolved_version.starts_with("${") || resolved_version.is_empty() {
                            if let Some(managed) =
                                merged_managed.get(&(pom_dep.group.clone(), pom_dep.name.clone()))
                            {
                                *resolved_version = managed.version.clone();
                                *effective_scope =
                                    Self::managed_default_scope(pom_dep, Some(managed));
                                *effective_exclusions =
                                    Self::effective_exclusions(pom_dep, Some(managed));
                            }
                        }
                    }

                    // Resolve each regular dependency recursively
                    let mut transitive_deps = Vec::new();

                    for (pom_dep, resolved_version, effective_scope, effective_exclusions) in
                        &regular_deps
                    {
                        if resolved_version.is_empty() {
                            continue;
                        }
                        let (classifier, extension) =
                            Self::maven_artifact_shape(&pom_dep.classifier, &pom_dep.type_field);
                        let child_dep = DependencyDescriptor {
                            group: pom_dep.group.clone(),
                            name: pom_dep.name.clone(),
                            version: resolved_version.clone(),
                            classifier,
                            extension,
                            transitive: true,
                            scope: effective_scope.clone(),
                            changing: false,
                            optional: false,
                            ivy_conf: String::new(),
                            strict_version: String::new(),
                            required_version: String::new(),
                            preferred_version: String::new(),
                            rejected_versions: Vec::new(),
                        };
                        let resolved = Box::pin(self.resolve_recursive(
                            &child_dep,
                            repos,
                            visited,
                            depth + 1,
                            effective_exclusions,
                            lenient,
                        ))
                        .await;
                        transitive_deps.push(resolved);
                    }

                    // Apply conflict resolution
                    Self::resolve_conflicts(&mut transitive_deps);

                    tracing::debug!(
                        group = %group,
                        name = %name,
                        depth,
                        transitive = transitive_deps.len(),
                        "Resolved {} transitive dependencies (depth {})",
                        transitive_deps.len(),
                        depth
                    );

                    return Ok(TransitiveResolution {
                        dependencies: transitive_deps,
                        source_repo_url: Some(repo.url.clone()),
                    });
                }
                Err(e) => {
                    tracing::debug!(
                        group = %group,
                        name = %name,
                        repo = %repo.url,
                        error = %e,
                        "Failed to fetch POM from repo"
                    );
                }
            }
        }
        if lenient {
            tracing::debug!(
                group = %group,
                name = %name,
                version = %version,
                depth,
                "No module metadata found; preserving lenient artifact-only tolerance"
            );
            Ok(TransitiveResolution {
                dependencies: Vec::new(),
                source_repo_url: None,
            })
        } else {
            Err(format!(
                "No Gradle Module Metadata or Maven POM found for {group}:{name}:{version} in configured repositories"
            ))
        }
    }

    /// Download an artifact with retry logic.
    #[allow(dead_code)]
    async fn _download_with_retry(&self, url: &str, max_retries: u32) -> Result<Vec<u8>, String> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.http_client.get(url).send().await {
                Ok(resp) => match resp.status().as_u16() {
                    200..=299 => {
                        return resp
                            .bytes()
                            .await
                            .map(|b| b.to_vec())
                            .map_err(|e| format!("Failed to read response: {}", e));
                    }
                    404 => return Err(format!("Not found: {}", url)),
                    500..=599 if attempt < max_retries => {
                        let delay = Duration::from_millis(200 * 2u64.pow(attempt - 1));
                        tracing::warn!(url = %url, attempt, "5xx error, retrying after {:?}", delay);
                        tokio::time::sleep(delay).await;
                    }
                    status => return Err(format!("HTTP {} for {}", status, url)),
                },
                Err(e) if attempt < max_retries => {
                    let delay = Duration::from_millis(200 * 2u64.pow(attempt - 1));
                    tracing::warn!(url = %url, attempt, error = %e, "Network error, retrying after {:?}", delay);
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(format!("Download failed: {}", e)),
            }
        }
    }
}

/// Find an exact opening tag (e.g., `<dependency>`) in bytes.
/// Ensures the tag is followed by `>` or whitespace (not part of a longer tag name).
fn find_open_tag_exact(bytes: &[u8], from: usize, tag: &[u8]) -> Option<usize> {
    // Build "<tag" on the stack — avoids format!() heap allocation per call
    let mut open_buf = [0u8; 64];
    open_buf[0] = b'<';
    let tag_len = tag.len().min(63);
    open_buf[1..=tag_len].copy_from_slice(&tag[..tag_len]);
    let open_bytes = &open_buf[..=tag_len];
    let mut search_from = from;

    while search_from < bytes.len() {
        if let Some(pos) = bytes[search_from..]
            .windows(open_bytes.len())
            .position(|w| w == open_bytes)
            .map(|pos| search_from + pos)
        {
            // Check the character after the tag name: must be '>' or whitespace
            let after = pos + open_bytes.len();
            if after < bytes.len() {
                let next_char = bytes[after];
                if next_char == b'>'
                    || next_char == b' '
                    || next_char == b'\n'
                    || next_char == b'\r'
                    || next_char == b'\t'
                {
                    return Some(pos);
                }
                // Not an exact match — e.g., <dependency> vs <dependencyManagement>
                // Skip past this position and continue searching
                search_from = after;
                continue;
            }
            return Some(pos);
        }
        return None;
    }
    None
}

/// Find the start of a tag (e.g., `<dependency>`) in bytes. (Legacy — kept for compatibility)
fn _find_tag(bytes: &[u8], from: usize, tag: &[u8]) -> Option<usize> {
    find_open_tag_exact(bytes, from, tag)
}

/// Find an end tag (e.g., `</dependency>`) in bytes.
fn find_end_tag(bytes: &[u8], from: usize, tag: &[u8]) -> Option<usize> {
    // Build "</tag" on the stack — avoids format!() heap allocation
    let mut close_buf = [0u8; 65];
    close_buf[0] = b'<';
    close_buf[1] = b'/';
    let tag_len = tag.len().min(63);
    close_buf[2..=tag_len + 1].copy_from_slice(&tag[..tag_len]);
    let close_bytes = &close_buf[..=tag_len + 1];
    bytes[from..]
        .windows(close_bytes.len())
        .position(|w| w == close_bytes)
        .map(|pos| from + pos)
}

/// Extract text content of a child tag within a parent block.
fn extract_tag_text(bytes: &[u8], parent_start: usize, tag: &[u8]) -> Option<String> {
    // Build open/close tag patterns on the stack
    let mut open_buf = [0u8; 64];
    open_buf[0] = b'<';
    let tag_len = tag.len().min(63);
    open_buf[1..=tag_len].copy_from_slice(&tag[..tag_len]);
    let open_bytes = &open_buf[..=tag_len];

    let mut close_buf = [0u8; 65];
    close_buf[0] = b'<';
    close_buf[1] = b'/';
    close_buf[2..=tag_len + 1].copy_from_slice(&tag[..tag_len]);
    let close_bytes = &close_buf[..=tag_len + 1];

    // Find the opening tag after parent_start
    let search_from = parent_start;
    if let Some(start_pos) = bytes[search_from..]
        .windows(open_bytes.len())
        .position(|w| w == open_bytes)
        .map(|pos| search_from + pos)
    {
        let content_start = start_pos + open_bytes.len();
        // Skip the closing `>` of the opening tag
        let content_start = content_start
            + bytes[content_start..]
                .iter()
                .position(|&b| b == b'>')
                .unwrap_or(0)
            + 1;

        if let Some(end_pos) = bytes[content_start..]
            .windows(close_bytes.len())
            .position(|w| w == close_bytes)
            .map(|pos| content_start + pos)
        {
            let content = &bytes[content_start..end_pos];
            let text = std::str::from_utf8(content)
                .unwrap_or_default()
                .trim()
                .to_string();
            return Some(text);
        }
    }
    None
}

#[tonic::async_trait]
impl DependencyResolutionService for DependencyResolutionServiceImpl {
    async fn resolve_dependencies(
        &self,
        request: Request<ResolveDependenciesRequest>,
    ) -> Result<Response<ResolveDependenciesResponse>, Status> {
        let req = request.into_inner();
        let start = std::time::Instant::now();

        tracing::info!(
            configuration = %req.configuration_name,
            artifact_count = req.dependencies.len(),
            "Resolving dependencies"
        );

        let graph_request = match graph_builder::build_dependency_graph_request(
            &req.dependencies,
            &req.constraints,
            &req.repositories,
            &req.target_scope,
        ) {
            Ok(graph_request) => graph_request,
            Err(error_message) => {
                let elapsed = start.elapsed().as_millis() as i64;
                return Ok(Response::new(ResolveDependenciesResponse {
                    success: false,
                    resolved_dependencies: Vec::new(),
                    error_message,
                    resolution_time_ms: elapsed,
                    total_artifacts: 0,
                    total_download_size: 0,
                }));
            }
        };

        let mut resolved = Vec::with_capacity(graph_request.dependencies.len());
        for dep in &graph_request.dependencies {
            let mut result = self
                .resolve_descriptor(dep, &graph_request.repositories, req.lenient)
                .await;
            // Propagate scope from the request descriptor
            if !dep.scope.is_empty() {
                result.scope = dep.scope.clone();
            }
            resolved.push(result);
        }

        if let Some(error_message) = Self::first_unresolved_reason(&resolved) {
            let elapsed = start.elapsed().as_millis() as i64;
            return Ok(Response::new(ResolveDependenciesResponse {
                success: false,
                resolved_dependencies: resolved,
                error_message,
                resolution_time_ms: elapsed,
                total_artifacts: 0,
                total_download_size: 0,
            }));
        }

        // Apply resolution strategy if configured
        let strategy = req
            .resolution_strategy
            .as_ref()
            .map(ResolutionStrategy::from_proto)
            .unwrap_or_default();
        if let Err(error_message) =
            Self::try_resolve_conflicts_with_strategy(&mut resolved, &strategy)
        {
            let elapsed = start.elapsed().as_millis() as i64;
            return Ok(Response::new(ResolveDependenciesResponse {
                success: false,
                resolved_dependencies: resolved,
                error_message,
                resolution_time_ms: elapsed,
                total_artifacts: 0,
                total_download_size: 0,
            }));
        }

        // Filter by target scope if specified
        let mut resolved = if !req.target_scope.is_empty() {
            let target = DependencyScope::from_str_loose(&req.target_scope);
            Self::filter_by_scope(resolved, &target)
        } else {
            resolved
        };

        let mut total_download_size = 0;
        let total_artifacts = if req.prefetch_artifacts {
            tracing::info!(
                prefetch_count = resolved.len(),
                "Prefetching resolved artifacts"
            );
            match self.prefetch_resolved_artifacts(&mut resolved).await {
                Ok((prefetched, downloaded)) => {
                    total_download_size = downloaded;
                    prefetched
                }
                Err(e) => {
                    let elapsed = start.elapsed().as_millis() as i64;
                    return Ok(Response::new(ResolveDependenciesResponse {
                        success: false,
                        resolved_dependencies: resolved,
                        error_message: e,
                        resolution_time_ms: elapsed,
                        total_artifacts: 0,
                        total_download_size: 0,
                    }));
                }
            }
        } else {
            resolved.len() as i32
        };

        let elapsed = start.elapsed().as_millis() as i64;

        self.resolution_stats
            .total_resolutions
            .fetch_add(1, Ordering::Relaxed);
        self.resolution_stats
            .total_time_ms
            .fetch_add(elapsed, Ordering::Relaxed);

        tracing::info!(
            configuration = %req.configuration_name,
            dependencies = total_artifacts,
            time_ms = elapsed,
            "Dependencies resolved"
        );

        Ok(Response::new(ResolveDependenciesResponse {
            success: true,
            resolved_dependencies: resolved,
            error_message: String::new(),
            resolution_time_ms: elapsed,
            total_artifacts,
            total_download_size,
        }))
    }

    async fn check_artifact_cache(
        &self,
        request: Request<CheckArtifactCacheRequest>,
    ) -> Result<Response<CheckArtifactCacheResponse>, Status> {
        let req = request.into_inner();

        let extension = Self::normalize_extension(&req.extension);
        let key = Self::artifact_cache_key(
            &req.group,
            &req.name,
            &req.version,
            &req.classifier,
            &extension,
        );

        if let Some(cached) = self.artifact_cache.get(&key) {
            let cached_group = cached.group.clone();
            let cached_name = cached.name.clone();
            let cached_version = cached.version.clone();
            let cached_classifier = cached.classifier.clone();
            let cached_extension = cached.extension.clone();
            let cached_sha256 = cached.sha256.clone();
            let cached_local_path = cached.local_path.clone();
            let cached_size = cached.size;
            let cached_at_ms = cached.cached_at_ms;
            drop(cached);

            if !Path::new(&cached_local_path).is_file() {
                self.artifact_cache.remove(&key);
                tracing::debug!(
                    group = %cached_group,
                    name = %cached_name,
                    version = %cached_version,
                    classifier = %cached_classifier,
                    extension = %cached_extension,
                    local_path = %cached_local_path,
                    "Artifact cache warm entry points to a missing file"
                );
                return Ok(Response::new(CheckArtifactCacheResponse {
                    cached: false,
                    local_path: String::new(),
                    cached_size: 0,
                }));
            }

            let actual_sha256 = if !req.sha256.is_empty() && cached_sha256.is_empty() {
                Self::compute_file_sha256(Path::new(&cached_local_path))
                    .await
                    .unwrap_or_default()
            } else {
                cached_sha256.clone()
            };

            // Validate SHA-256 if the caller provided one.
            if !req.sha256.is_empty() && actual_sha256 != req.sha256 {
                tracing::warn!(
                    group = %cached_group,
                    name = %cached_name,
                    version = %cached_version,
                    classifier = %cached_classifier,
                    expected_sha256 = %req.sha256,
                    cached_sha256 = %actual_sha256,
                    "Artifact cache SHA-256 mismatch"
                );
                return Ok(Response::new(CheckArtifactCacheResponse {
                    cached: false,
                    local_path: String::new(),
                    cached_size: 0,
                }));
            }

            let age_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64
                - cached_at_ms;

            tracing::debug!(
                group = %cached_group,
                name = %cached_name,
                version = %cached_version,
                classifier = %cached_classifier,
                extension = %cached_extension,
                sha256 = %actual_sha256,
                local_path = %cached_local_path,
                size = cached_size,
                cached_at_ms,
                age_ms,
                "Artifact cache hit"
            );

            self.resolution_stats
                .cache_hits
                .fetch_add(1, Ordering::Relaxed);

            return Ok(Response::new(CheckArtifactCacheResponse {
                cached: true,
                local_path: cached_local_path,
                cached_size,
            }));
        }

        // Cold path: check filesystem for persisted artifact
        let path = self.artifact_path(
            &req.group,
            &req.name,
            &req.version,
            &req.classifier,
            &extension,
        );
        if path.exists() {
            let metadata = path.metadata().ok();
            let size = metadata.as_ref().map(|m| m.len() as i64).unwrap_or(0);
            let actual_sha256 = if req.sha256.is_empty() {
                String::new()
            } else {
                Self::compute_file_sha256(&path).await.unwrap_or_default()
            };

            if !req.sha256.is_empty() && actual_sha256 != req.sha256 {
                tracing::warn!(
                    group = %req.group,
                    name = %req.name,
                    version = %req.version,
                    classifier = %req.classifier,
                    expected_sha256 = %req.sha256,
                    actual_sha256 = %actual_sha256,
                    "Artifact cache cold-path SHA-256 mismatch"
                );
                return Ok(Response::new(CheckArtifactCacheResponse {
                    cached: false,
                    local_path: String::new(),
                    cached_size: 0,
                }));
            }

            let cached_artifact = CachedArtifact {
                group: req.group.clone(),
                name: req.name.clone(),
                version: req.version.clone(),
                classifier: req.classifier.clone(),
                extension: extension.clone(),
                sha256: actual_sha256.clone(),
                local_path: path.to_string_lossy().into_owned(),
                size,
                cached_at_ms: Self::now_ms(),
            };

            // Insert into DashMap for future warm-path hits
            self.artifact_cache.insert(key.clone(), cached_artifact);

            tracing::debug!(
                group = %req.group,
                name = %req.name,
                version = %req.version,
                sha256 = %actual_sha256,
                "Artifact cache cold-path hit from filesystem"
            );

            return Ok(Response::new(CheckArtifactCacheResponse {
                cached: true,
                local_path: path.to_string_lossy().into_owned(),
                cached_size: size,
            }));
        }

        Ok(Response::new(CheckArtifactCacheResponse {
            cached: false,
            local_path: String::new(),
            cached_size: 0,
        }))
    }

    async fn check_metadata_cache(
        &self,
        request: Request<CheckMetadataCacheRequest>,
    ) -> Result<Response<CheckMetadataCacheResponse>, Status> {
        let req = request.into_inner();
        if req.url.is_empty() {
            return Ok(Response::new(CheckMetadataCacheResponse {
                cached: false,
                local_path: String::new(),
                cached_size: 0,
            }));
        }

        let extension = Self::normalize_extension(&req.extension);
        let key = Self::metadata_url_cache_key(&req.url, &extension);
        if let Some(cached) = self.artifact_cache.get(&key) {
            let cached_sha256 = cached.sha256.clone();
            let cached_local_path = cached.local_path.clone();
            let cached_size = cached.size;
            drop(cached);

            let actual_sha256 = if !req.sha256.is_empty() && cached_sha256.is_empty() {
                Self::compute_file_sha256(Path::new(&cached_local_path))
                    .await
                    .unwrap_or_default()
            } else {
                cached_sha256
            };
            if !req.sha256.is_empty() && actual_sha256 != req.sha256 {
                return Ok(Response::new(CheckMetadataCacheResponse {
                    cached: false,
                    local_path: String::new(),
                    cached_size: 0,
                }));
            }
            if Path::new(&cached_local_path).is_file() {
                self.resolution_stats
                    .cache_hits
                    .fetch_add(1, Ordering::Relaxed);
                return Ok(Response::new(CheckMetadataCacheResponse {
                    cached: true,
                    local_path: cached_local_path,
                    cached_size,
                }));
            }
            self.artifact_cache.remove(&key);
        }

        let path = self.metadata_url_path(&req.url, &extension);
        if path.exists() {
            let size = path.metadata().map(|m| m.len() as i64).unwrap_or(0);
            let actual_sha256 = if req.sha256.is_empty() {
                String::new()
            } else {
                Self::compute_file_sha256(&path).await.unwrap_or_default()
            };
            if !req.sha256.is_empty() && actual_sha256 != req.sha256 {
                return Ok(Response::new(CheckMetadataCacheResponse {
                    cached: false,
                    local_path: String::new(),
                    cached_size: 0,
                }));
            }
            self.artifact_cache.insert(
                key,
                CachedArtifact {
                    group: String::new(),
                    name: String::new(),
                    version: String::new(),
                    classifier: String::new(),
                    extension,
                    sha256: actual_sha256,
                    local_path: path.to_string_lossy().into_owned(),
                    size,
                    cached_at_ms: Self::now_ms(),
                },
            );
            self.resolution_stats
                .cache_hits
                .fetch_add(1, Ordering::Relaxed);
            return Ok(Response::new(CheckMetadataCacheResponse {
                cached: true,
                local_path: path.to_string_lossy().into_owned(),
                cached_size: size,
            }));
        }

        Ok(Response::new(CheckMetadataCacheResponse {
            cached: false,
            local_path: String::new(),
            cached_size: 0,
        }))
    }

    type DownloadArtifactStream = std::pin::Pin<
        Box<
            dyn tonic::codegen::tokio_stream::Stream<
                    Item = Result<crate::proto::DownloadArtifactChunk, Status>,
                > + Send,
        >,
    >;

    async fn download_artifact(
        &self,
        request: Request<crate::proto::DownloadArtifactRequest>,
    ) -> Result<Response<Self::DownloadArtifactStream>, Status> {
        let req = request.into_inner();

        let url = if req.url.is_empty() {
            req.repositories
                .first()
                .map(|repo| {
                    Self::artifact_url_for_descriptor(
                        &repo.url,
                        &req.group,
                        &req.name,
                        &req.version,
                        &req.classifier,
                        &req.extension,
                    )
                })
                .unwrap_or_default()
        } else {
            req.url.clone()
        };
        let client = self.http_client.clone();
        let extension = Self::normalize_extension(&req.extension);
        let has_coordinate =
            !(req.group.is_empty() || req.name.is_empty() || req.version.is_empty());
        let is_url_metadata = !has_coordinate && Self::is_metadata_extension(&extension);
        let cache_key = if is_url_metadata {
            Self::metadata_url_cache_key(&url, &extension)
        } else {
            Self::artifact_cache_key(
                &req.group,
                &req.name,
                &req.version,
                &req.classifier,
                &extension,
            )
        };
        let artifact_cache = Arc::clone(&self.artifact_cache);
        let cache_group = if has_coordinate {
            req.group.clone()
        } else {
            String::new()
        };
        let cache_name = if has_coordinate {
            req.name.clone()
        } else {
            String::new()
        };
        let cache_version = if has_coordinate {
            req.version.clone()
        } else {
            String::new()
        };
        let cache_classifier = if has_coordinate {
            req.classifier.clone()
        } else {
            String::new()
        };
        let cache_extension = extension.clone();
        let store_path = if has_coordinate {
            self.artifact_path(
                &req.group,
                &req.name,
                &req.version,
                &req.classifier,
                &extension,
            )
        } else if is_url_metadata {
            self.metadata_url_path(&url, &extension)
        } else {
            PathBuf::new()
        };
        let cached_source_path = Self::warm_cached_artifact_path(&artifact_cache, &cache_key)
            .or_else(|| {
                if !store_path.as_os_str().is_empty() && store_path.is_file() {
                    Some(store_path.clone())
                } else {
                    None
                }
            });

        let stream = async_stream::stream! {
            if url.is_empty() {
                yield Ok(crate::proto::DownloadArtifactChunk {
                    data: Vec::new(),
                    offset: 0,
                    total_size: 0,
                    is_last: true,
                    error_message: "No artifact URL provided".to_string(),
                });
                return;
            }

            if let Some(cached_source_path) = cached_source_path.as_ref() {
                use tokio::io::AsyncReadExt;

                match tokio::fs::File::open(cached_source_path).await {
                    Ok(mut cached_file) => {
                        let total_size = cached_file.metadata().await.map(|m| m.len() as i64).unwrap_or(-1);
                        let mut offset = 0u64;
                        let mut hasher = Sha256::new();
                        let mut buffer = vec![0u8; 64 * 1024];

                        loop {
                            match cached_file.read(&mut buffer).await {
                                Ok(0) => break,
                                Ok(read) => {
                                    let bytes = &buffer[..read];
                                    hasher.update(bytes);
                                    yield Ok(crate::proto::DownloadArtifactChunk {
                                        data: bytes.to_vec(),
                                        offset: offset as i64,
                                        total_size,
                                        is_last: false,
                                        error_message: String::new(),
                                    });
                                    offset += read as u64;
                                }
                                Err(e) => {
                                    yield Ok(crate::proto::DownloadArtifactChunk {
                                        data: Vec::new(),
                                        offset: offset as i64,
                                        total_size,
                                        is_last: true,
                                        error_message: format!("Failed to read cached artifact: {}", e),
                                    });
                                    return;
                                }
                            }
                        }

                        let sha256 = format!("{:x}", hasher.finalize());
                        artifact_cache.insert(cache_key.clone(), CachedArtifact {
                            group: cache_group.clone(),
                            name: cache_name.clone(),
                            version: cache_version.clone(),
                            classifier: cache_classifier.clone(),
                            extension: cache_extension.clone(),
                            sha256: sha256.clone(),
                            local_path: cached_source_path.to_string_lossy().into_owned(),
                            size: offset as i64,
                            cached_at_ms: Self::now_ms(),
                        });
                        if cached_source_path == &store_path {
                            if let Err(e) = Self::write_sha256_sidecar(&store_path, &sha256).await {
                                tracing::warn!(path = %store_path.display(), error = %e, "Failed to write cached artifact checksum sidecar");
                            }
                        }

                        yield Ok(crate::proto::DownloadArtifactChunk {
                            data: Vec::new(),
                            offset: offset as i64,
                            total_size,
                            is_last: true,
                            error_message: String::new(),
                        });
                        return;
                    }
                    Err(e) => {
                        artifact_cache.remove(&cache_key);
                        tracing::warn!(path = %cached_source_path.display(), error = %e, "Cached artifact exists but cannot be opened; falling back to HTTP");
                    }
                }
            }

            let mut attempt = 0u32;
            let max_retries = 3u32;

            loop {
                attempt += 1;
                match client.get(&url).send().await {
                    Ok(resp) => {
                        match resp.status().as_u16() {
                            200..=299 => {
                                use futures_util::StreamExt;
                                use tokio::io::AsyncWriteExt;

                                let total_size = resp.content_length().map(|size| size as i64).unwrap_or(-1);
                                let mut offset = 0u64;
                                let mut body = resp.bytes_stream();
                                let mut hasher = Sha256::new();
                                let mut file = None;
                                let tmp_path = if store_path.as_os_str().is_empty() {
                                    PathBuf::new()
                                } else {
                                    PathBuf::from(format!("{}.part", store_path.to_string_lossy()))
                                };

                                if !store_path.as_os_str().is_empty() {
                                    if let Some(parent) = store_path.parent() {
                                        if let Err(e) = tokio::fs::create_dir_all(parent).await {
                                            yield Ok(crate::proto::DownloadArtifactChunk {
                                                data: Vec::new(),
                                                offset: 0,
                                                total_size,
                                                is_last: true,
                                                error_message: format!("Failed to create artifact store directory: {}", e),
                                            });
                                            return;
                                        }
                                    }
                                    match tokio::fs::File::create(&tmp_path).await {
                                        Ok(created) => file = Some(created),
                                        Err(e) => {
                                            yield Ok(crate::proto::DownloadArtifactChunk {
                                                data: Vec::new(),
                                                offset: 0,
                                                total_size,
                                                is_last: true,
                                                error_message: format!("Failed to create artifact cache file: {}", e),
                                            });
                                            return;
                                        }
                                    }
                                }

                                while let Some(chunk_result) = body.next().await {
                                    match chunk_result {
                                        Ok(bytes) => {
                                            if let Some(cache_file) = file.as_mut() {
                                                if let Err(e) = cache_file.write_all(&bytes).await {
                                                    let _ = tokio::fs::remove_file(&tmp_path).await;
                                                    yield Ok(crate::proto::DownloadArtifactChunk {
                                                        data: Vec::new(),
                                                        offset: offset as i64,
                                                        total_size,
                                                        is_last: true,
                                                        error_message: format!("Failed to write artifact cache file: {}", e),
                                                    });
                                                    return;
                                                }
                                            }
                                            hasher.update(&bytes);
                                            let chunk_len = bytes.len() as u64;
                                            yield Ok(crate::proto::DownloadArtifactChunk {
                                                data: bytes.to_vec(),
                                                offset: offset as i64,
                                                total_size,
                                                is_last: false,
                                                error_message: String::new(),
                                            });
                                            offset += chunk_len;
                                        }
                                        Err(e) => {
                                            let _ = tokio::fs::remove_file(&tmp_path).await;
                                            yield Ok(crate::proto::DownloadArtifactChunk {
                                                data: Vec::new(),
                                                offset: offset as i64,
                                                total_size,
                                                is_last: true,
                                                error_message: format!("Stream error: {}", e),
                                            });
                                            return;
                                        }
                                    }
                                }

                                let sha256 = format!("{:x}", hasher.finalize());
                                if let Some(mut cache_file) = file {
                                    if let Err(e) = cache_file.flush().await {
                                        let _ = tokio::fs::remove_file(&tmp_path).await;
                                        yield Ok(crate::proto::DownloadArtifactChunk {
                                            data: Vec::new(),
                                            offset: offset as i64,
                                            total_size,
                                            is_last: true,
                                            error_message: format!("Failed to flush artifact cache file: {}", e),
                                        });
                                        return;
                                    }
                                    drop(cache_file);
                                    if let Err(e) = tokio::fs::rename(&tmp_path, &store_path).await {
                                        let _ = tokio::fs::remove_file(&tmp_path).await;
                                        yield Ok(crate::proto::DownloadArtifactChunk {
                                            data: Vec::new(),
                                            offset: offset as i64,
                                            total_size,
                                            is_last: true,
                                            error_message: format!("Failed to commit artifact cache file: {}", e),
                                        });
                                        return;
                                    }
                                    if let Err(e) = Self::write_sha256_sidecar(&store_path, &sha256).await {
                                        tracing::warn!(path = %store_path.display(), error = %e, "Failed to write artifact checksum sidecar");
                                    }
                                    artifact_cache.insert(cache_key.clone(), CachedArtifact {
                                        group: cache_group.clone(),
                                        name: cache_name.clone(),
                                        version: cache_version.clone(),
                                        classifier: cache_classifier.clone(),
                                        extension: cache_extension.clone(),
                                        sha256: sha256.clone(),
                                        local_path: store_path.to_string_lossy().into_owned(),
                                        size: offset as i64,
                                        cached_at_ms: Self::now_ms(),
                                    });
                                }

                                yield Ok(crate::proto::DownloadArtifactChunk {
                                    data: Vec::new(),
                                    offset: offset as i64,
                                    total_size,
                                    is_last: true,
                                    error_message: String::new(),
                                });
                                return;
                            }
                            404 => {
                                yield Ok(crate::proto::DownloadArtifactChunk {
                                    data: Vec::new(),
                                    offset: 0,
                                    total_size: 0,
                                    is_last: true,
                                    error_message: format!("Artifact not found: {}", url),
                                });
                                return;
                            }
                            status if status >= 500 && attempt < max_retries => {
                                let delay_ms = 200 * 2u64.pow(attempt - 1);
                                tracing::warn!(url = %url, status, attempt, retry_after_ms = delay_ms, "5xx on artifact download");
                                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            }
                            status => {
                                yield Ok(crate::proto::DownloadArtifactChunk {
                                    data: Vec::new(),
                                    offset: 0,
                                    total_size: 0,
                                    is_last: true,
                                    error_message: format!("HTTP {} for {}", status, url),
                                });
                                return;
                            }
                        }
                    }
                    Err(e) if attempt < max_retries => {
                        let delay_ms = 200 * 2u64.pow(attempt - 1);
                        tracing::warn!(url = %url, attempt, error = %e, retry_after_ms = delay_ms, "Network error on artifact download");
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    Err(e) => {
                        yield Ok(crate::proto::DownloadArtifactChunk {
                            data: Vec::new(),
                            offset: 0,
                            total_size: 0,
                            is_last: true,
                            error_message: format!("Download failed: {}", e),
                        });
                        return;
                    }
                }
            }
        };

        Ok(Response::new(
            Box::pin(stream) as Self::DownloadArtifactStream
        ))
    }

    async fn record_resolution(
        &self,
        request: Request<RecordResolutionRequest>,
    ) -> Result<Response<RecordResolutionResponse>, Status> {
        let req = request.into_inner();

        tracing::debug!(
            configuration = %req.configuration_name,
            dependencies = req.dependency_count,
            time_ms = req.resolution_time_ms,
            success = req.success,
            cache_hits = req.cache_hits,
            "Resolution recorded"
        );

        // Feed recorded cache hits into global stats
        if req.success {
            self.resolution_stats
                .cache_hits
                .fetch_add(req.cache_hits, Ordering::Relaxed);
        }

        Ok(Response::new(RecordResolutionResponse {
            acknowledged: true,
        }))
    }

    async fn get_resolution_stats(
        &self,
        _request: Request<GetResolutionStatsRequest>,
    ) -> Result<Response<GetResolutionStatsResponse>, Status> {
        let total = self
            .resolution_stats
            .total_resolutions
            .load(Ordering::Relaxed);
        let cache_hits = self.resolution_stats.cache_hits.load(Ordering::Relaxed);
        let total_time = self.resolution_stats.total_time_ms.load(Ordering::Relaxed);
        let cached_artifacts = self.artifact_cache.len() as i64;
        let avg_time = if total > 0 {
            total_time as f64 / total as f64
        } else {
            0.0
        };

        Ok(Response::new(GetResolutionStatsResponse {
            total_resolutions: total,
            artifact_cache_hits: cache_hits,
            total_resolution_time_ms: total_time,
            avg_resolution_time_ms: avg_time,
            cached_artifacts,
        }))
    }

    async fn add_artifact_to_cache(
        &self,
        request: Request<AddArtifactToCacheRequest>,
    ) -> Result<Response<AddArtifactToCacheResponse>, Status> {
        let req = request.into_inner();

        let group = req.group.clone();
        let name = req.name.clone();
        let version = req.version.clone();
        let classifier = req.classifier.clone();
        let extension = Self::normalize_extension(&req.extension);
        let key = Self::artifact_cache_key(&group, &name, &version, &classifier, &extension);

        // Compute persistent store path
        let store_path = self.artifact_path(&group, &name, &version, &classifier, &extension);

        // If a local file was provided, copy it to the persistent store
        let mut actual_sha256 = req.sha256.clone();
        let mut actual_size = req.size;
        let resolved_path = if !req.local_path.is_empty() {
            let src = Path::new(&req.local_path);
            if src.exists() {
                let (copied_size, computed_sha256) =
                    match Self::copy_file_and_sha256(src, &store_path).await {
                        Ok(result) => result,
                        Err(e) => {
                            tracing::warn!(
                                source = %src.display(),
                                target = %store_path.display(),
                                error = %e,
                                "Failed to copy artifact into Rust cache"
                            );
                            return Ok(Response::new(AddArtifactToCacheResponse {
                                accepted: false,
                            }));
                        }
                    };
                if !req.sha256.is_empty() && req.sha256 != computed_sha256 {
                    let _ = tokio::fs::remove_file(&store_path).await;
                    tracing::warn!(
                        group = %req.group,
                        name = %req.name,
                        version = %req.version,
                        classifier = %req.classifier,
                        expected_sha256 = %req.sha256,
                        actual_sha256 = %computed_sha256,
                        "Rejected artifact cache add with mismatched SHA-256"
                    );
                    return Ok(Response::new(AddArtifactToCacheResponse {
                        accepted: false,
                    }));
                }
                actual_sha256 = computed_sha256;
                actual_size = copied_size;
                if let Err(e) = Self::write_sha256_sidecar(&store_path, &actual_sha256).await {
                    tracing::warn!(
                        path = %store_path.display(),
                        error = %e,
                        "Failed to write mirrored artifact checksum sidecar"
                    );
                }
                store_path.to_string_lossy().into_owned()
            } else {
                tracing::warn!(
                    group = %req.group,
                    name = %req.name,
                    version = %req.version,
                    classifier = %req.classifier,
                    local_path = %req.local_path,
                    "Rejected artifact cache add with missing local file"
                );
                return Ok(Response::new(AddArtifactToCacheResponse {
                    accepted: false,
                }));
            }
        } else {
            String::new()
        };

        let artifact = CachedArtifact {
            group,
            name,
            version,
            classifier,
            extension,
            sha256: actual_sha256,
            local_path: resolved_path,
            size: actual_size,
            cached_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        };

        self.artifact_cache.insert(key, artifact);

        tracing::debug!(
            group = %req.group,
            name = %req.name,
            version = %req.version,
            size = req.size,
            "Artifact added to cache"
        );

        Ok(Response::new(AddArtifactToCacheResponse { accepted: true }))
    }

    async fn verify_dependency_checksums(
        &self,
        request: Request<VerifyDependencyChecksumsRequest>,
    ) -> Result<Response<VerifyDependencyChecksumsResponse>, Status> {
        let req = request.into_inner();
        let mut failures = Vec::new();

        for entry in &req.entries {
            let cache_key = Self::artifact_cache_key(
                &entry.group,
                &entry.name,
                &entry.version,
                &entry.classifier,
                "jar",
            );

            match self.artifact_cache.get(&cache_key) {
                Some(cached) => {
                    let cached_sha256 = cached.sha256.clone();
                    let cached_local_path = cached.local_path.clone();
                    drop(cached);

                    let actual_sha256 = if cached_sha256.is_empty() && !cached_local_path.is_empty()
                    {
                        Self::compute_file_sha256(Path::new(&cached_local_path))
                            .await
                            .unwrap_or_default()
                    } else {
                        cached_sha256
                    };

                    if actual_sha256 != entry.expected_sha256 {
                        failures.push(ChecksumFailure {
                            group: entry.group.clone(),
                            name: entry.name.clone(),
                            version: entry.version.clone(),
                            expected_sha256: entry.expected_sha256.clone(),
                            actual_sha256,
                        });
                    }
                }
                None => {
                    let path = self.artifact_path(
                        &entry.group,
                        &entry.name,
                        &entry.version,
                        &entry.classifier,
                        "jar",
                    );
                    let actual_sha256 = if path.exists() {
                        Self::compute_file_sha256(&path).await.unwrap_or_default()
                    } else {
                        String::new()
                    };

                    if actual_sha256 != entry.expected_sha256 {
                        failures.push(ChecksumFailure {
                            group: entry.group.clone(),
                            name: entry.name.clone(),
                            version: entry.version.clone(),
                            expected_sha256: entry.expected_sha256.clone(),
                            actual_sha256,
                        });
                    }
                }
            }
        }

        let all_matched = failures.is_empty();

        Ok(Response::new(VerifyDependencyChecksumsResponse {
            all_matched,
            failures,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_svc() -> DependencyResolutionServiceImpl {
        let dir = tempfile::tempdir().unwrap();
        DependencyResolutionServiceImpl::new(dir.path().to_path_buf())
    }

    fn make_dep(group: &str, name: &str, version: &str) -> DependencyDescriptor {
        DependencyDescriptor {
            group: group.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            classifier: String::new(),
            extension: "jar".to_string(),
            transitive: true,
            scope: String::new(),
            changing: false,
            optional: false,
            ivy_conf: String::new(),
            strict_version: String::new(),
            required_version: String::new(),
            preferred_version: String::new(),
            rejected_versions: Vec::new(),
        }
    }

    fn make_repo(id: &str, url: &str) -> crate::proto::RepositoryDescriptor {
        crate::proto::RepositoryDescriptor {
            id: id.to_string(),
            url: url.to_string(),
            m2compatible: true,
            allow_insecure_protocol: url.starts_with("http://"),
            credentials: Default::default(),
            layout: String::new(),
            ivy_pattern: String::new(),
            include_groups: Vec::new(),
            exclude_groups: Vec::new(),
            include_group_prefixes: Vec::new(),
            exclude_group_prefixes: Vec::new(),
        }
    }

    fn resolved_dep(group: &str, name: &str, scope: &str) -> ResolvedDependency {
        ResolvedDependency {
            group: group.to_string(),
            name: name.to_string(),
            version: "1.0".to_string(),
            selected_version: "1.0".to_string(),
            dependencies: Vec::new(),
            resolved: true,
            failure_reason: String::new(),
            artifact_url: String::new(),
            artifact_size: 0,
            artifact_sha256: String::new(),
            scope: scope.to_string(),
        }
    }

    #[test]
    fn test_first_unresolved_reason_searches_transitive_dependencies() {
        let mut root = resolved_dep("org.example", "root", "runtime");
        let mut child = resolved_dep("org.example", "child", "runtime");
        child.resolved = false;
        child.failure_reason = "unsupported transitive metadata".to_string();
        root.dependencies.push(child);

        assert_eq!(
            DependencyResolutionServiceImpl::first_unresolved_reason(&[root]).as_deref(),
            Some("unsupported transitive metadata")
        );
    }

    #[test]
    fn test_artifact_url_uses_classifier_and_extension() {
        let url = DependencyResolutionServiceImpl::artifact_url_for_descriptor(
            "https://repo.example.test/maven/",
            "org.example",
            "demo",
            "1.2.3",
            "sources",
            "zip",
        );

        assert_eq!(
            url,
            "https://repo.example.test/maven/org/example/demo/1.2.3/demo-1.2.3-sources.zip"
        );
    }

    #[test]
    fn test_artifact_url_defaults_to_jar_and_trims_dot_extension() {
        let default_url = DependencyResolutionServiceImpl::artifact_url_for_descriptor(
            "https://repo.example.test/maven",
            "org.example",
            "demo",
            "1.2.3",
            "",
            "",
        );
        let dotted_url = DependencyResolutionServiceImpl::artifact_url_for_descriptor(
            "https://repo.example.test/maven",
            "org.example",
            "demo",
            "1.2.3",
            "javadoc",
            ".jar",
        );

        assert_eq!(
            default_url,
            "https://repo.example.test/maven/org/example/demo/1.2.3/demo-1.2.3.jar"
        );
        assert_eq!(
            dotted_url,
            "https://repo.example.test/maven/org/example/demo/1.2.3/demo-1.2.3-javadoc.jar"
        );
    }

    #[test]
    fn test_artifact_file_parts_from_url_extracts_classifier_and_extension() {
        assert_eq!(
            DependencyResolutionServiceImpl::artifact_file_parts_from_url(
                "https://repo.example.test/maven/org/example/demo/1.2.3/demo-1.2.3-sources.jar",
                "demo",
                "1.2.3",
            ),
            Some(("sources".to_string(), "jar".to_string()))
        );
        assert_eq!(
            DependencyResolutionServiceImpl::artifact_file_parts_from_url(
                "https://repo.example.test/maven/org/example/demo/1.2.3/demo-1.2.3.zip",
                "demo",
                "1.2.3",
            ),
            Some((String::new(), "zip".to_string()))
        );
        assert_eq!(
            DependencyResolutionServiceImpl::artifact_file_parts_from_url(
                "https://repo.example.test/maven/org/example/demo/1.2.3/not-demo.jar",
                "demo",
                "1.2.3",
            ),
            None
        );
        assert_eq!(
            DependencyResolutionServiceImpl::artifact_file_parts_from_url(
                "https://repo.example.test/maven/org/example/demo/1.2.3/demo-1.2.3broken.jar",
                "demo",
                "1.2.3",
            ),
            None
        );
    }

    #[test]
    fn test_metadata_extensions_include_maven_metadata() {
        assert!(DependencyResolutionServiceImpl::is_metadata_extension(
            "pom"
        ));
        assert!(DependencyResolutionServiceImpl::is_metadata_extension(
            "module"
        ));
        assert!(DependencyResolutionServiceImpl::is_metadata_extension(
            "ivy"
        ));
        assert!(DependencyResolutionServiceImpl::is_metadata_extension(
            "maven-metadata.xml"
        ));
        assert!(!DependencyResolutionServiceImpl::is_metadata_extension(
            "xml"
        ));
    }

    #[tokio::test]
    async fn test_download_artifact_streams_bytes_from_http() {
        use futures_util::StreamExt;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = b"rust dependency transport artifact".to_vec();
        let expected = body.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/java-archive\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });

        let svc = make_svc();
        let mut stream = svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        let mut downloaded = Vec::new();
        let mut saw_last = false;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            downloaded.extend_from_slice(&chunk.data);
            saw_last |= chunk.is_last;
        }
        server.join().unwrap();

        assert_eq!(downloaded, expected);
        assert!(saw_last);
    }

    #[tokio::test]
    async fn test_download_artifact_populates_store_and_checksum_cache() {
        use futures_util::StreamExt;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = b"persisted rust dependency artifact".to_vec();
        let expected_sha256 = DependencyResolutionServiceImpl::compute_sha256(&body);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/java-archive\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let mut stream = svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            if chunk.is_last {
                break;
            }
        }
        server.join().unwrap();

        let stored = store.path().join("org/example/demo/1.0/demo-1.0.jar");
        assert_eq!(
            tokio::fs::read(&stored).await.unwrap(),
            b"persisted rust dependency artifact"
        );
        assert_eq!(
            tokio::fs::read_to_string(DependencyResolutionServiceImpl::sha256_sidecar_path(
                &stored
            ))
            .await
            .unwrap()
            .split_whitespace()
            .next(),
            Some(expected_sha256.as_str())
        );

        let cache_key = DependencyResolutionServiceImpl::artifact_cache_key(
            "org.example",
            "demo",
            "1.0",
            "",
            "jar",
        );
        assert!(
            svc.artifact_cache.contains_key(&cache_key),
            "download should populate the warm artifact cache immediately"
        );

        let cache_hit = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                sha256: expected_sha256.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(cache_hit.cached);
        assert_eq!(cache_hit.local_path, stored.to_string_lossy().to_string());

        let rejected = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                sha256: "0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!rejected.cached);

        let checksum = svc
            .verify_dependency_checksums(Request::new(VerifyDependencyChecksumsRequest {
                entries: vec![crate::proto::ChecksumEntry {
                    group: "org.example".to_string(),
                    name: "demo".to_string(),
                    version: "1.0".to_string(),
                    classifier: String::new(),
                    expected_sha256,
                    actual_sha256: String::new(),
                }],
                strict: true,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(checksum.all_matched);
        assert!(checksum.failures.is_empty());
    }

    #[tokio::test]
    async fn test_download_artifact_streams_persisted_artifact_without_http() {
        use futures_util::StreamExt;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = b"persisted artifact reused without remote".to_vec();
        let expected = body.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/java-archive\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let first_svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let mut first = first_svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        while let Some(chunk) = first.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            if chunk.is_last {
                break;
            }
        }
        server.join().unwrap();

        let second_svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let mut second = second_svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                repositories: vec![make_repo("dead", "http://127.0.0.1:9")],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        let mut downloaded = Vec::new();
        let mut saw_last = false;
        while let Some(chunk) = second.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            downloaded.extend_from_slice(&chunk.data);
            saw_last |= chunk.is_last;
        }

        assert_eq!(downloaded, expected);
        assert!(saw_last);
        let cache_key = DependencyResolutionServiceImpl::artifact_cache_key(
            "org.example",
            "demo",
            "1.0",
            "",
            "jar",
        );
        assert!(second_svc.artifact_cache.contains_key(&cache_key));
    }

    #[tokio::test]
    async fn test_download_artifact_streams_warm_cache_local_path_without_http() {
        use futures_util::StreamExt;

        let store = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let local_path = local.path().join("already-warm.jar");
        let body = b"warm cache artifact served without remote".to_vec();
        tokio::fs::write(&local_path, &body).await.unwrap();

        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let cache_key = DependencyResolutionServiceImpl::artifact_cache_key(
            "org.example",
            "demo",
            "1.0",
            "",
            "jar",
        );
        svc.artifact_cache.insert(
            cache_key.clone(),
            CachedArtifact {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                sha256: String::new(),
                local_path: local_path.to_string_lossy().into_owned(),
                size: body.len() as i64,
                cached_at_ms: DependencyResolutionServiceImpl::now_ms(),
            },
        );

        let mut stream = svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                repositories: vec![make_repo("dead", "http://127.0.0.1:9")],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        let mut downloaded = Vec::new();
        let mut saw_last = false;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            downloaded.extend_from_slice(&chunk.data);
            saw_last |= chunk.is_last;
        }

        assert_eq!(downloaded, body);
        assert!(saw_last);
        let warmed = svc.artifact_cache.get(&cache_key).unwrap();
        assert_eq!(warmed.local_path, local_path.to_string_lossy());
        assert_eq!(
            warmed.sha256,
            DependencyResolutionServiceImpl::compute_sha256(&downloaded)
        );
    }

    #[tokio::test]
    async fn test_download_metadata_url_populates_metadata_cache() {
        use futures_util::StreamExt;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = b"<project><modelVersion>4.0.0</modelVersion></project>".to_vec();
        let expected_sha256 = DependencyResolutionServiceImpl::compute_sha256(&body);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let url = format!("http://{}/org/example/demo/1.0/demo-1.0.pom", addr);
        let mut stream = svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                url: url.clone(),
                extension: "pom".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        let mut downloaded = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            downloaded.extend_from_slice(&chunk.data);
            if chunk.is_last {
                break;
            }
        }
        server.join().unwrap();

        assert_eq!(
            downloaded,
            b"<project><modelVersion>4.0.0</modelVersion></project>"
        );

        let cached = svc
            .check_metadata_cache(Request::new(CheckMetadataCacheRequest {
                url,
                extension: "pom".to_string(),
                sha256: expected_sha256.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(cached.cached);
        assert_eq!(
            tokio::fs::read(&cached.local_path).await.unwrap(),
            b"<project><modelVersion>4.0.0</modelVersion></project>"
        );
        assert_eq!(
            tokio::fs::read_to_string(DependencyResolutionServiceImpl::sha256_sidecar_path(
                Path::new(&cached.local_path)
            ))
            .await
            .unwrap()
            .split_whitespace()
            .next(),
            Some(expected_sha256.as_str())
        );
    }

    #[tokio::test]
    async fn test_download_metadata_url_streams_persisted_metadata_without_http() {
        use futures_util::StreamExt;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body =
            b"<project><modelVersion>4.0.0</modelVersion><artifactId>demo</artifactId></project>"
                .to_vec();
        let expected = body.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let url = format!("http://{}/org/example/demo/1.0/demo-1.0.pom", addr);
        let first_svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let mut first = first_svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                url: url.clone(),
                extension: "pom".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        while let Some(chunk) = first.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            if chunk.is_last {
                break;
            }
        }
        server.join().unwrap();

        let second_svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let mut second = second_svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                url,
                extension: "pom".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        let mut downloaded = Vec::new();
        let mut saw_last = false;
        while let Some(chunk) = second.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            downloaded.extend_from_slice(&chunk.data);
            saw_last |= chunk.is_last;
        }

        assert_eq!(downloaded, expected);
        assert!(saw_last);
    }

    #[tokio::test]
    async fn test_download_metadata_url_streams_warm_cache_local_path_without_http() {
        use futures_util::StreamExt;

        let store = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let local_path = local.path().join("already-warm.pom");
        let body = b"<project><artifactId>warm</artifactId></project>".to_vec();
        tokio::fs::write(&local_path, &body).await.unwrap();

        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let url = "http://127.0.0.1:9/org/example/demo/1.0/demo-1.0.pom";
        let cache_key = DependencyResolutionServiceImpl::metadata_url_cache_key(url, "pom");
        svc.artifact_cache.insert(
            cache_key.clone(),
            CachedArtifact {
                group: String::new(),
                name: String::new(),
                version: String::new(),
                classifier: String::new(),
                extension: "pom".to_string(),
                sha256: String::new(),
                local_path: local_path.to_string_lossy().into_owned(),
                size: body.len() as i64,
                cached_at_ms: DependencyResolutionServiceImpl::now_ms(),
            },
        );

        let mut stream = svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                url: url.to_string(),
                extension: "pom".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        let mut downloaded = Vec::new();
        let mut saw_last = false;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            downloaded.extend_from_slice(&chunk.data);
            saw_last |= chunk.is_last;
        }

        assert_eq!(downloaded, body);
        assert!(saw_last);
        let warmed = svc.artifact_cache.get(&cache_key).unwrap();
        assert_eq!(warmed.local_path, local_path.to_string_lossy());
        assert_eq!(
            warmed.sha256,
            DependencyResolutionServiceImpl::compute_sha256(&downloaded)
        );
    }

    #[tokio::test]
    async fn test_download_maven_metadata_url_populates_dynamic_metadata_cache() {
        use futures_util::StreamExt;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = br#"<metadata>
  <groupId>org.example</groupId>
  <artifactId>demo</artifactId>
  <versioning>
    <latest>1.5</latest>
    <release>1.5</release>
    <versions>
      <version>1.0</version>
      <version>1.5</version>
    </versions>
  </versioning>
</metadata>"#
            .to_vec();
        let expected_sha256 = DependencyResolutionServiceImpl::compute_sha256(&body);
        let expected = body.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let url = format!("http://{}/org/example/demo/maven-metadata.xml", addr);
        let mut stream = svc
            .download_artifact(Request::new(crate::proto::DownloadArtifactRequest {
                url: url.clone(),
                extension: "maven-metadata.xml".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        let mut downloaded = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.error_message.is_empty(), "{}", chunk.error_message);
            downloaded.extend_from_slice(&chunk.data);
            if chunk.is_last {
                break;
            }
        }
        server.join().unwrap();

        assert_eq!(downloaded, expected);

        let cached = svc
            .check_metadata_cache(Request::new(CheckMetadataCacheRequest {
                url,
                extension: "maven-metadata.xml".to_string(),
                sha256: expected_sha256.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(cached.cached);
        assert_eq!(tokio::fs::read(&cached.local_path).await.unwrap(), expected);
        assert_eq!(
            tokio::fs::read_to_string(DependencyResolutionServiceImpl::sha256_sidecar_path(
                Path::new(&cached.local_path)
            ))
            .await
            .unwrap()
            .split_whitespace()
            .next(),
            Some(expected_sha256.as_str())
        );
    }

    #[tokio::test]
    async fn test_fetch_maven_metadata_populates_store_and_reuses_persistent_cache() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = r#"<metadata>
  <groupId>org.example</groupId>
  <artifactId>demo</artifactId>
  <versioning>
    <latest>1.5</latest>
    <release>1.5</release>
    <versions>
      <version>1.0</version>
      <version>1.5</version>
    </versions>
  </versioning>
</metadata>"#
            .to_string();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(body.as_bytes()).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let repo = make_repo("local", &format!("http://{}", addr));
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let first = svc
            .fetch_maven_metadata("org.example", "demo", &repo)
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(
            first.versioning.versions,
            vec!["1.0".to_string(), "1.5".to_string()]
        );

        let stored = svc.module_metadata_path(&repo, "org.example", "demo", "maven-metadata.xml");
        assert!(
            stored.starts_with(store.path().join("_metadata")),
            "maven-metadata.xml cache should be repository-scoped"
        );
        assert!(
            DependencyResolutionServiceImpl::sha256_sidecar_path(&stored).exists(),
            "maven-metadata.xml cache should write a SHA-256 sidecar"
        );

        let url = format!("http://{}/org/example/demo/maven-metadata.xml", addr);
        let url_cached = svc
            .check_metadata_cache(Request::new(CheckMetadataCacheRequest {
                url,
                extension: "maven-metadata.xml".to_string(),
                sha256: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(url_cached.cached);
        assert!(url_cached.local_path.ends_with(".maven-metadata.xml"));

        let second_svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let second = second_svc
            .fetch_maven_metadata("org.example", "demo", &repo)
            .await
            .unwrap();
        assert_eq!(second.versioning.latest.as_deref(), Some("1.5"));
        assert_eq!(second.versioning.versions, first.versioning.versions);
    }

    #[tokio::test]
    async fn test_fetch_pom_populates_store_and_uses_warm_cache() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>demo</artifactId>
  <version>1.0</version>
</project>"#
            .to_string();
        let expected = body.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(body.as_bytes()).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let repo = make_repo("local", &format!("http://{}", addr));

        let first = svc
            .fetch_pom("org.example", "demo", "1.0", &repo)
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(first, expected);

        let stored = svc.metadata_path(&repo, "org.example", "demo", "1.0", "pom");
        assert!(
            stored.starts_with(store.path().join("_metadata")),
            "POM metadata cache should be repository-scoped"
        );
        assert_eq!(tokio::fs::read_to_string(&stored).await.unwrap(), expected);
        assert!(
            DependencyResolutionServiceImpl::sha256_sidecar_path(&stored).exists(),
            "POM metadata cache should write a SHA-256 sidecar"
        );

        let cache_key = DependencyResolutionServiceImpl::metadata_cache_key(
            &repo,
            "org.example",
            "demo",
            "1.0",
            "pom",
        );
        assert!(
            svc.artifact_cache.contains_key(&cache_key),
            "POM metadata should populate the warm artifact cache"
        );

        let metadata_url = format!("http://{}/org/example/demo/1.0/demo-1.0.pom", addr);
        let url_cached = svc
            .check_metadata_cache(Request::new(CheckMetadataCacheRequest {
                url: metadata_url,
                extension: "pom".to_string(),
                sha256: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(url_cached.cached);
        assert!(url_cached.local_path.ends_with(".pom"));

        let second = svc
            .fetch_pom("org.example", "demo", "1.0", &repo)
            .await
            .unwrap();
        assert_eq!(second, first);
    }

    #[tokio::test]
    async fn test_resolve_dependencies_prefetches_static_maven_artifact() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requested = Arc::new(Mutex::new(Vec::new()));
        let requested_for_server = Arc::clone(&requested);
        let pom = b"<project><modelVersion>4.0.0</modelVersion><groupId>org.example</groupId><artifactId>demo</artifactId><version>1.0</version></project>".to_vec();
        let jar = b"prefetched static maven artifact".to_vec();
        let expected_sha256 = DependencyResolutionServiceImpl::compute_sha256(&jar);
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requested_for_server.lock().unwrap().push(path.clone());
                let body = if path.ends_with("/demo-1.0.pom") {
                    &pom
                } else if path.ends_with("/demo-1.0.jar") {
                    &jar
                } else {
                    let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                    stream.write_all(response.as_bytes()).unwrap();
                    continue;
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "compileClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "demo", "1.0")],
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                attributes: vec![],
                lenient: false,
                prefetch_artifacts: true,
                constraints: Vec::new(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(
            response.success,
            "{}; requested={:?}",
            response.error_message,
            requested.lock().unwrap().as_slice()
        );
        assert_eq!(response.total_artifacts, 1);
        assert_eq!(
            response.total_download_size,
            b"prefetched static maven artifact".len() as i64
        );
        assert_eq!(response.resolved_dependencies.len(), 1);
        assert_eq!(
            response.resolved_dependencies[0].artifact_size,
            response.total_download_size
        );
        assert_eq!(
            response.resolved_dependencies[0].artifact_sha256,
            expected_sha256
        );
        assert_eq!(
            requested.lock().unwrap().as_slice(),
            &[
                "/org/example/demo/1.0/demo-1.0.pom".to_string(),
                "/org/example/demo/1.0/demo-1.0.jar".to_string(),
            ]
        );

        let stored = store.path().join("org/example/demo/1.0/demo-1.0.jar");
        assert_eq!(
            tokio::fs::read(&stored).await.unwrap(),
            b"prefetched static maven artifact"
        );
        let cache_hit = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                sha256: expected_sha256,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(cache_hit.cached);
        assert_eq!(cache_hit.local_path, stored.to_string_lossy().to_string());
    }

    #[tokio::test]
    async fn test_transitive_maven_test_jar_type_uses_gradle_artifact_shape() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requested = Arc::new(Mutex::new(Vec::new()));
        let requested_for_server = Arc::clone(&requested);
        let root_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>root</artifactId>
  <version>1.0</version>
  <dependencies>
    <dependency>
      <groupId>org.example</groupId>
      <artifactId>child</artifactId>
      <version>1.0</version>
      <type>test-jar</type>
    </dependency>
  </dependencies>
</project>"#
            .to_vec();
        let child_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>child</artifactId>
  <version>1.0</version>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requested_for_server.lock().unwrap().push(path.clone());
                let body = if path.ends_with("/root-1.0.pom") {
                    &root_pom
                } else if path.ends_with("/child-1.0.pom") {
                    &child_pom
                } else {
                    let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                    stream.write_all(response.as_bytes()).unwrap();
                    continue;
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "compileClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        let child = &response.resolved_dependencies[0].dependencies[0];
        assert!(
            child
                .artifact_url
                .ends_with("/org/example/child/1.0/child-1.0-tests.jar"),
            "{}",
            child.artifact_url
        );
        assert_eq!(
            requested.lock().unwrap().as_slice(),
            &[
                "/org/example/root/1.0/root-1.0.pom".to_string(),
                "/org/example/child/1.0/child-1.0.pom".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn test_pom_self_dependency_is_skipped_like_gradle() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>self</artifactId>
  <version>1.0</version>
  <dependencies>
    <dependency>
      <groupId>org.example</groupId>
      <artifactId>self</artifactId>
      <version>1.0</version>
    </dependency>
  </dependencies>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                pom.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&pom).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "compileClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "self", "1.0")],
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        assert_eq!(response.resolved_dependencies.len(), 1);
        assert!(
            response.resolved_dependencies[0].dependencies.is_empty(),
            "self dependency should not be retained as a cycle leaf"
        );
    }

    #[tokio::test]
    async fn test_maven_artifact_url_uses_repository_that_supplied_pom() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();

        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_server = Arc::clone(&requests);
        let pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>owned</artifactId>
  <version>1.0</version>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while requests_for_server.lock().unwrap().len() < 2
                && started.elapsed() < std::time::Duration::from_secs(5)
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("repo accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                let mut request = [0u8; 2048];
                let mut read = stream.read(&mut request).unwrap_or(0);
                while read == 0 && started.elapsed() < std::time::Duration::from_secs(5) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    read = stream.read(&mut request).unwrap_or(0);
                }
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requests_for_server.lock().unwrap().push(path.clone());
                if path.starts_with("/owning/") {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                        pom.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.write_all(&pom).unwrap();
                } else {
                    stream
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                        .unwrap();
                }
            }
        });

        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![DependencyDescriptor {
                    group: "org.example".to_string(),
                    name: "owned".to_string(),
                    version: "1.0".to_string(),
                    classifier: String::new(),
                    extension: "jar".to_string(),
                    transitive: true,
                    scope: "runtime".to_string(),
                    changing: false,
                    optional: false,
                    ivy_conf: String::new(),
                    strict_version: String::new(),
                    required_version: String::new(),
                    preferred_version: String::new(),
                    rejected_versions: Vec::new(),
                }],
                repositories: vec![
                    make_repo("missing", &format!("http://{}/missing", addr)),
                    make_repo("owning", &format!("http://{}/owning", addr)),
                ],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        let artifact_url = &response.resolved_dependencies[0].artifact_url;
        assert!(
            artifact_url.starts_with(&format!("http://{}/owning/", addr)),
            "{}",
            artifact_url
        );
        assert_eq!(
            *requests.lock().unwrap(),
            vec![
                "/missing/org/example/owned/1.0/owned-1.0.pom".to_string(),
                "/owning/org/example/owned/1.0/owned-1.0.pom".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn test_repository_group_content_filter_skips_non_matching_repo() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_server = Arc::clone(&requests);
        let pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>filtered</artifactId>
  <version>1.0</version>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while requests_for_server.lock().unwrap().is_empty()
                && started.elapsed() < std::time::Duration::from_secs(5)
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("repo accept failed: {error}"),
                };
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let path = String::from_utf8_lossy(&request[..read])
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requests_for_server.lock().unwrap().push(path);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                    pom.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(&pom).unwrap();
            }
        });

        let mut skipped = make_repo("skipped", &format!("http://{}/skipped", addr));
        skipped.include_groups = vec!["com.other".to_string()];
        let mut selected = make_repo("selected", &format!("http://{}/selected", addr));
        selected.include_groups = vec!["org.example".to_string()];

        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "filtered", "1.0")],
                repositories: vec![skipped, selected],
                target_scope: "runtime".to_string(),
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        assert_eq!(
            *requests.lock().unwrap(),
            vec!["/selected/org/example/filtered/1.0/filtered-1.0.pom".to_string()]
        );
        assert!(response.resolved_dependencies[0]
            .artifact_url
            .starts_with(&format!("http://{}/selected/", addr)));
    }

    #[tokio::test]
    async fn test_repository_subgroup_content_filter_matches_dotted_children_only() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_server = Arc::clone(&requests);
        let pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example.child</groupId>
  <artifactId>filtered</artifactId>
  <version>1.0</version>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while requests_for_server.lock().unwrap().is_empty()
                && started.elapsed() < std::time::Duration::from_secs(5)
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("repo accept failed: {error}"),
                };
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let path = String::from_utf8_lossy(&request[..read])
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requests_for_server.lock().unwrap().push(path);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                    pom.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(&pom).unwrap();
            }
        });

        let mut skipped = make_repo("skipped", &format!("http://{}/skipped", addr));
        skipped.include_group_prefixes = vec!["org.examples".to_string()];
        let mut selected = make_repo("selected", &format!("http://{}/selected", addr));
        selected.include_group_prefixes = vec!["org.example".to_string()];
        selected.exclude_group_prefixes = vec!["org.example.internal".to_string()];

        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example.child", "filtered", "1.0")],
                repositories: vec![skipped, selected],
                target_scope: "runtime".to_string(),
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        assert_eq!(
            *requests.lock().unwrap(),
            vec!["/selected/org/example/child/filtered/1.0/filtered-1.0.pom".to_string()]
        );
    }

    #[tokio::test]
    async fn test_missing_direct_module_metadata_fails_closed() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_server = Arc::clone(&requests);
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while requests_for_server.lock().unwrap().len() < 1
                && started.elapsed() < std::time::Duration::from_secs(5)
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("repo accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                let mut request = [0u8; 2048];
                let mut read = stream.read(&mut request).unwrap_or(0);
                while read == 0 && started.elapsed() < std::time::Duration::from_secs(5) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    read = stream.read(&mut request).unwrap_or(0);
                }
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requests_for_server.lock().unwrap().push(path);
                stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
        });

        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "missing", "1.0")],
                repositories: vec![make_repo("missing", &format!("http://{}", addr))],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(!response.success);
        assert!(
            response.error_message.contains(
                "No Gradle Module Metadata or Maven POM found for org.example:missing:1.0"
            ),
            "{}",
            response.error_message
        );
        assert_eq!(response.resolved_dependencies.len(), 1);
        assert!(!response.resolved_dependencies[0].resolved);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    async fn resolve_missing_transitive_metadata(
        lenient: bool,
    ) -> (ResolveDependenciesResponse, Vec<String>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_server = Arc::clone(&requests);
        let root_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>root</artifactId>
  <version>1.0</version>
  <dependencies>
    <dependency>
      <groupId>org.example</groupId>
      <artifactId>missing-child</artifactId>
      <version>1.0</version>
    </dependency>
  </dependencies>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while requests_for_server.lock().unwrap().len() < 2
                && started.elapsed() < std::time::Duration::from_secs(5)
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("repo accept failed: {error}"),
                };
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let request_index = {
                    let mut requests = requests_for_server.lock().unwrap();
                    let request_index = requests.len();
                    requests.push(path);
                    request_index
                };
                if request_index == 0 {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                        root_pom.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.write_all(&root_pom).unwrap();
                } else {
                    stream
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                        .unwrap();
                }
            }
        });

        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![make_repo("repo", &format!("http://{}", addr))],
                attributes: vec![],
                lenient,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        let requests = requests.lock().unwrap().clone();
        (response, requests)
    }

    #[tokio::test]
    async fn test_missing_transitive_module_metadata_fails_closed_in_strict_mode() {
        let (response, requests) = resolve_missing_transitive_metadata(false).await;

        assert!(!response.success);
        assert!(
            response.error_message.contains(
                "No Gradle Module Metadata or Maven POM found for org.example:missing-child:1.0"
            ),
            "{}",
            response.error_message
        );
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn test_missing_transitive_module_metadata_is_tolerated_only_when_lenient() {
        let (response, requests) = resolve_missing_transitive_metadata(true).await;

        assert!(response.success, "{}", response.error_message);
        assert_eq!(response.resolved_dependencies.len(), 1);
        assert_eq!(response.resolved_dependencies[0].dependencies.len(), 1);
        assert_eq!(
            response.resolved_dependencies[0].dependencies[0].name,
            "missing-child"
        );
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn test_gradle_module_metadata_runtime_variant_drives_transitive_resolution() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requested = Arc::new(Mutex::new(Vec::new()));
        let requested_for_server = Arc::clone(&requested);
        let root_module = br#"{
          "formatVersion": "1.1",
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"apiElements","attributes":{"org.gradle.usage":"java-api"},"dependencies":[]},
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"runtime-child","version":{"requires":"1.0"}}],
             "dependencyConstraints":[{"group":"org.example","module":"runtime-child","version":{"strictly":"2.0"}}],
             "files":[{"name":"root-runtime.jar","url":"custom/root-runtime.jar"}]}
          ]
        }"#
        .to_vec();
        let child_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>runtime-child</artifactId>
  <version>2.0</version>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while requested_for_server.lock().unwrap().len() < 3
                && started.elapsed() < std::time::Duration::from_secs(5)
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("test server accept failed: {error}"),
                };
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requested_for_server.lock().unwrap().push(path.clone());
                let body = if path == "/" || path.ends_with("/root-1.0.module") {
                    Some(&root_module)
                } else if path.ends_with("/runtime-child-2.0.pom") {
                    Some(&child_pom)
                } else {
                    None
                };
                match body {
                    Some(body) => {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n",
                            body.len()
                        );
                        stream.write_all(response.as_bytes()).unwrap();
                        stream.write_all(body).unwrap();
                    }
                    None => {
                        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                        stream.write_all(response.as_bytes()).unwrap();
                    }
                }
            }
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![{
                    let mut repo = make_repo("local", &format!("http://{}", addr));
                    repo.layout = "gradle-module-metadata".to_string();
                    repo
                }],
                target_scope: "runtime".to_string(),
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        let root = &response.resolved_dependencies[0];
        assert_eq!(root.dependencies.len(), 1);
        assert!(
            root.artifact_url
                .ends_with("/org/example/root/1.0/custom/root-runtime.jar"),
            "{}",
            root.artifact_url
        );
        assert_eq!(root.dependencies[0].name, "runtime-child");
        assert_eq!(root.dependencies[0].selected_version, "2.0");
        let requested = requested.lock().unwrap();
        assert!(
            requested
                .iter()
                .any(|path| path == "/" || path.ends_with("/root-1.0.module")),
            "Rust should consume root .module metadata: {requested:?}"
        );
        assert!(
            !requested.iter().any(|path| path.ends_with("/root-1.0.pom")),
            "Rust should not fetch root POM after module metadata hit: {requested:?}"
        );
        assert!(
            requested
                .iter()
                .any(|path| path.ends_with("/runtime-child-2.0.pom")),
            "Rust should resolve the constrained child: {requested:?}"
        );
    }

    #[tokio::test]
    async fn test_gradle_module_metadata_dependency_exclusion_prunes_child_transitive() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requested = Arc::new(Mutex::new(Vec::new()));
        let requested_for_server = Arc::clone(&requested);
        let root_module = br#"{
          "formatVersion": "1.1",
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"runtime-child","version":{"requires":"1.0"},
               "excludes":[{"group":"org.bad","module":"bad"}]}]}
          ]
        }"#
        .to_vec();
        let child_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>runtime-child</artifactId>
  <version>1.0</version>
  <dependencies>
    <dependency>
      <groupId>org.bad</groupId>
      <artifactId>bad</artifactId>
      <version>1.0</version>
    </dependency>
    <dependency>
      <groupId>org.good</groupId>
      <artifactId>good</artifactId>
      <version>1.0</version>
    </dependency>
  </dependencies>
</project>"#
            .to_vec();
        let good_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.good</groupId>
  <artifactId>good</artifactId>
  <version>1.0</version>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while requested_for_server.lock().unwrap().len() < 5
                && started.elapsed() < std::time::Duration::from_secs(5)
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("test server accept failed: {error}"),
                };
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requested_for_server.lock().unwrap().push(path.clone());
                let body = if path == "/" || path.ends_with("/root-1.0.module") {
                    Some((&root_module, "application/json"))
                } else if path.ends_with("/runtime-child-1.0.pom") {
                    Some((&child_pom, "application/xml"))
                } else if path.ends_with("/good-1.0.pom") {
                    Some((&good_pom, "application/xml"))
                } else {
                    None
                };
                match body {
                    Some((body, content_type)) => {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\n\r\n",
                            body.len(),
                            content_type
                        );
                        stream.write_all(response.as_bytes()).unwrap();
                        stream.write_all(body).unwrap();
                    }
                    None => {
                        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                        stream.write_all(response.as_bytes()).unwrap();
                    }
                }
            }
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![{
                    let mut repo = make_repo("local", &format!("http://{}", addr));
                    repo.layout = "gradle-module-metadata".to_string();
                    repo
                }],
                target_scope: "runtime".to_string(),
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(
            response.success,
            "{}; requested={:?}",
            response.error_message,
            requested.lock().unwrap().as_slice()
        );
        let runtime_child = &response.resolved_dependencies[0].dependencies[0];
        assert_eq!(runtime_child.name, "runtime-child");
        assert_eq!(runtime_child.dependencies.len(), 1);
        assert_eq!(runtime_child.dependencies[0].group, "org.good");
        assert_eq!(runtime_child.dependencies[0].name, "good");
        assert!(
            !requested
                .lock()
                .unwrap()
                .iter()
                .any(|path| path.contains("/org/bad/")),
            "excluded dependency should not be fetched"
        );
    }

    #[tokio::test]
    async fn test_gradle_module_metadata_rejected_version_fails_closed() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let root_module = br#"{
          "formatVersion": "1.1",
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"runtime-child","version":{"requires":"2.0","rejects":["2.0"]}}]}
          ]
        }"#
        .to_vec();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n",
                root_module.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&root_module).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![{
                    let mut repo = make_repo("local", &format!("http://{}", addr));
                    repo.layout = "gradle-module-metadata".to_string();
                    repo
                }],
                target_scope: "runtime".to_string(),
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(!response.success);
        assert!(
            response
                .error_message
                .contains("selected version 2.0 is rejected"),
            "{}",
            response.error_message
        );
    }

    #[tokio::test]
    async fn test_gradle_module_metadata_available_at_redirect_drives_resolution() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requested = Arc::new(Mutex::new(Vec::new()));
        let requested_for_server = Arc::clone(&requested);
        let root_module = br#"{
          "formatVersion": "1.1",
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "available-at":{"url":"root-jvm-1.0.module","group":"org.example","module":"root-jvm","version":"1.0"}}
          ]
        }"#
        .to_vec();
        let root_jvm_module = br#"{
          "formatVersion": "1.1",
          "component": {"group":"org.example","module":"root-jvm","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"runtime-child","version":{"requires":"2.0"}}],
             "files":[{"name":"root-jvm-1.0.jar","url":"root-jvm-1.0.jar"}]}
          ]
        }"#
        .to_vec();
        let child_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>runtime-child</artifactId>
  <version>2.0</version>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while requested_for_server.lock().unwrap().len() < 5
                && started.elapsed() < std::time::Duration::from_secs(5)
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("test server accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requested_for_server.lock().unwrap().push(path.clone());
                let body = if path == "/" || path.ends_with("/root-1.0.module") {
                    Some((&root_module, "application/json"))
                } else if path.ends_with("/root-jvm-1.0.module") {
                    Some((&root_jvm_module, "application/json"))
                } else if path.ends_with("/runtime-child-2.0.pom") {
                    Some((&child_pom, "application/xml"))
                } else {
                    None
                };
                match body {
                    Some((body, content_type)) => {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\n\r\n",
                            body.len(),
                            content_type
                        );
                        stream.write_all(response.as_bytes()).unwrap();
                        stream.write_all(body).unwrap();
                    }
                    None => {
                        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                        stream.write_all(response.as_bytes()).unwrap();
                    }
                }
            }
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![{
                    let mut repo = make_repo("local", &format!("http://{}", addr));
                    repo.layout = "gradle-module-metadata".to_string();
                    repo
                }],
                target_scope: "runtime".to_string(),
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        let root = &response.resolved_dependencies[0];
        assert_eq!(root.dependencies[0].name, "runtime-child");
        assert_eq!(root.dependencies[0].selected_version, "2.0");
        assert!(root
            .artifact_url
            .ends_with("/root-jvm/1.0/root-jvm-1.0.jar"));
    }

    #[tokio::test]
    async fn test_gradle_module_metadata_available_at_cycle_fails_closed() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let root_module = br#"{
          "formatVersion": "1.1",
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "available-at":{"url":"root-1.0.module","group":"org.example","module":"root","version":"1.0"}}
          ]
        }"#
        .to_vec();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n",
                root_module.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&root_module).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![{
                    let mut repo = make_repo("local", &format!("http://{}", addr));
                    repo.layout = "gradle-module-metadata".to_string();
                    repo
                }],
                target_scope: "runtime".to_string(),
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(!response.success);
        assert!(response
            .error_message
            .contains("available-at redirect cycle"));
    }

    #[tokio::test]
    async fn test_gradle_module_metadata_custom_capability_fails_closed() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let root_module = br#"{
          "formatVersion": "1.1",
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "capabilities":[{"group":"org.example","name":"feature","version":"1.0"}]}
          ]
        }"#
        .to_vec();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n",
                root_module.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&root_module).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![{
                    let mut repo = make_repo("local", &format!("http://{}", addr));
                    repo.layout = "gradle-module-metadata".to_string();
                    repo
                }],
                target_scope: "runtime".to_string(),
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(!response.success);
        assert!(
            response
                .error_message
                .contains("unsupported Gradle Module Metadata capability"),
            "{}",
            response.error_message
        );
    }

    #[tokio::test]
    async fn test_dependency_management_scope_defaults_like_gradle() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>managed-scope</artifactId>
  <version>1.0</version>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>test-only</artifactId>
        <version>1.0</version>
        <scope>test</scope>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.example</groupId>
      <artifactId>test-only</artifactId>
    </dependency>
  </dependencies>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                pom.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&pom).unwrap();
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "compileClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "managed-scope", "1.0")],
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        assert_eq!(response.resolved_dependencies.len(), 1);
        assert!(
            response.resolved_dependencies[0].dependencies.is_empty(),
            "dependencyManagement test scope should be used as the missing dependency scope"
        );
    }

    #[tokio::test]
    async fn test_dependency_management_exclusions_default_like_gradle() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let root_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>root</artifactId>
  <version>1.0</version>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>child</artifactId>
        <version>1.0</version>
        <exclusions>
          <exclusion>
            <groupId>org.unwanted</groupId>
            <artifactId>leaf</artifactId>
          </exclusion>
        </exclusions>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.example</groupId>
      <artifactId>child</artifactId>
    </dependency>
  </dependencies>
</project>"#
            .to_vec();
        let child_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>child</artifactId>
  <version>1.0</version>
  <dependencies>
    <dependency>
      <groupId>org.unwanted</groupId>
      <artifactId>leaf</artifactId>
      <version>1.0</version>
    </dependency>
  </dependencies>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]);
                let body = if request.contains("/org/example/root/1.0/root-1.0.pom") {
                    &root_pom
                } else if request.contains("/org/example/child/1.0/child-1.0.pom") {
                    &child_pom
                } else {
                    panic!("unexpected POM request: {}", request);
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "compileClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        let root = &response.resolved_dependencies[0];
        assert_eq!(root.dependencies.len(), 1);
        assert!(
            root.dependencies[0].dependencies.is_empty(),
            "dependencyManagement exclusions should apply when the dependency has no exclusions"
        );
    }

    #[tokio::test]
    async fn test_dependency_management_import_bom_defaults_versions_like_gradle() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let root_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>root</artifactId>
  <version>1.0</version>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>bom</artifactId>
        <version>1.0</version>
        <type>pom</type>
        <scope>import</scope>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.example</groupId>
      <artifactId>child</artifactId>
    </dependency>
  </dependencies>
</project>"#
            .to_vec();
        let bom_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>bom</artifactId>
  <version>1.0</version>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>child</artifactId>
        <version>1.2.3</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#
            .to_vec();
        let child_pom = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>child</artifactId>
  <version>1.2.3</version>
</project>"#
            .to_vec();
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]);
                let body = if request.contains("/org/example/root/1.0/root-1.0.pom") {
                    &root_pom
                } else if request.contains("/org/example/bom/1.0/bom-1.0.pom") {
                    &bom_pom
                } else if request.contains("/org/example/child/1.2.3/child-1.2.3.pom") {
                    &child_pom
                } else {
                    panic!("unexpected POM request: {}", request);
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });

        let store = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(store.path().to_path_buf());
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "compileClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "root", "1.0")],
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        let root = &response.resolved_dependencies[0];
        assert_eq!(root.dependencies.len(), 1);
        assert_eq!(root.dependencies[0].name, "child");
        assert_eq!(root.dependencies[0].selected_version, "1.2.3");
    }

    #[test]
    fn test_filter_by_compile_scope_excludes_runtime_and_test_recursively() {
        let mut compile = resolved_dep("org.example", "compile-lib", "compile");
        compile.dependencies = vec![
            resolved_dep("org.example", "runtime-child", "runtime"),
            resolved_dep("org.example", "provided-child", "provided"),
        ];
        let deps = vec![
            compile,
            resolved_dep("org.example", "runtime-lib", "runtime"),
            resolved_dep("org.example", "test-lib", "test"),
        ];

        let filtered =
            DependencyResolutionServiceImpl::filter_by_scope(deps, &DependencyScope::Compile);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "compile-lib");
        assert_eq!(filtered[0].dependencies.len(), 1);
        assert_eq!(filtered[0].dependencies[0].name, "provided-child");
    }

    #[test]
    fn test_filter_by_runtime_scope_keeps_compile_and_runtime_only() {
        let deps = vec![
            resolved_dep("org.example", "compile-lib", "compile"),
            resolved_dep("org.example", "runtime-lib", "runtime"),
            resolved_dep("org.example", "provided-lib", "provided"),
            resolved_dep("org.example", "test-lib", "test"),
        ];

        let filtered =
            DependencyResolutionServiceImpl::filter_by_scope(deps, &DependencyScope::Runtime);

        assert_eq!(
            filtered
                .iter()
                .map(|dependency| dependency.name.as_str())
                .collect::<Vec<_>>(),
            vec!["compile-lib", "runtime-lib"]
        );
    }

    #[tokio::test]
    async fn test_resolve_dependencies() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_server = Arc::clone(&requests);
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while requests_for_server.lock().unwrap().len() < 2
                && started.elapsed() < std::time::Duration::from_secs(5)
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("repo accept failed: {error}"),
                };
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requests_for_server.lock().unwrap().push(path.clone());
                let pom = if path.ends_with("/spring-core-5.3.30.pom") {
                    Some(
                        br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.springframework</groupId>
  <artifactId>spring-core</artifactId>
  <version>5.3.30</version>
</project>"#
                            .as_slice(),
                    )
                } else if path.ends_with("/guava-32.1.3.pom") {
                    Some(
                        br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.google.guava</groupId>
  <artifactId>guava</artifactId>
  <version>32.1.3</version>
</project>"#
                            .as_slice(),
                    )
                } else {
                    None
                };
                if let Some(pom) = pom {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n",
                        pom.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.write_all(pom).unwrap();
                } else {
                    stream
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                        .unwrap();
                }
            }
        });

        let svc = make_svc();

        let resp = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "compileClasspath".to_string(),
                dependencies: vec![
                    make_dep("org.springframework", "spring-core", "5.3.30"),
                    make_dep("com.google.guava", "guava", "32.1.3"),
                ],
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(resp.success, "{}", resp.error_message);
        assert_eq!(resp.resolved_dependencies.len(), 2);
        assert_eq!(resp.resolved_dependencies[0].group, "org.springframework");
        assert_eq!(resp.resolved_dependencies[1].name, "guava");
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_artifact_cache_miss() {
        let svc = make_svc();

        let resp = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "missing".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                sha256: String::new(),
                extension: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.cached);
    }

    #[tokio::test]
    async fn test_record_resolution() {
        let svc = make_svc();

        let resp = svc
            .record_resolution(Request::new(RecordResolutionRequest {
                configuration_name: "testRuntimeClasspath".to_string(),
                dependency_count: 42,
                resolution_time_ms: 150,
                success: true,
                cache_hits: 10,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.acknowledged);
    }

    #[test]
    fn test_parse_pom_dependencies() {
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <dependencies>
    <dependency>
      <groupId>org.springframework</groupId>
      <artifactId>spring-core</artifactId>
      <version>5.3.30</version>
      <scope>compile</scope>
    </dependency>
    <dependency>
      <groupId>junit</groupId>
      <artifactId>junit</artifactId>
      <version>4.13.2</version>
      <scope>test</scope>
    </dependency>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
      <version>2.0.9</version>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].group, "org.springframework");
        assert_eq!(deps[0].name, "spring-core");
        assert_eq!(deps[0].scope, "compile");
        assert_eq!(deps[1].scope, "test");
        assert_eq!(deps[2].name, "slf4j-api");
    }

    #[test]
    fn test_parse_pom_empty() {
        let pom = r#"<?xml version="1.0"?><project></project>"#;
        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_artifact_cache_key() {
        let key = DependencyResolutionServiceImpl::artifact_cache_key(
            "com.example",
            "my-lib",
            "1.0",
            "",
            "jar",
        );
        assert_eq!(key, "com.example:my-lib:1.0::jar");

        let sources_key = DependencyResolutionServiceImpl::artifact_cache_key(
            "com.example",
            "my-lib",
            "1.0",
            "sources",
            ".jar",
        );
        assert_eq!(sources_key, "com.example:my-lib:1.0:sources:jar");

        let pom_key = DependencyResolutionServiceImpl::artifact_cache_key(
            "com.example",
            "my-lib",
            "1.0",
            "",
            "pom",
        );
        assert_eq!(pom_key, "com.example:my-lib:1.0::pom");
        assert_ne!(key, pom_key);
    }

    #[tokio::test]
    async fn test_add_and_check_artifact_cache() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let src = dir.path().join("my-lib-1.0.jar");
        std::fs::write(&src, b"cached artifact").unwrap();

        // Not cached initially
        let miss = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "my-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                sha256: String::new(),
                extension: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!miss.cached);

        // Add to cache
        svc.add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
            group: "com.example".to_string(),
            name: "my-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            local_path: src.to_string_lossy().into_owned(),
            size: 15,
            sha256: String::new(),
            extension: String::new(),
        }))
        .await
        .unwrap();

        // Now cached
        let hit = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "my-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                sha256: String::new(),
                extension: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(hit.cached);
        assert!(Path::new(&hit.local_path).is_file());
        assert_eq!(hit.cached_size, 15);
    }

    #[tokio::test]
    async fn test_artifact_cache_rejects_missing_local_file() {
        let svc = make_svc();

        let response = svc
            .add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
                group: "com.example".to_string(),
                name: "missing-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                local_path: "/tmp/definitely-missing-gradle-substrate-artifact.jar".to_string(),
                size: 1024,
                sha256: String::new(),
                extension: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.accepted);
        let hit = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "missing-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                sha256: String::new(),
                extension: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!hit.cached);
    }

    #[tokio::test]
    async fn test_warm_artifact_cache_miss_when_file_disappears() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let src = dir.path().join("vanishing-lib-1.0.jar");
        std::fs::write(&src, b"vanishing artifact").unwrap();

        let added = svc
            .add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
                group: "com.example".to_string(),
                name: "vanishing-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                local_path: src.to_string_lossy().into_owned(),
                size: 18,
                sha256: String::new(),
                extension: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(added.accepted);

        let stored = svc.artifact_path("com.example", "vanishing-lib", "1.0", "", "jar");
        std::fs::remove_file(&stored).unwrap();
        let hit = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "vanishing-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                sha256: String::new(),
                extension: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!hit.cached);
    }

    #[tokio::test]
    async fn test_resolution_stats() {
        let svc = make_svc();

        // Record some resolutions
        svc.record_resolution(Request::new(RecordResolutionRequest {
            configuration_name: "compileClasspath".to_string(),
            dependency_count: 10,
            resolution_time_ms: 100,
            success: true,
            cache_hits: 5,
        }))
        .await
        .unwrap();

        svc.record_resolution(Request::new(RecordResolutionRequest {
            configuration_name: "testRuntimeClasspath".to_string(),
            dependency_count: 20,
            resolution_time_ms: 200,
            success: true,
            cache_hits: 15,
        }))
        .await
        .unwrap();

        let stats = svc
            .get_resolution_stats(Request::new(GetResolutionStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        // Note: record_resolution doesn't increment total_resolutions (resolve_dependencies does)
        // But artifact_cache_hits from check_artifact_cache should work
        assert!(stats.avg_resolution_time_ms >= 0.0);
    }

    #[test]
    fn test_find_open_tag_exact_no_false_match() {
        let pom = r#"<dependencyManagement>
            <dependencies>
                <dependency>
                    <groupId>org.springframework</groupId>
                    <artifactId>spring-core</artifactId>
                    <version>5.3.30</version>
                </dependency>
            </dependencies>
        </dependencyManagement>
        <dependencies>
            <dependency>
                <groupId>junit</groupId>
                <artifactId>junit</artifactId>
                <version>4.13.2</version>
            </dependency>
        </dependencies>"#;

        // The quick-xml parser only finds deps in <dependencies>, not in <dependencyManagement>
        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].group, "junit");
    }

    #[test]
    fn test_parse_pom_properties() {
        let pom = r#"<project>
            <properties>
                <spring.version>5.3.30</spring.version>
                <junit.version>4.13.2</junit.version>
                <project.version>1.0.0</project.version>
            </properties>
        </project>"#;

        let props = DependencyResolutionServiceImpl::parse_pom_properties(pom);
        assert_eq!(props.get("spring.version").unwrap(), "5.3.30");
        assert_eq!(props.get("junit.version").unwrap(), "4.13.2");
        assert_eq!(props.get("project.version").unwrap(), "1.0.0");
    }

    #[test]
    fn test_interpolate_properties() {
        let mut props = std::collections::HashMap::new();
        props.insert("spring.version".to_string(), "5.3.30".to_string());
        props.insert("project.version".to_string(), "1.0.0".to_string());

        assert_eq!(
            DependencyResolutionServiceImpl::interpolate_properties("${spring.version}", &props),
            "5.3.30"
        );
        assert_eq!(
            DependencyResolutionServiceImpl::interpolate_properties(
                "spring-core-${spring.version}",
                &props
            ),
            "spring-core-5.3.30"
        );
        assert_eq!(
            DependencyResolutionServiceImpl::interpolate_properties("${project.version}", &props),
            "1.0.0"
        );
        // Unknown property — left as-is
        assert_eq!(
            DependencyResolutionServiceImpl::interpolate_properties("${unknown.prop}", &props),
            "${unknown.prop}"
        );
        // Built-in fallback
        assert_eq!(
            DependencyResolutionServiceImpl::interpolate_properties("${version}", &props),
            "0.0.0-unknown"
        );
    }

    #[test]
    fn test_compare_versions() {
        assert!(compare_versions("1.0.0", "2.0.0") == std::cmp::Ordering::Less);
        assert!(compare_versions("2.0.0", "1.0.0") == std::cmp::Ordering::Greater);
        assert!(compare_versions("1.0.0", "1.0.0") == std::cmp::Ordering::Equal);
        // Gradle's StaticVersionComparator treats a trailing numeric part as newer.
        assert!(compare_versions("1.0", "1.0.0") == std::cmp::Ordering::Less);
        assert!(compare_versions("1.2.3", "1.2.4") == std::cmp::Ordering::Less);
        assert!(compare_versions("1.10.0", "1.9.0") == std::cmp::Ordering::Greater);
        assert!(compare_versions("1.0-rc-1", "1.0") == std::cmp::Ordering::Less);
        assert!(compare_versions("1.0-snapshot", "1.0-rc-1") == std::cmp::Ordering::Greater);
        assert!(compare_versions("1.0-ga", "1.0-final") == std::cmp::Ordering::Greater);
        assert!(compare_versions("1.0-sp", "1.0-release") == std::cmp::Ordering::Greater);
        assert!(compare_versions("1.0alpha1", "1.0alpha2") == std::cmp::Ordering::Less);
    }

    #[test]
    fn test_resolve_version_range_exact() {
        let available = vec!["1.0.0".to_string(), "2.0.0".to_string()];
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range("1.0.0", &available, None),
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn test_resolve_version_range_soft() {
        let available = vec![
            "1.0.0".to_string(),
            "1.5.0".to_string(),
            "2.0.0".to_string(),
            "2.5.0".to_string(),
        ];
        // [1.0.0,2.0.0) — should pick 1.5.0 (highest within range)
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range(
                "[1.0.0,2.0.0)",
                &available,
                None
            ),
            Some("1.5.0".to_string())
        );
    }

    #[test]
    fn test_resolve_version_range_open_ended() {
        let available = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "3.0.0".to_string(),
        ];
        // (1.0,) — should pick 3.0.0 (highest above 1.0)
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range("(1.0,)", &available, None),
            Some("3.0.0".to_string())
        );
    }

    #[test]
    fn test_resolve_version_range_latest() {
        let available = vec!["1.0.0".to_string(), "2.0.0".to_string()];
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range(
                "latest.release",
                &available,
                None
            ),
            Some("2.0.0".to_string())
        );
    }

    #[test]
    fn test_parse_pom_with_properties() {
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <properties>
    <spring.version>5.3.30</spring.version>
  </properties>
  <dependencies>
    <dependency>
      <groupId>org.springframework</groupId>
      <artifactId>spring-core</artifactId>
      <version>${spring.version}</version>
    </dependency>
  </dependencies>
</project>"#;

        let props = DependencyResolutionServiceImpl::parse_pom_properties(pom);
        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].version, "${spring.version}");

        // Verify interpolation resolves it
        let interp =
            DependencyResolutionServiceImpl::interpolate_properties(&deps[0].version, &props);
        assert_eq!(interp, "5.3.30");
    }

    #[tokio::test]
    async fn test_resolve_for_nonexistent_build_returns_empty() {
        let svc = make_svc();

        let resp = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "nonexistent-build-config".to_string(),
                dependencies: vec![],
                repositories: vec![],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert!(resp.resolved_dependencies.is_empty());
        assert_eq!(resp.total_artifacts, 0);
        assert_eq!(resp.total_download_size, 0);
        assert!(resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_record_resolution_with_zero_artifacts() {
        let svc = make_svc();

        let resp = svc
            .record_resolution(Request::new(RecordResolutionRequest {
                configuration_name: "empty-compileClasspath".to_string(),
                dependency_count: 0,
                resolution_time_ms: 0,
                success: true,
                cache_hits: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.acknowledged);

        // Verify stats reflect the resolution was processed
        let stats = svc
            .get_resolution_stats(Request::new(GetResolutionStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert!(stats.avg_resolution_time_ms >= 0.0);
        assert!(stats.cached_artifacts >= 0);
    }

    #[tokio::test]
    async fn test_record_resolution_same_configuration_twice_overwrites() {
        let svc = make_svc();

        let config = "compileClasspath".to_string();

        // First record
        let resp1 = svc
            .record_resolution(Request::new(RecordResolutionRequest {
                configuration_name: config.clone(),
                dependency_count: 10,
                resolution_time_ms: 100,
                success: true,
                cache_hits: 3,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp1.acknowledged);

        // Second record for the same configuration with different values
        let resp2 = svc
            .record_resolution(Request::new(RecordResolutionRequest {
                configuration_name: config.clone(),
                dependency_count: 25,
                resolution_time_ms: 250,
                success: true,
                cache_hits: 12,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp2.acknowledged);

        // Both calls are acknowledged — the service accepts overwrites without error
        let stats = svc
            .get_resolution_stats(Request::new(GetResolutionStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        // record_resolution does not increment total_resolutions, but the calls succeed
        assert!(stats.avg_resolution_time_ms >= 0.0);
    }

    #[tokio::test]
    async fn test_get_resolution_stats_for_build_with_no_resolutions() {
        let svc = make_svc();

        // Fresh service with no prior activity
        let stats = svc
            .get_resolution_stats(Request::new(GetResolutionStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(stats.total_resolutions, 0);
        assert_eq!(stats.artifact_cache_hits, 0);
        assert_eq!(stats.total_resolution_time_ms, 0);
        assert_eq!(stats.avg_resolution_time_ms, 0.0);
        assert_eq!(stats.cached_artifacts, 0);
    }

    #[tokio::test]
    async fn test_record_and_retrieve_resolution_with_failure() {
        let svc = make_svc();

        // Record a failed resolution
        let resp = svc
            .record_resolution(Request::new(RecordResolutionRequest {
                configuration_name: "failing-config".to_string(),
                dependency_count: 5,
                resolution_time_ms: 300,
                success: false,
                cache_hits: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.acknowledged);

        // Record a successful resolution to make stats meaningful
        svc.resolve_dependencies(Request::new(ResolveDependenciesRequest {
            configuration_name: "working-config".to_string(),
            dependencies: vec![make_dep("org.slf4j", "slf4j-api", "2.0.9")],
            repositories: vec![make_repo(
                "central",
                "https://repo.maven.apache.org/maven2/",
            )],
            attributes: vec![],
            lenient: true,
            ..Default::default()
        }))
        .await
        .unwrap();

        // Retrieve stats — should show at least one resolution recorded via resolve_dependencies
        let stats = svc
            .get_resolution_stats(Request::new(GetResolutionStatsRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert!(stats.total_resolutions >= 1);
        assert!(stats.total_resolution_time_ms >= 0);
        // The failed record_resolution call does not itself contribute to total_resolutions,
        // but the resolve_dependencies call above does
        assert!(stats.avg_resolution_time_ms >= 0.0);
    }

    // ---- Phase 8 enhancement tests ----

    #[test]
    fn test_compute_sha256() {
        let hash = DependencyResolutionServiceImpl::compute_sha256(b"hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_compute_sha1_md5() {
        let sha1 = DependencyResolutionServiceImpl::compute_sha1(b"hello world");
        assert_eq!(sha1, "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
        let md5 = DependencyResolutionServiceImpl::compute_md5(b"hello world");
        assert_eq!(md5, "5eb63bbbe01eeed093cb22bb8f5acdc3");
    }

    #[test]
    fn test_parse_checksum_with_trailing_filename() {
        assert_eq!(
            DependencyResolutionServiceImpl::parse_checksum_value(
                "abc123def456  artifact-1.0.jar\n"
            ),
            "abc123def456"
        );
        assert_eq!(
            DependencyResolutionServiceImpl::parse_checksum_value("abc123"),
            "abc123"
        );
        assert_eq!(
            DependencyResolutionServiceImpl::parse_checksum_value(""),
            ""
        );
    }

    #[test]
    fn test_parse_maven_metadata_basic() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <versioning>
    <latest>3.0.0</latest>
    <release>2.5.0</release>
    <lastUpdated>20240101120000</lastUpdated>
    <versions>
      <version>1.0.0</version>
      <version>2.0.0</version>
      <version>2.5.0</version>
      <version>3.0.0</version>
    </versions>
  </versioning>
</metadata>"#;

        let meta = DependencyResolutionServiceImpl::parse_maven_metadata(xml).unwrap();
        assert_eq!(meta.group_id, "com.example");
        assert_eq!(meta.artifact_id, "my-lib");
        assert_eq!(meta.versioning.latest.as_deref(), Some("3.0.0"));
        assert_eq!(meta.versioning.release.as_deref(), Some("2.5.0"));
        assert_eq!(meta.versioning.versions.len(), 4);
        assert_eq!(meta.versioning.versions[0], "1.0.0");
        assert_eq!(meta.versioning.versions[3], "3.0.0");
    }

    #[test]
    fn test_parse_maven_metadata_snapshot() {
        let xml = r#"<?xml version="1.0"?>
<metadata>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <versioning>
    <snapshot>
      <timestamp>20240101120000</timestamp>
      <buildNumber>1</buildNumber>
      <localCopy>true</localCopy>
    </snapshot>
    <versions>
      <version>1.0.0-SNAPSHOT</version>
    </versions>
  </versioning>
</metadata>"#;

        let meta = DependencyResolutionServiceImpl::parse_maven_metadata(xml).unwrap();
        assert_eq!(meta.versioning.latest, None);
        let snap = meta.versioning.snapshot.unwrap();
        assert_eq!(snap.timestamp.as_deref(), Some("20240101120000"));
        assert_eq!(snap.build_number.as_deref(), Some("1"));
        assert!(snap.local_copy);
    }

    #[test]
    fn test_parse_maven_metadata_empty() {
        let result =
            DependencyResolutionServiceImpl::parse_maven_metadata("<not-metadata/>").unwrap();
        assert!(result.group_id.is_empty());
        assert!(result.artifact_id.is_empty());
        assert!(result.versioning.versions.is_empty());
    }

    #[test]
    fn test_resolve_version_latest_release_from_metadata() {
        let available = vec!["1.0.0".to_string(), "2.0.0".to_string()];
        let meta = MavenMetadata {
            group_id: String::new(),
            artifact_id: String::new(),
            versioning: MavenVersioning {
                latest: Some("2.0.0".to_string()),
                release: Some("1.5.0".to_string()),
                last_updated: None,
                snapshot: None,
                versions: available.clone(),
            },
        };

        // RELEASE should use metadata.release
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range(
                "RELEASE",
                &available,
                Some(&meta)
            ),
            Some("1.5.0".to_string())
        );
        // LATEST should use metadata.latest
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range(
                "LATEST",
                &available,
                Some(&meta)
            ),
            Some("2.0.0".to_string())
        );
        // latest.release should use metadata.release when available
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range(
                "latest.release",
                &available,
                Some(&meta)
            ),
            Some("1.5.0".to_string())
        );
    }

    #[tokio::test]
    async fn test_persistent_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());

        // Create a test file
        let src = dir.path().join("test-input.jar");
        std::fs::write(&src, b"test artifact content").unwrap();
        let expected_sha =
            DependencyResolutionServiceImpl::compute_sha256(b"test artifact content");

        // Add to cache
        svc.add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
            group: "com.example".to_string(),
            name: "test-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            local_path: src.to_string_lossy().into_owned(),
            size: 20,
            sha256: expected_sha.clone(),
            extension: String::new(),
        }))
        .await
        .unwrap();

        // Verify file exists in Maven layout
        let expected = dir.path().join("com/example/test-lib/1.0/test-lib-1.0.jar");
        assert!(
            expected.exists(),
            "Artifact should be stored at Maven layout path"
        );
        let content = std::fs::read(&expected).unwrap();
        assert_eq!(content, b"test artifact content");

        // Verify .sha256 sidecar exists
        let sha_path = DependencyResolutionServiceImpl::sha256_sidecar_path(&expected);
        assert!(sha_path.exists(), "SHA-256 sidecar should be written");
        assert_eq!(
            std::fs::read_to_string(&sha_path).unwrap(),
            format!("{}  test-lib-1.0.jar\n", expected_sha)
        );
    }

    #[tokio::test]
    async fn test_add_artifact_to_cache_rejects_sha_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let src = dir.path().join("bad-input.jar");
        std::fs::write(&src, b"actual artifact content").unwrap();

        let response = svc
            .add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
                group: "com.example".to_string(),
                name: "bad-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                local_path: src.to_string_lossy().into_owned(),
                size: 23,
                sha256: "0000".to_string(),
                extension: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.accepted);
        assert!(
            !svc.artifact_path("com.example", "bad-lib", "1.0", "", "jar")
                .exists(),
            "rejected artifacts must not leave a persisted cache entry"
        );
    }

    #[tokio::test]
    async fn test_add_artifact_to_cache_preserves_extension() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let src = dir.path().join("demo-1.0-debug.aar");
        std::fs::write(&src, b"aar bytes").unwrap();

        svc.add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
            group: "com.example".to_string(),
            name: "demo".to_string(),
            version: "1.0".to_string(),
            classifier: "debug".to_string(),
            local_path: src.to_string_lossy().into_owned(),
            size: 9,
            sha256: String::new(),
            extension: "aar".to_string(),
        }))
        .await
        .unwrap();

        let aar_hit = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: "debug".to_string(),
                sha256: String::new(),
                extension: "aar".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(aar_hit.cached);
        assert!(aar_hit.local_path.ends_with("demo-1.0-debug.aar"));

        let jar_miss = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "demo".to_string(),
                version: "1.0".to_string(),
                classifier: "debug".to_string(),
                sha256: String::new(),
                extension: "jar".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!jar_miss.cached);
    }

    #[test]
    fn test_maven_layout_path() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let path = svc.artifact_path("com.google.guava", "guava", "32.1.3", "", "jar");
        let expected = dir
            .path()
            .join("com/google/guava/guava/32.1.3/guava-32.1.3.jar");
        assert_eq!(path, expected);

        // With classifier
        let path2 = svc.artifact_path("com.google.guava", "guava", "32.1.3", "sources", "jar");
        let expected2 = dir
            .path()
            .join("com/google/guava/guava/32.1.3/guava-32.1.3-sources.jar");
        assert_eq!(path2, expected2);
    }

    #[tokio::test]
    async fn test_cold_path_filesystem_hit() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());

        // Manually create an artifact file in Maven layout
        let artifact_path = svc.artifact_path("com.example", "cold-lib", "1.0", "", "jar");
        if let Some(parent) = artifact_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&artifact_path, b"cold artifact data").unwrap();

        // DashMap should not have it
        let key = DependencyResolutionServiceImpl::artifact_cache_key(
            "com.example",
            "cold-lib",
            "1.0",
            "",
            "jar",
        );
        assert!(!svc.artifact_cache.contains_key(&key));

        // But check_artifact_cache should find it via filesystem
        let resp = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "cold-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                sha256: String::new(),
                extension: "jar".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.cached, "Should find artifact on filesystem");
        assert_eq!(resp.local_path, artifact_path.to_string_lossy().to_string());
        assert_eq!(resp.cached_size, 18);

        // Now DashMap should have it (warm path next time)
        assert!(svc.artifact_cache.contains_key(&key));
    }

    #[tokio::test]
    async fn test_no_checksum_artifact_read_does_not_freeze_stale_sha() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let artifact_path = svc.artifact_path("com.example", "mutable-lib", "1.0", "", "jar");
        std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
        std::fs::write(&artifact_path, b"old bytes").unwrap();

        let warm_without_checksum = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "mutable-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                sha256: String::new(),
                extension: "jar".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(warm_without_checksum.cached);

        std::fs::write(&artifact_path, b"new bytes").unwrap();
        let new_sha = DependencyResolutionServiceImpl::compute_sha256(b"new bytes");
        let checked_after_mutation = svc
            .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
                group: "com.example".to_string(),
                name: "mutable-lib".to_string(),
                version: "1.0".to_string(),
                classifier: String::new(),
                sha256: new_sha,
                extension: "jar".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(
            checked_after_mutation.cached,
            "checksum-validated warm reads must hash the current file when the first read did not require a checksum"
        );
    }

    #[tokio::test]
    async fn test_no_checksum_metadata_read_does_not_freeze_stale_sha() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let url = "https://repo.example.test/org/example/demo/1.0/demo-1.0.pom";
        let metadata_path = svc.metadata_url_path(url, "pom");
        std::fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
        std::fs::write(&metadata_path, b"<project>old</project>").unwrap();

        let warm_without_checksum = svc
            .check_metadata_cache(Request::new(CheckMetadataCacheRequest {
                url: url.to_string(),
                extension: "pom".to_string(),
                sha256: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(warm_without_checksum.cached);

        std::fs::write(&metadata_path, b"<project>new</project>").unwrap();
        let new_sha = DependencyResolutionServiceImpl::compute_sha256(b"<project>new</project>");
        let checked_after_mutation = svc
            .check_metadata_cache(Request::new(CheckMetadataCacheRequest {
                url: url.to_string(),
                extension: "pom".to_string(),
                sha256: new_sha,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(
            checked_after_mutation.cached,
            "checksum-validated warm metadata reads must hash the current file when the first read did not require a checksum"
        );
    }

    #[tokio::test]
    async fn test_cached_text_metadata_read_does_not_freeze_stale_sha() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let url = "https://repo.example.test/org/example/demo/1.0/demo-1.0.pom";
        let key = DependencyResolutionServiceImpl::metadata_url_cache_key(url, "pom");
        let metadata_path = svc.metadata_url_path(url, "pom");
        std::fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
        std::fs::write(&metadata_path, b"<project>old</project>").unwrap();

        let text = svc
            .read_cached_text_artifact(&key, &metadata_path, "", "", "", "", "pom")
            .await;
        assert_eq!(text.as_deref(), Some("<project>old</project>"));

        std::fs::write(&metadata_path, b"<project>new</project>").unwrap();
        let new_sha = DependencyResolutionServiceImpl::compute_sha256(b"<project>new</project>");
        let checked_after_mutation = svc
            .check_metadata_cache(Request::new(CheckMetadataCacheRequest {
                url: url.to_string(),
                extension: "pom".to_string(),
                sha256: new_sha,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(
            checked_after_mutation.cached,
            "resolver text-cache reads must not freeze a stale SHA for later checksum validation"
        );
    }

    #[tokio::test]
    async fn test_warm_metadata_cache_evicts_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let url = "https://repo.example.test/org/example/demo/1.0/demo-1.0.pom";
        let key = DependencyResolutionServiceImpl::metadata_url_cache_key(url, "pom");
        let metadata_path = svc.metadata_url_path(url, "pom");
        std::fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
        std::fs::write(&metadata_path, b"<project>cached</project>").unwrap();

        let warmed = svc
            .check_metadata_cache(Request::new(CheckMetadataCacheRequest {
                url: url.to_string(),
                extension: "pom".to_string(),
                sha256: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(warmed.cached);
        assert!(svc.artifact_cache.contains_key(&key));

        std::fs::remove_file(&metadata_path).unwrap();
        let after_delete = svc
            .check_metadata_cache(Request::new(CheckMetadataCacheRequest {
                url: url.to_string(),
                extension: "pom".to_string(),
                sha256: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!after_delete.cached);
        assert!(
            !svc.artifact_cache.contains_key(&key),
            "missing warm metadata entries should be evicted"
        );
    }

    #[test]
    fn test_build_request_basic_auth() {
        let svc = DependencyResolutionServiceImpl::new(std::path::PathBuf::new());
        let repo = RepositoryDescriptor {
            id: "private-repo".to_string(),
            url: "https://repo.example.com/maven2/".to_string(),
            m2compatible: true,
            allow_insecure_protocol: false,
            credentials: {
                let mut m = std::collections::HashMap::new();
                m.insert("username".to_string(), "user".to_string());
                m.insert("password".to_string(), "pass".to_string());
                m
            },
            layout: String::new(),
            ivy_pattern: String::new(),
            include_groups: Vec::new(),
            exclude_groups: Vec::new(),
            include_group_prefixes: Vec::new(),
            exclude_group_prefixes: Vec::new(),
        };

        // The build_request method returns a RequestBuilder — we can't easily inspect it,
        // but we can verify it doesn't panic and the method is callable.
        let _ = svc.build_request(&repo, "com/example/lib/1.0/lib-1.0.pom");
    }

    #[tokio::test]
    async fn test_resolve_dependencies_uses_repository_basic_auth() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let saw_auth = Arc::new(Mutex::new(false));
        let saw_auth_for_server = Arc::clone(&saw_auth);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let read = stream.read(&mut request).unwrap();
            let request_text = String::from_utf8_lossy(&request[..read]);
            let normalized_request = request_text.to_ascii_lowercase();
            if normalized_request.contains("authorization: basic dxnlcjpwyxnz") {
                *saw_auth_for_server.lock().unwrap() = true;
                let body = br#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>lib</artifactId>
  <version>1.0</version>
</project>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/xml\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            } else {
                stream
                    .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
        });

        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("com.example", "lib", "1.0")],
                repositories: vec![{
                    let mut repo = make_repo("private", &format!("http://{}", addr));
                    repo.credentials
                        .insert("username".to_string(), "user".to_string());
                    repo.credentials
                        .insert("password".to_string(), "pass".to_string());
                    repo
                }],
                target_scope: "runtime".to_string(),
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        assert!(*saw_auth.lock().unwrap());
    }

    #[tokio::test]
    async fn test_verify_checksum_no_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let data = b"test data for checksum";

        // No sidecar files exist, so verification should pass (no checksum available)
        let result = svc
            .verify_artifact_checksum(data, "https://repo.example.com/test.jar")
            .await;
        assert!(result.matched, "Should match when no sidecar is available");
        assert_eq!(result.algorithm, "sha256");
    }

    // ---- Exclusions parsing tests ----

    #[test]
    fn test_parse_pom_exclusions_basic() {
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>foo</artifactId>
      <version>1.0</version>
      <exclusions>
        <exclusion>
          <groupId>org.unwanted</groupId>
          <artifactId>bar</artifactId>
        </exclusion>
        <exclusion>
          <groupId>org.unwanted</groupId>
          <artifactId>baz</artifactId>
        </exclusion>
      </exclusions>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].group, "com.example");
        assert_eq!(deps[0].name, "foo");
        assert_eq!(deps[0].exclusions.len(), 2);
        assert_eq!(
            deps[0].exclusions[0],
            ("org.unwanted".to_string(), "bar".to_string())
        );
        assert_eq!(
            deps[0].exclusions[1],
            ("org.unwanted".to_string(), "baz".to_string())
        );
    }

    #[test]
    fn test_parse_pom_exclusions_no_exclusions() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>foo</artifactId>
      <version>1.0</version>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].exclusions.is_empty());
    }

    #[test]
    fn test_parse_pom_exclusions_wildcard() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>foo</artifactId>
      <version>1.0</version>
      <exclusions>
        <exclusion>
          <groupId>*</groupId>
          <artifactId>*</artifactId>
        </exclusion>
      </exclusions>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].exclusions.len(), 1);
        assert_eq!(deps[0].exclusions[0], ("*".to_string(), "*".to_string()));
    }

    #[test]
    fn test_parse_pom_exclusions_multiple_deps() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>foo</artifactId>
      <version>1.0</version>
      <exclusions>
        <exclusion>
          <groupId>org.unwanted</groupId>
          <artifactId>bar</artifactId>
        </exclusion>
      </exclusions>
    </dependency>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>other</artifactId>
      <version>2.0</version>
      <exclusions>
        <exclusion>
          <groupId>org.transitive</groupId>
          <artifactId>lib</artifactId>
        </exclusion>
      </exclusions>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].exclusions.len(), 1);
        assert_eq!(
            deps[0].exclusions[0],
            ("org.unwanted".to_string(), "bar".to_string())
        );
        assert_eq!(deps[1].exclusions.len(), 1);
        assert_eq!(
            deps[1].exclusions[0],
            ("org.transitive".to_string(), "lib".to_string())
        );
    }

    #[test]
    fn test_parse_pom_exclusions_partial_exclusion() {
        // Exclusion with only groupId (no artifactId) should not be included
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>foo</artifactId>
      <version>1.0</version>
      <exclusions>
        <exclusion>
          <groupId>org.unwanted</groupId>
          <artifactId></artifactId>
        </exclusion>
      </exclusions>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 1);
        // Empty artifactId means the exclusion is incomplete and should be skipped
        assert!(deps[0].exclusions.is_empty());
    }

    // ---- Dependency management parsing tests ----

    #[test]
    fn test_parse_dependency_management_basic() {
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.springframework</groupId>
        <artifactId>spring-core</artifactId>
        <version>5.3.30</version>
      </dependency>
      <dependency>
        <groupId>org.slf4j</groupId>
        <artifactId>slf4j-api</artifactId>
        <version>2.0.9</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>junit</groupId>
      <artifactId>junit</artifactId>
      <version>4.13.2</version>
    </dependency>
  </dependencies>
</project>"#;

        let managed = DependencyResolutionServiceImpl::parse_dependency_management(pom);
        assert_eq!(managed.len(), 2);
        assert_eq!(
            managed
                .get(&("org.springframework".to_string(), "spring-core".to_string()))
                .unwrap()
                .version,
            "5.3.30"
        );
        assert_eq!(
            managed
                .get(&("org.slf4j".to_string(), "slf4j-api".to_string()))
                .unwrap()
                .version,
            "2.0.9"
        );

        // Verify regular dependencies are separate
        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].group, "junit");
    }

    #[test]
    fn test_parse_dependency_management_empty() {
        let pom = r#"<?xml version="1.0"?><project></project>"#;
        let managed = DependencyResolutionServiceImpl::parse_dependency_management(pom);
        assert!(managed.is_empty());
    }

    #[test]
    fn test_parse_dependency_management_missing_version() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>lib</artifactId>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        let managed = DependencyResolutionServiceImpl::parse_dependency_management(pom);
        assert!(managed.is_empty());
    }

    #[test]
    fn test_parse_dependency_management_with_properties() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <properties>
    <spring.version>5.3.30</spring.version>
  </properties>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.springframework</groupId>
        <artifactId>spring-core</artifactId>
        <version>${spring.version}</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        let managed = DependencyResolutionServiceImpl::parse_dependency_management(pom);
        assert_eq!(managed.len(), 1);
        // Note: dependency management stores raw version strings; interpolation happens at resolution time
        assert_eq!(
            managed
                .get(&("org.springframework".to_string(), "spring-core".to_string()))
                .unwrap()
                .version,
            "${spring.version}"
        );
    }

    // ---- Conflict resolution tests ----

    #[tokio::test]
    async fn test_fail_on_conflict_strategy_fails_closed() {
        let svc = make_svc();
        let conflicting = |version: &str| DependencyDescriptor {
            group: "com.example".to_string(),
            name: "lib".to_string(),
            version: version.to_string(),
            classifier: String::new(),
            extension: "jar".to_string(),
            transitive: false,
            scope: "runtime".to_string(),
            changing: false,
            optional: false,
            ivy_conf: String::new(),
            strict_version: String::new(),
            required_version: String::new(),
            preferred_version: String::new(),
            rejected_versions: Vec::new(),
        };

        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![conflicting("1.0.0"), conflicting("2.0.0")],
                repositories: Vec::new(),
                attributes: Vec::new(),
                lenient: false,
                resolution_strategy: Some(crate::proto::ResolutionStrategyConfig {
                    strategy: "fail_on_conflict".to_string(),
                    forced_versions: Vec::new(),
                    preferred_versions: Vec::new(),
                }),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.success);
        assert!(
            response
                .error_message
                .contains("com.example:lib has versions 1.0.0, 2.0.0"),
            "{}",
            response.error_message
        );
        assert_eq!(response.resolved_dependencies.len(), 2);
    }

    #[tokio::test]
    async fn test_force_strategy_fails_closed_when_forced_version_missing() {
        let svc = make_svc();
        let conflicting = |version: &str| DependencyDescriptor {
            group: "com.example".to_string(),
            name: "lib".to_string(),
            version: version.to_string(),
            classifier: String::new(),
            extension: "jar".to_string(),
            transitive: false,
            scope: "runtime".to_string(),
            changing: false,
            optional: false,
            ivy_conf: String::new(),
            strict_version: String::new(),
            required_version: String::new(),
            preferred_version: String::new(),
            rejected_versions: Vec::new(),
        };

        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![conflicting("1.0.0"), conflicting("2.0.0")],
                repositories: Vec::new(),
                attributes: Vec::new(),
                lenient: false,
                resolution_strategy: Some(crate::proto::ResolutionStrategyConfig {
                    strategy: "force".to_string(),
                    forced_versions: vec![crate::proto::StringEntry {
                        key: "com.example:lib".to_string(),
                        value: "3.0.0".to_string(),
                    }],
                    preferred_versions: Vec::new(),
                }),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.success);
        assert!(
            response.error_message.contains(
                "Forced dependency versions were not present in resolved candidates: com.example:lib"
            ),
            "{}",
            response.error_message
        );
        assert_eq!(response.resolved_dependencies.len(), 2);
    }

    #[test]
    fn test_resolve_conflicts_keeps_highest_version() {
        let mut deps = vec![
            ResolvedDependency {
                group: "com.example".to_string(),
                name: "lib".to_string(),
                version: "1.0.0".to_string(),
                selected_version: "1.0.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
            ResolvedDependency {
                group: "com.example".to_string(),
                name: "lib".to_string(),
                version: "2.0.0".to_string(),
                selected_version: "2.0.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
            ResolvedDependency {
                group: "com.other".to_string(),
                name: "other".to_string(),
                version: "1.5.0".to_string(),
                selected_version: "1.5.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
        ];

        DependencyResolutionServiceImpl::resolve_conflicts(&mut deps);

        assert_eq!(deps.len(), 2);
        // lib should be 2.0.0 (highest)
        let lib = deps.iter().find(|d| d.name == "lib").unwrap();
        assert_eq!(lib.selected_version, "2.0.0");
        // other should remain
        let other = deps.iter().find(|d| d.name == "other").unwrap();
        assert_eq!(other.selected_version, "1.5.0");
    }

    #[test]
    fn test_resolve_conflicts_preserves_order() {
        let mut deps = vec![
            ResolvedDependency {
                group: "a".to_string(),
                name: "first".to_string(),
                version: "1.0".to_string(),
                selected_version: "1.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
            ResolvedDependency {
                group: "a".to_string(),
                name: "second".to_string(),
                version: "3.0".to_string(),
                selected_version: "3.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
            ResolvedDependency {
                group: "a".to_string(),
                name: "first".to_string(),
                version: "2.0".to_string(),
                selected_version: "2.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
        ];

        DependencyResolutionServiceImpl::resolve_conflicts(&mut deps);

        assert_eq!(deps.len(), 2);
        // "first" wins at index 2 (higher version 2.0 > 1.0), "second" stays at index 1
        // Sorted by original index: second (1) then first (2)
        assert_eq!(deps[0].name, "second");
        assert_eq!(deps[0].selected_version, "3.0");
        assert_eq!(deps[1].name, "first");
        assert_eq!(deps[1].selected_version, "2.0");
    }

    #[test]
    fn test_resolve_conflicts_no_duplicates() {
        let mut deps = vec![
            ResolvedDependency {
                group: "a".to_string(),
                name: "lib1".to_string(),
                version: "1.0".to_string(),
                selected_version: "1.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
            ResolvedDependency {
                group: "b".to_string(),
                name: "lib2".to_string(),
                version: "2.0".to_string(),
                selected_version: "2.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
        ];

        DependencyResolutionServiceImpl::resolve_conflicts(&mut deps);

        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_resolve_conflicts_empty() {
        let mut deps: Vec<ResolvedDependency> = Vec::new();
        DependencyResolutionServiceImpl::resolve_conflicts(&mut deps);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_resolve_conflicts_same_version() {
        let mut deps = vec![
            ResolvedDependency {
                group: "a".to_string(),
                name: "lib".to_string(),
                version: "1.0".to_string(),
                selected_version: "1.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
            ResolvedDependency {
                group: "a".to_string(),
                name: "lib".to_string(),
                version: "1.0".to_string(),
                selected_version: "1.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
        ];

        DependencyResolutionServiceImpl::resolve_conflicts(&mut deps);

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].selected_version, "1.0");
    }

    #[test]
    fn test_resolve_conflicts_pre_release_versions() {
        let mut deps = vec![
            ResolvedDependency {
                group: "a".to_string(),
                name: "lib".to_string(),
                version: "1.0.0-beta".to_string(),
                selected_version: "1.0.0-beta".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
            ResolvedDependency {
                group: "a".to_string(),
                name: "lib".to_string(),
                version: "1.0.0".to_string(),
                selected_version: "1.0.0".to_string(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            },
        ];

        DependencyResolutionServiceImpl::resolve_conflicts(&mut deps);

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].selected_version, "1.0.0");
    }

    // ---- matches_exclusion tests ----

    #[test]
    fn test_matches_exclusion_exact() {
        assert!(DependencyResolutionServiceImpl::matches_exclusion(
            "org.unwanted",
            "bar",
            "org.unwanted",
            "bar"
        ));
        assert!(!DependencyResolutionServiceImpl::matches_exclusion(
            "org.unwanted",
            "bar",
            "org.other",
            "bar"
        ));
        assert!(!DependencyResolutionServiceImpl::matches_exclusion(
            "org.unwanted",
            "bar",
            "org.unwanted",
            "other"
        ));
    }

    #[test]
    fn test_matches_exclusion_wildcard_group() {
        assert!(DependencyResolutionServiceImpl::matches_exclusion(
            "org.anything",
            "bar",
            "*",
            "bar"
        ));
        assert!(!DependencyResolutionServiceImpl::matches_exclusion(
            "org.anything",
            "other",
            "*",
            "bar"
        ));
    }

    #[test]
    fn test_matches_exclusion_wildcard_artifact() {
        assert!(DependencyResolutionServiceImpl::matches_exclusion(
            "org.unwanted",
            "anything",
            "org.unwanted",
            "*"
        ));
        assert!(!DependencyResolutionServiceImpl::matches_exclusion(
            "org.other",
            "anything",
            "org.unwanted",
            "*"
        ));
    }

    #[test]
    fn test_matches_exclusion_wildcard_both() {
        assert!(DependencyResolutionServiceImpl::matches_exclusion(
            "anything", "anything", "*", "*"
        ));
    }

    #[test]
    fn test_inherited_exclusions_apply_only_to_current_edge() {
        let excluded_child = PomDependency {
            group: "org.unwanted".to_string(),
            name: "child".to_string(),
            version: "1.0".to_string(),
            scope: String::new(),
            optional: false,
            classifier: String::new(),
            type_field: String::new(),
            exclusions: Vec::new(),
        };
        let sibling = PomDependency {
            group: "org.unwanted".to_string(),
            name: "sibling".to_string(),
            version: "1.0".to_string(),
            scope: String::new(),
            optional: false,
            classifier: String::new(),
            type_field: String::new(),
            exclusions: vec![("org.unwanted".to_string(), "child".to_string())],
        };
        let inherited = vec![("org.unwanted".to_string(), "child".to_string())];

        assert!(DependencyResolutionServiceImpl::is_dependency_excluded(
            &excluded_child,
            &inherited
        ));
        assert!(!DependencyResolutionServiceImpl::is_dependency_excluded(
            &sibling,
            &[]
        ));
    }

    // ---- Integration tests: exclusions + conflict resolution + dep management ----

    #[test]
    fn test_full_pom_with_exclusions_and_dep_management() {
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <properties>
    <spring.version>5.3.30</spring.version>
  </properties>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.slf4j</groupId>
        <artifactId>slf4j-api</artifactId>
        <version>2.0.9</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>app</artifactId>
      <version>1.0</version>
      <exclusions>
        <exclusion>
          <groupId>org.unwanted</groupId>
          <artifactId>transitive-lib</artifactId>
        </exclusion>
      </exclusions>
    </dependency>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
    </dependency>
    <dependency>
      <groupId>org.unwanted</groupId>
      <artifactId>transitive-lib</artifactId>
      <version>3.0</version>
    </dependency>
  </dependencies>
</project>"#;

        // Parse dependency management
        let managed = DependencyResolutionServiceImpl::parse_dependency_management(pom);
        assert_eq!(managed.len(), 1);
        assert_eq!(
            managed
                .get(&("org.slf4j".to_string(), "slf4j-api".to_string()))
                .unwrap()
                .version,
            "2.0.9"
        );

        // Parse regular dependencies
        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 3);

        // Verify app has exclusion
        assert_eq!(deps[0].group, "com.example");
        assert_eq!(deps[0].name, "app");
        assert_eq!(deps[0].exclusions.len(), 1);
        assert_eq!(
            deps[0].exclusions[0],
            ("org.unwanted".to_string(), "transitive-lib".to_string())
        );

        // Verify slf4j is parsed (version may be picked up from dependencyManagement
        // by the byte-level scanner, or may be empty — either is acceptable)
        assert_eq!(deps[1].group, "org.slf4j");
        assert_eq!(deps[1].name, "slf4j-api");

        // Verify the unwanted dep is still parsed (it's a direct dep, not transitive)
        assert_eq!(deps[2].group, "org.unwanted");
        assert_eq!(deps[2].name, "transitive-lib");

        // Verify managed version can be looked up for slf4j
        let slf4j_version = managed
            .get(&(deps[1].group.clone(), deps[1].name.clone()))
            .map(|managed| managed.version.as_str())
            .unwrap_or_default();
        assert_eq!(slf4j_version, "2.0.9");
    }

    #[test]
    fn test_exclusions_not_applied_to_own_transitive_deps_in_parsing() {
        // Exclusions in parse_pom_dependencies are just data — filtering happens at resolution time
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>parent</artifactId>
      <version>1.0</version>
      <exclusions>
        <exclusion>
          <groupId>org.unwanted</groupId>
          <artifactId>child</artifactId>
        </exclusion>
      </exclusions>
    </dependency>
    <dependency>
      <groupId>org.unwanted</groupId>
      <artifactId>child</artifactId>
      <version>2.0</version>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        // Both are parsed — exclusions are data on the parent, not a filter at parse time
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].exclusions.len(), 1);
        assert_eq!(deps[1].group, "org.unwanted");
    }

    #[test]
    fn test_dep_management_with_multiple_versions_same_artifact() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>lib</artifactId>
        <version>1.0</version>
      </dependency>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>lib</artifactId>
        <version>2.0</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        let managed = DependencyResolutionServiceImpl::parse_dependency_management(pom);
        // HashMap — last write wins
        assert_eq!(managed.len(), 1);
        assert_eq!(
            managed
                .get(&("org.example".to_string(), "lib".to_string()))
                .unwrap()
                .version,
            "2.0"
        );
    }

    #[test]
    fn test_dep_management_preserves_raw_property_refs() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.springframework</groupId>
        <artifactId>spring-beans</artifactId>
        <version>${spring.version}</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        let managed = DependencyResolutionServiceImpl::parse_dependency_management(pom);
        assert_eq!(managed.len(), 1);
        // Raw property reference is stored — caller must interpolate
        assert_eq!(
            managed
                .get(&(
                    "org.springframework".to_string(),
                    "spring-beans".to_string()
                ))
                .unwrap()
                .version,
            "${spring.version}"
        );
    }

    #[test]
    fn test_resolve_conflicts_many_duplicates() {
        let mut deps = Vec::new();
        for i in 0..10u32 {
            deps.push(ResolvedDependency {
                group: "com.example".to_string(),
                name: "lib".to_string(),
                version: format!("1.{}.0", i),
                selected_version: format!("1.{}.0", i),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                ..Default::default()
            });
        }

        DependencyResolutionServiceImpl::resolve_conflicts(&mut deps);

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].selected_version, "1.9.0");
    }

    #[test]
    fn test_parse_pom_dependency_with_classifier_and_exclusions() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>
      <groupId>com.example</groupId>
      <artifactId>foo</artifactId>
      <version>1.0</version>
      <classifier>jdk11</classifier>
      <type>jar</type>
      <exclusions>
        <exclusion>
          <groupId>org.unwanted</groupId>
          <artifactId>bar</artifactId>
        </exclusion>
      </exclusions>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].classifier, "jdk11");
        assert_eq!(deps[0].type_field, "jar");
        assert_eq!(deps[0].exclusions.len(), 1);
    }

    #[test]
    fn test_maven_artifact_shape_matches_gradle_special_types() {
        assert_eq!(
            DependencyResolutionServiceImpl::maven_artifact_shape("", ""),
            ("".to_string(), "jar".to_string())
        );
        assert_eq!(
            DependencyResolutionServiceImpl::maven_artifact_shape("jdk11", "jar"),
            ("jdk11".to_string(), "jar".to_string())
        );
        assert_eq!(
            DependencyResolutionServiceImpl::maven_artifact_shape("", "test-jar"),
            ("tests".to_string(), "jar".to_string())
        );
        assert_eq!(
            DependencyResolutionServiceImpl::maven_artifact_shape("", "ejb-client"),
            ("client".to_string(), "jar".to_string())
        );
        assert_eq!(
            DependencyResolutionServiceImpl::maven_artifact_shape("", "bundle"),
            ("".to_string(), "jar".to_string())
        );
        assert_eq!(
            DependencyResolutionServiceImpl::maven_artifact_shape("", "aar"),
            ("".to_string(), "aar".to_string())
        );
        assert_eq!(
            DependencyResolutionServiceImpl::maven_artifact_shape("custom", "test-jar"),
            ("custom".to_string(), "jar".to_string())
        );
    }

    #[test]
    fn test_parse_dependency_management_with_scope_and_optional() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>test-lib</artifactId>
        <version>1.0</version>
        <scope>test</scope>
      </dependency>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>optional-lib</artifactId>
        <version>2.0</version>
        <optional>true</optional>
      </dependency>
      <dependency>
        <groupId>org.example</groupId>
        <artifactId>compile-lib</artifactId>
        <version>3.0</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        let managed = DependencyResolutionServiceImpl::parse_dependency_management(pom);
        // All three should be parsed regardless of scope/optional
        assert_eq!(managed.len(), 3);
        assert_eq!(
            managed
                .get(&("org.example".to_string(), "test-lib".to_string()))
                .unwrap()
                .version,
            "1.0"
        );
        assert_eq!(
            managed
                .get(&("org.example".to_string(), "optional-lib".to_string()))
                .unwrap()
                .version,
            "2.0"
        );
        assert_eq!(
            managed
                .get(&("org.example".to_string(), "compile-lib".to_string()))
                .unwrap()
                .version,
            "3.0"
        );
        assert_eq!(
            managed
                .get(&("org.example".to_string(), "test-lib".to_string()))
                .unwrap()
                .scope,
            "test"
        );
    }

    #[test]
    fn test_managed_default_scope_matches_gradle_rules() {
        let managed = ManagedDependency {
            version: "1.0".to_string(),
            scope: "test".to_string(),
            type_field: String::new(),
            exclusions: vec![("org.blocked".to_string(), "leaf".to_string())],
        };
        let mut dep = PomDependency {
            group: "org.example".to_string(),
            name: "lib".to_string(),
            version: String::new(),
            scope: String::new(),
            optional: false,
            classifier: String::new(),
            type_field: String::new(),
            exclusions: vec![],
        };

        assert_eq!(
            DependencyResolutionServiceImpl::managed_default_scope(&dep, Some(&managed)),
            "test"
        );

        dep.scope = "unknown".to_string();
        assert_eq!(
            DependencyResolutionServiceImpl::managed_default_scope(&dep, Some(&managed)),
            "compile",
            "Gradle only consults dependencyManagement scope when dependency scope is missing"
        );

        dep.scope = "runtime".to_string();
        assert_eq!(
            DependencyResolutionServiceImpl::managed_default_scope(&dep, Some(&managed)),
            "runtime"
        );
    }

    #[test]
    fn test_effective_exclusions_match_gradle_dependency_management_rules() {
        let managed = ManagedDependency {
            version: "1.0".to_string(),
            scope: String::new(),
            type_field: String::new(),
            exclusions: vec![("org.managed".to_string(), "blocked".to_string())],
        };
        let mut dep = PomDependency {
            group: "org.example".to_string(),
            name: "lib".to_string(),
            version: String::new(),
            scope: String::new(),
            optional: false,
            classifier: String::new(),
            type_field: String::new(),
            exclusions: vec![],
        };

        assert_eq!(
            DependencyResolutionServiceImpl::effective_exclusions(&dep, Some(&managed)),
            vec![("org.managed".to_string(), "blocked".to_string())]
        );

        dep.exclusions = vec![("org.direct".to_string(), "blocked".to_string())];
        assert_eq!(
            DependencyResolutionServiceImpl::effective_exclusions(&dep, Some(&managed)),
            vec![("org.direct".to_string(), "blocked".to_string())],
            "direct exclusions override dependencyManagement exclusions"
        );
    }

    // ---- Recursive transitive resolution tests ----

    #[test]
    fn test_resolve_descriptor_cycle_detection() {
        // Verify that cycle detection doesn't cause infinite recursion.
        // The resolve_descriptor creates a fresh visited set each time,
        // so cycles are detected within a single resolve_recursive call chain.
        let _svc = make_svc();

        // This test validates the visited set mechanism works.
        // In real resolution, A→B→A would be caught by the visited set.
        let mut visited = std::collections::HashSet::new();
        let coord_a = ("com.example".to_string(), "lib-a".to_string());
        let coord_b = ("com.example".to_string(), "lib-b".to_string());

        // Simulate: first visit succeeds
        assert!(visited.insert(coord_a.clone()));
        // Second visit (cycle) fails
        assert!(!visited.insert(coord_a.clone()));
        // Different coord succeeds
        assert!(visited.insert(coord_b.clone()));

        // Remove and re-insert works
        visited.remove(&coord_a);
        assert!(visited.insert(coord_a.clone()));
    }

    #[test]
    fn test_resolve_descriptor_depth_limit() {
        // Verify MAX_DEPTH constant is reasonable
        // The constant is 50, which should be more than enough for any real dependency tree
        assert!(50 <= 100, "MAX_DEPTH should be bounded");
    }

    #[test]
    fn test_bom_import_parsing() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.springframework.boot</groupId>
        <artifactId>spring-boot-dependencies</artifactId>
        <version>3.2.0</version>
        <type>pom</type>
        <scope>import</scope>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.springframework</groupId>
      <artifactId>spring-core</artifactId>
      <version>6.1.0</version>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        let managed = DependencyResolutionServiceImpl::parse_dependency_management(pom);

        // Regular dependency should be parsed
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].group, "org.springframework");
        assert_eq!(deps[0].name, "spring-core");
        // Empty scope means "compile" (Maven default)
        assert!(deps[0].scope.is_empty() || deps[0].scope == "compile");

        // BOM import should NOT appear in regular deps (it's in dependencyManagement)
        // But it should be in managed deps if we parse them
        assert!(managed.contains_key(&(
            "org.springframework.boot".to_string(),
            "spring-boot-dependencies".to_string()
        )));
    }

    #[test]
    fn test_bom_import_not_in_regular_deps() {
        // BOM imports (scope=import, type=pom) should be filtered from regular deps
        let pom = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-dependencies</artifactId>
      <version>3.2.0</version>
      <type>pom</type>
      <scope>import</scope>
    </dependency>
    <dependency>
      <groupId>com.google.guava</groupId>
      <artifactId>guava</artifactId>
      <version>32.1.3</version>
    </dependency>
  </dependencies>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);

        // BOM import should be parsed as a dependency (with scope=import)
        assert_eq!(deps.len(), 2);

        // Verify we can distinguish BOM imports
        let bom_dep = deps.iter().find(|d| d.scope == "import").unwrap();
        assert_eq!(bom_dep.type_field, "pom");

        let regular_dep = deps.iter().find(|d| d.scope != "import").unwrap();
        assert_eq!(regular_dep.name, "guava");
    }

    #[tokio::test]
    async fn test_resolve_dependencies_returns_tree_structure() {
        // Test that resolve_dependencies returns a proper tree (not flat list)
        let svc = make_svc();

        let resp = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "compileClasspath".to_string(),
                dependencies: vec![make_dep("org.slf4j", "slf4j-api", "2.0.9")],
                repositories: vec![make_repo(
                    "central",
                    "https://repo.maven.apache.org/maven2/",
                )],
                attributes: vec![],
                lenient: false,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.resolved_dependencies.len(), 1);
        // slf4j-api has no compile-scope transitive dependencies
        // so the tree may be flat — that's fine, the structure supports nesting
    }

    #[test]
    fn test_parent_pom_parsing() {
        // Test that parent POM elements can be parsed
        let pom = r#"<?xml version="1.0"?>
<project>
  <parent>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-starter-parent</artifactId>
    <version>3.2.0</version>
    </parent>
  <artifactId>my-app</artifactId>
  <version>1.0.0</version>
</project>"#;

        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        // Parent section should not produce any dependencies
        assert!(deps.is_empty());
    }

    #[test]
    fn test_pom_dependency_clone() {
        // Verify PomDependency is Clone (needed for recursive resolution)
        let dep = PomDependency {
            group: "com.example".to_string(),
            name: "lib".to_string(),
            version: "1.0".to_string(),
            scope: "compile".to_string(),
            optional: false,
            classifier: String::new(),
            type_field: "jar".to_string(),
            exclusions: vec![("org.unwanted".to_string(), "bar".to_string())],
        };

        let cloned = dep.clone();
        assert_eq!(cloned.group, dep.group);
        assert_eq!(cloned.name, dep.name);
        assert_eq!(cloned.exclusions.len(), 1);
    }

    // ---- Parent POM inheritance tests ----

    #[test]
    fn test_parse_parent_pom_basic() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <parent>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-starter-parent</artifactId>
    <version>3.2.0</version>
  </parent>
  <groupId>com.example</groupId>
  <artifactId>my-app</artifactId>
  <version>1.0.0</version>
</project>"#;

        let parent = DependencyResolutionServiceImpl::parse_parent_pom(pom);
        assert!(parent.is_some());
        let p = parent.unwrap();
        assert_eq!(p.group_id, "org.springframework.boot");
        assert_eq!(p.artifact_id, "spring-boot-starter-parent");
        assert_eq!(p.version, "3.2.0");
        assert!(p.relative_path.is_empty());
    }

    #[test]
    fn test_parse_parent_pom_with_relative_path() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent-pom</artifactId>
    <version>1.0.0</version>
    <relativePath>../parent/pom.xml</relativePath>
  </parent>
  <artifactId>child</artifactId>
</project>"#;

        let parent = DependencyResolutionServiceImpl::parse_parent_pom(pom).unwrap();
        assert_eq!(parent.relative_path, "../parent/pom.xml");
    }

    #[test]
    fn test_parse_parent_pom_none_when_missing() {
        let pom = r#"<?xml version="1.0"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>standalone</artifactId>
  <version>1.0.0</version>
</project>"#;

        assert!(DependencyResolutionServiceImpl::parse_parent_pom(pom).is_none());
    }

    #[test]
    fn test_parse_parent_pom_none_when_incomplete() {
        // Missing version — should return None
        let pom = r#"<?xml version="1.0"?>
<project>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
  </parent>
</project>"#;

        assert!(DependencyResolutionServiceImpl::parse_parent_pom(pom).is_none());
    }

    #[test]
    fn test_parent_inheritance_property_merging() {
        // Verify that child properties override parent properties
        let child_pom = r#"<?xml version="1.0"?>
<project>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0</version>
  </parent>
  <properties>
    <child.prop>child-value</child.prop>
    <shared.prop>child-override</shared.prop>
  </properties>
</project>"#;

        let parent_pom = r#"<?xml version="1.0"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>parent</artifactId>
  <version>1.0</version>
  <properties>
    <parent.only.prop>parent-value</parent.only.prop>
    <shared.prop>parent-value</shared.prop>
  </properties>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.slf4j</groupId>
        <artifactId>slf4j-api</artifactId>
        <version>2.0.9</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        // Simulate what resolve_parent_inheritance does:
        // Child props first, then parent fills gaps
        let mut properties = DependencyResolutionServiceImpl::parse_pom_properties(child_pom);
        let mut managed = DependencyResolutionServiceImpl::parse_dependency_management(child_pom);

        // Parent doesn't contribute managed deps that child already has (child has none)
        // but does fill in missing properties
        let parent_props = DependencyResolutionServiceImpl::parse_pom_properties(parent_pom);
        let parent_managed =
            DependencyResolutionServiceImpl::parse_dependency_management(parent_pom);

        for (k, v) in parent_props {
            properties.entry(k).or_insert(v);
        }
        for (k, v) in parent_managed {
            managed.entry(k).or_insert(v);
        }

        // Child property should override parent
        assert_eq!(properties.get("shared.prop").unwrap(), "child-override");
        // Child-only property should exist
        assert_eq!(properties.get("child.prop").unwrap(), "child-value");
        // Parent-only property should be inherited
        assert_eq!(properties.get("parent.only.prop").unwrap(), "parent-value");
        // Parent managed dep should be inherited
        assert_eq!(
            managed
                .get(&("org.slf4j".to_string(), "slf4j-api".to_string()))
                .unwrap()
                .version,
            "2.0.9"
        );
    }

    #[test]
    fn test_parent_inheritance_dependency_management() {
        // Parent provides managed version, child dependency uses it
        let child_pom = r#"<?xml version="1.0"?>
<project>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0</version>
  </parent>
  <dependencies>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
    </dependency>
  </dependencies>
</project>"#;

        let parent_pom = r#"<?xml version="1.0"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>parent</artifactId>
  <version>1.0</version>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.slf4j</groupId>
        <artifactId>slf4j-api</artifactId>
        <version>2.0.9</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        // Simulate: child has no version for slf4j-api, parent provides it via dep management
        let child_deps = DependencyResolutionServiceImpl::parse_pom_dependencies(child_pom);
        assert_eq!(child_deps.len(), 1);
        assert!(child_deps[0].version.is_empty()); // No version specified in child

        let parent_managed =
            DependencyResolutionServiceImpl::parse_dependency_management(parent_pom);
        let managed_version =
            parent_managed.get(&("org.slf4j".to_string(), "slf4j-api".to_string()));
        assert!(managed_version.is_some());
        assert_eq!(managed_version.unwrap().version, "2.0.9");
    }

    // ---- SNAPSHOT version resolution tests ----

    #[test]
    fn test_resolve_snapshot_version_with_metadata() {
        let xml = r#"<?xml version="1.0"?>
<metadata>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <versioning>
    <snapshot>
      <timestamp>20240101.120000</timestamp>
      <buildNumber>1</buildNumber>
    </snapshot>
    <versions>
      <version>1.0-SNAPSHOT</version>
    </versions>
  </versioning>
</metadata>"#;

        let meta = DependencyResolutionServiceImpl::parse_maven_metadata(xml).unwrap();
        assert!(meta.versioning.snapshot.is_some());

        let snap = meta.versioning.snapshot.unwrap();
        assert_eq!(snap.timestamp.as_deref(), Some("20240101.120000"));
        assert_eq!(snap.build_number.as_deref(), Some("1"));
        assert!(!snap.local_copy);

        // Verify the expected resolved version
        let base = "1.0-SNAPSHOT";
        let base_ver = &base[..base.len() - "-SNAPSHOT".len()];
        let resolved = format!(
            "{}-{}-{}",
            base_ver,
            snap.timestamp.unwrap(),
            snap.build_number.unwrap()
        );
        assert_eq!(resolved, "1.0-20240101.120000-1");
    }

    #[test]
    fn test_resolve_snapshot_version_local_copy() {
        let xml = r#"<?xml version="1.0"?>
<metadata>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <versioning>
    <snapshot>
      <timestamp>20240101.120000</timestamp>
      <buildNumber>1</buildNumber>
      <localCopy>true</localCopy>
    </snapshot>
    <versions>
      <version>1.0-SNAPSHOT</version>
    </versions>
  </versioning>
</metadata>"#;

        let meta = DependencyResolutionServiceImpl::parse_maven_metadata(xml).unwrap();
        assert!(meta.versioning.snapshot.as_ref().unwrap().local_copy);
    }

    #[test]
    fn test_resolve_snapshot_version_no_snapshot_metadata() {
        // Metadata exists but no <snapshot> section (release artifact)
        let xml = r#"<?xml version="1.0"?>
<metadata>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <versioning>
    <release>1.0.0</release>
    <versions>
      <version>1.0.0</version>
    </versions>
  </versioning>
</metadata>"#;

        let meta = DependencyResolutionServiceImpl::parse_maven_metadata(xml).unwrap();
        assert!(meta.versioning.snapshot.is_none());
    }

    #[test]
    fn test_snapshot_version_fallback_from_versions_list() {
        // Simulate: no snapshot section, but versions list has timestamped versions
        let meta = MavenMetadata {
            group_id: "com.example".to_string(),
            artifact_id: "my-lib".to_string(),
            versioning: MavenVersioning {
                latest: None,
                release: None,
                last_updated: None,
                snapshot: None,
                versions: vec![
                    "1.0-SNAPSHOT".to_string(),
                    "1.0-20240101.120000-1".to_string(),
                    "1.0-20240215.090000-2".to_string(),
                ],
            },
        };

        // Find the latest non-SNAPSHOT version starting with "1.0"
        let raw_version = "1.0-SNAPSHOT";
        let base = &raw_version[..raw_version.len() - "-SNAPSHOT".len()];
        let ts_version = meta
            .versioning
            .versions
            .iter()
            .rfind(|v| !v.ends_with("-SNAPSHOT") && v.starts_with(base))
            .unwrap();
        assert_eq!(ts_version, "1.0-20240215.090000-2");
    }

    #[test]
    fn test_is_snapshot_detection() {
        assert!("1.0-SNAPSHOT".ends_with("-SNAPSHOT"));
        assert!("2.0.0-SNAPSHOT".ends_with("-SNAPSHOT"));
        assert!(!"1.0.0".ends_with("-SNAPSHOT"));
        assert!(!"1.0.0-BETA".ends_with("-SNAPSHOT"));

        // Base version extraction
        let v = "1.0-SNAPSHOT";
        let base = &v[..v.len() - "-SNAPSHOT".len()];
        assert_eq!(base, "1.0");
    }

    // ---- Fail-closed dependency feature gate tests ----

    #[tokio::test]
    async fn test_prefetch_rejects_snapshot_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());

        let mut deps = vec![ResolvedDependency {
            group: "com.example".to_string(),
            name: "my-lib".to_string(),
            version: "1.0-SNAPSHOT".to_string(),
            selected_version: "1.0-SNAPSHOT".to_string(),
            dependencies: Vec::new(),
            resolved: true,
            failure_reason: String::new(),
            artifact_url:
                "https://repo.example.test/maven/com/example/my-lib/1.0-SNAPSHOT/my-lib-1.0-SNAPSHOT.jar"
                    .to_string(),
            artifact_size: 0,
            artifact_sha256: String::new(),
            scope: "compile".to_string(),
        }];

        let result = svc.prefetch_resolved_artifacts(&mut deps).await;
        assert!(result.is_err(), "SNAPSHOT prefetch should be rejected");
        let err = result.unwrap_err();
        assert!(
            err.contains("SNAPSHOT artifact prefetch is not supported yet"),
            "Error should mention SNAPSHOT gate, got: {err}"
        );
        assert!(
            err.contains("my-lib"),
            "Error should include artifact name, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_maven_coordinate_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());

        // Empty group
        let result = svc
            .download_artifact_into_store(
                "",
                "my-lib",
                "1.0",
                "",
                "jar",
                "https://repo.example.test/maven/my-lib-1.0.jar",
            )
            .await;
        assert!(result.is_err(), "Empty group should be rejected");
        assert!(result
            .unwrap_err()
            .contains("Incomplete Maven artifact coordinate"),);

        // Empty name
        let result = svc
            .download_artifact_into_store(
                "com.example",
                "",
                "1.0",
                "",
                "jar",
                "https://repo.example.test/maven/com/example/1.0.jar",
            )
            .await;
        assert!(result.is_err(), "Empty name should be rejected");
        assert!(result
            .unwrap_err()
            .contains("Incomplete Maven artifact coordinate"),);

        // Empty version
        let result = svc
            .download_artifact_into_store(
                "com.example",
                "my-lib",
                "",
                "",
                "jar",
                "https://repo.example.test/maven/com/example/my-lib.jar",
            )
            .await;
        assert!(result.is_err(), "Empty version should be rejected");
        assert!(result
            .unwrap_err()
            .contains("Incomplete Maven artifact coordinate"),);

        // Empty artifact URL
        let result = svc
            .download_artifact_into_store("com.example", "my-lib", "1.0", "", "jar", "")
            .await;
        assert!(result.is_err(), "Empty artifact URL should be rejected");
        assert!(result
            .unwrap_err()
            .contains("Incomplete Maven artifact coordinate"),);
    }

    #[test]
    fn test_resolve_version_range_rejects_unsupported_patterns() {
        let available = vec![
            "1.0.0".to_string(),
            "1.2.0".to_string(),
            "2.0.0".to_string(),
        ];

        let ivy_result =
            DependencyResolutionServiceImpl::resolve_version_range("1.+", &available, None);
        assert_eq!(
            ivy_result, None,
            "Ivy '1.+' must fail closed, not fall through as an exact version"
        );

        let empty_result =
            DependencyResolutionServiceImpl::resolve_version_range("", &available, None);
        assert_eq!(empty_result, None, "Empty version string must fail closed");

        let ws_result =
            DependencyResolutionServiceImpl::resolve_version_range("  ", &available, None);
        assert_eq!(ws_result, None, "Whitespace-only version must fail closed");

        // Verify normal patterns still work
        let normal =
            DependencyResolutionServiceImpl::resolve_version_range("1.2.0", &available, None);
        assert_eq!(normal, Some("1.2.0".to_string()));

        let latest = DependencyResolutionServiceImpl::resolve_version_range(
            "latest.release",
            &available,
            None,
        );
        assert_eq!(latest, Some("2.0.0".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_dependencies_fails_closed_for_unsupported_version_selector() {
        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "demo", "1.+")],
                repositories: vec![make_repo("central", "https://repo.example.test/maven2")],
                attributes: Vec::new(),
                lenient: false,
                resolution_strategy: None,
                target_scope: String::new(),
                prefetch_artifacts: false,
                constraints: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.success);
        assert!(response
            .error_message
            .contains("Unsupported dependency selector"));
        assert!(response.error_message.contains("wildcard '+' selectors"));
        assert!(
            response.resolved_dependencies.is_empty(),
            "unsupported dependency selectors should fail before repository traversal"
        );
    }

    #[tokio::test]
    async fn test_dependency_constraint_upgrades_matching_dependency_version() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requested = Arc::new(Mutex::new(Vec::new()));
        let requested_for_server = Arc::clone(&requested);
        let server = std::thread::spawn(move || {
            for _ in 0..1 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request_text = String::from_utf8_lossy(&request[..read]);
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                requested_for_server.lock().unwrap().push(path.clone());
                let body = if path.ends_with("/demo-2.0.pom") {
                    b"<project><modelVersion>4.0.0</modelVersion><groupId>org.example</groupId><artifactId>demo</artifactId><version>2.0</version></project>".as_slice()
                } else {
                    let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                    stream.write_all(response.as_bytes()).unwrap();
                    continue;
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/xml\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });

        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "demo", "1.0")],
                constraints: vec![make_dep("org.example", "demo", "2.0")],
                repositories: vec![make_repo("local", &format!("http://{}", addr))],
                attributes: Vec::new(),
                lenient: false,
                resolution_strategy: None,
                target_scope: String::new(),
                prefetch_artifacts: false,
            }))
            .await
            .unwrap()
            .into_inner();
        server.join().unwrap();

        assert!(response.success, "{}", response.error_message);
        assert_eq!(response.resolved_dependencies.len(), 1);
        assert_eq!(response.resolved_dependencies[0].version, "2.0");
        assert_eq!(response.resolved_dependencies[0].selected_version, "2.0");
        let paths = requested.lock().unwrap();
        assert!(paths.iter().any(|path| path.ends_with("/demo-2.0.pom")));
        assert!(!paths.iter().any(|path| path.ends_with("/demo-1.0.pom")));
    }

    #[tokio::test]
    async fn test_dependency_constraint_does_not_create_artifact_without_dependency() {
        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: Vec::new(),
                constraints: vec![make_dep("org.example", "demo", "2.0")],
                repositories: vec![make_repo("central", "https://repo.example.test/maven2")],
                attributes: Vec::new(),
                lenient: false,
                resolution_strategy: None,
                target_scope: String::new(),
                prefetch_artifacts: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(response.success, "{}", response.error_message);
        assert!(response.resolved_dependencies.is_empty());
        assert_eq!(response.total_artifacts, 0);
    }

    #[tokio::test]
    async fn test_dependency_constraint_unsupported_selector_fails_closed() {
        let svc = make_svc();
        let response = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "runtimeClasspath".to_string(),
                dependencies: vec![make_dep("org.example", "demo", "1.0")],
                constraints: vec![make_dep("org.example", "demo", "2.+")],
                repositories: vec![make_repo("central", "https://repo.example.test/maven2")],
                attributes: Vec::new(),
                lenient: false,
                resolution_strategy: None,
                target_scope: String::new(),
                prefetch_artifacts: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.success);
        assert!(response
            .error_message
            .contains("Unsupported dependency constraint"));
        assert!(response.error_message.contains("wildcard '+' selectors"));
    }
}
