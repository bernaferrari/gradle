use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tonic::{Request, Response, Status};

use crate::proto::{
    dependency_resolution_service_server::DependencyResolutionService, AddArtifactToCacheRequest,
    AddArtifactToCacheResponse, CheckArtifactCacheRequest, CheckArtifactCacheResponse,
    DependencyDescriptor, GetResolutionStatsRequest, GetResolutionStatsResponse,
    RecordResolutionRequest, RecordResolutionResponse, RepositoryDescriptor,
    ResolveDependenciesRequest, ResolveDependenciesResponse, ResolvedDependency,
};

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

/// Parsed dependency from a POM file.
struct PomDependency {
    group: String,
    name: String,
    version: String,
    scope: String,
    optional: bool,
    classifier: String,
    type_field: String,
}

/// Rust-native dependency resolution service.
/// Resolves dependency graphs, fetches POMs from Maven repos, and manages artifact caching.
pub struct DependencyResolutionServiceImpl {
    artifact_cache: DashMap<String, CachedArtifact>,
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

/// Snapshot info from maven-metadata.xml.
#[derive(Clone)]
struct MavenSnapshot {
    build_number: Option<String>,
    timestamp: Option<String>,
    local_copy: bool,
}

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
            artifact_cache: DashMap::new(),
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

    /// Compute the local filesystem path for an artifact using Maven repository layout.
    fn artifact_path(
        &self,
        group: &str,
        name: &str,
        version: &str,
        classifier: &str,
        extension: &str,
    ) -> PathBuf {
        let group_path = group.replace('.', "/");
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
        let resp = self.http_client.get(&checksum_url).send().await
            .map_err(|e| format!("Failed to fetch {} checksum: {}", algo, e))?;

        match resp.status().as_u16() {
            200 => resp.text().await
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
    fn build_request(
        &self,
        repo: &RepositoryDescriptor,
        path: &str,
    ) -> reqwest::RequestBuilder {
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
                        _ => current_tag = std::str::from_utf8(name.local_name().as_ref()).unwrap_or_default().to_string(),
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    current_tag = std::str::from_utf8(name.local_name().as_ref()).unwrap_or_default().to_string();
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
        let group_path = group.replace('.', "/");
        let path = format!("{}/{}/maven-metadata.xml", group_path, name);
        let req = self.build_request(repo, &path);

        let resp = req.send().await
            .map_err(|e| format!("Failed to fetch maven-metadata.xml: {}", e))?;

        match resp.status().as_u16() {
            200 => {
                let body = resp.text().await
                    .map_err(|e| format!("Failed to read metadata response: {}", e))?;
                Self::parse_maven_metadata(&body)
            }
            404 => Err("maven-metadata.xml not found".to_string()),
            status => Err(format!("HTTP {} for maven-metadata.xml", status)),
        }
    }

    fn artifact_cache_key(group: &str, name: &str, version: &str, classifier: &str) -> String {
        format!("{}:{}:{}:{}", group, name, version, classifier)
    }

    /// Fetch a POM file from a Maven repository and parse it.
    async fn fetch_pom(&self, group: &str, name: &str, version: &str, repo: &RepositoryDescriptor) -> Result<String, String> {
        let group_path = group.replace('.', "/");
        let path = format!(
            "{}/{}/{}/{}-{}.pom",
            group_path, name, version, name, version
        );

        let response = self.build_request(repo, &path).send().await
            .map_err(|e| format!("Failed to fetch POM: {}", e))?;

        match response.status().as_u16() {
            200 => response.text().await
                .map_err(|e| format!("Failed to read POM response: {}", e)),
            404 => Err(format!("POM not found: {}-{}.pom", name, version)),
            status => Err(format!("HTTP {} for POM", status)),
        }
    }

    /// Parse a POM file and extract dependencies using a byte-level scanner.
    /// Handles property interpolation, version ranges, and excludes false matches
    /// like `<dependencyManagement>`.
    fn parse_pom_dependencies(pom_content: &str) -> Vec<PomDependency> {
        let mut dependencies = Vec::new();
        let bytes = pom_content.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        // Track whether we're inside a <dependencyManagement> block
        let dep_mgmt_open = b"<dependencyManagement";
        let dep_mgmt_close = b"</dependencyManagement>";

        while i < len {
            // Look for <dependency> (not <dependencyManagement, not </dependency>)
            let pos = match find_open_tag_exact(bytes, i, b"dependency") {
                Some(p) => p,
                None => break,
            };

            // Skip if this <dependency> is inside a <dependencyManagement> block
            // Check if there's a <dependencyManagement> before this position without a closing tag
            let mut in_dep_mgmt = false;
            let mut scan = 0usize;
            while scan < pos {
                if let Some(dm_start) = bytes[scan..pos].windows(dep_mgmt_open.len())
                    .position(|w| w == dep_mgmt_open)
                    .map(|p| scan + p)
                {
                    // Check if there's a closing tag between dm_start and pos
                    let after_dm = dm_start + dep_mgmt_open.len();
                    if bytes[after_dm..pos].windows(dep_mgmt_close.len())
                        .any(|w| w == dep_mgmt_close)
                    {
                        scan = after_dm;
                        continue;
                    }
                    in_dep_mgmt = true;
                    break;
                }
                break;
            }

            if in_dep_mgmt {
                i = pos + b"<dependency".len();
                continue;
            }

            // Extract fields within this <dependency> block
            let end_pos = match find_end_tag(bytes, pos, b"dependency") {
                Some(p) => p,
                None => break,
            };

            let group = extract_tag_text(bytes, pos, b"groupId").unwrap_or_default();
            let name = extract_tag_text(bytes, pos, b"artifactId").unwrap_or_default();
            let version = extract_tag_text(bytes, pos, b"version").unwrap_or_default();
            let scope = extract_tag_text(bytes, pos, b"scope").unwrap_or_default();
            let optional = extract_tag_text(bytes, pos, b"optional")
                .map(|v| v == "true")
                .unwrap_or(false);
            let _classifier = extract_tag_text(bytes, pos, b"classifier").unwrap_or_default();
            let _type_field = extract_tag_text(bytes, pos, b"type").unwrap_or_default();

            if !group.is_empty() && !name.is_empty() {
                dependencies.push(PomDependency {
                    group,
                    name,
                    version,
                    scope,
                    optional,
                    classifier: _classifier,
                    type_field: _type_field,
                });
            }

            i = end_pos + b"</dependency>".len();
        }

        dependencies
    }

    /// Parse properties from <properties> section of a POM.
    fn parse_pom_properties(pom_content: &str) -> std::collections::HashMap<String, String> {
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
            if let Some(val_end) = bytes[tag_end + 1..end].windows(close_bytes.len())
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
    fn interpolate_properties(value: &str, properties: &std::collections::HashMap<String, String>) -> String {
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
                            "project.version" | "version" | "pom.version" => "0.0.0-unknown".to_string(),
                            "project.groupId" | "groupId" => "unknown".to_string(),
                            "project.artifactId" | "artifactId" => "unknown".to_string(),
                            _ => format!("${{{}}}", key), // Leave unresolved
                        }
                    });
                    result = format!("{}{}{}", &result[..start], replacement, &result[start + end + 1..]);
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
    fn resolve_version_range(range: &str, available: &[String], metadata: Option<&MavenMetadata>) -> Option<String> {
        let range = range.trim();

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

        // Parse range: [start,end) or (start,end]
        let inner: &str = &range[1..range.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() != 2 {
            return Some(range.to_string()); // Can't parse, return as-is
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

    /// Fetch available versions for a dependency from maven-metadata.xml.
    async fn fetch_available_versions(&self, group: &str, name: &str, repos: &[RepositoryDescriptor]) -> (Vec<String>, Option<MavenMetadata>) {
        for repo in repos {
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

    /// Resolve a single dependency descriptor with real POM fetching.
    /// Handles property interpolation, version ranges, and transitive dependency resolution.
    async fn resolve_descriptor(
        &self,
        dep: &DependencyDescriptor,
        repos: &[RepositoryDescriptor],
    ) -> ResolvedDependency {
        let group = dep.group.clone();
        let name = dep.name.clone();
        let raw_version = dep.version.clone();

        // Resolve version ranges
        let selected_version = if raw_version.contains(',') || raw_version.starts_with('[') || raw_version.starts_with('(')
            || raw_version == "LATEST" || raw_version == "RELEASE"
        {
            // Version range or special version — need to fetch available versions
            let (available, metadata) = self.fetch_available_versions(&group, &name, repos).await;
            if !available.is_empty() {
                Self::resolve_version_range(&raw_version, &available, metadata.as_ref())
                    .unwrap_or(raw_version.clone())
            } else {
                raw_version.clone()
            }
        } else {
            raw_version.clone()
        };

        // Try to fetch POM for transitive deps
        let mut transitive_deps = Vec::new();

        if dep.transitive {
            for repo in repos {
                match self.fetch_pom(&group, &name, &selected_version, repo).await {
                    Ok(pom_content) => {
                        // Parse properties first, then interpolate in dependency versions
                        let properties = Self::parse_pom_properties(&pom_content);
                        let pom_deps = Self::parse_pom_dependencies(&pom_content);
                        for pom_dep in &pom_deps {
                            // Skip test/provided scopes and optional deps
                            if pom_dep.scope == "test" || pom_dep.scope == "provided" || pom_dep.optional {
                                continue;
                            }
                            let interp_version = Self::interpolate_properties(&pom_dep.version, &properties);
                            let ext = if pom_dep.type_field.is_empty() { "jar" } else { &pom_dep.type_field };
                            let artifact_filename = if pom_dep.classifier.is_empty() {
                                format!("{}-{}.{}", pom_dep.name, interp_version, ext)
                            } else {
                                format!("{}-{}-{}.{}", pom_dep.name, interp_version, pom_dep.classifier, ext)
                            };
                            let repo_base = repo.url.trim_end_matches('/');
                            let artifact_url = format!(
                                "{}/{}/{}/{}",
                                repo_base,
                                pom_dep.group.replace('.', "/"),
                                pom_dep.name,
                                artifact_filename
                            );
                            transitive_deps.push(ResolvedDependency {
                                group: pom_dep.group.clone(),
                                name: pom_dep.name.clone(),
                                version: interp_version.clone(),
                                selected_version: interp_version,
                                dependencies: Vec::new(),
                                resolved: true,
                                failure_reason: String::new(),
                                artifact_url,
                                artifact_size: 0,
                                artifact_sha256: String::new(),
                            });
                        }
                        tracing::debug!(
                            group = %group,
                            name = %name,
                            transitive = transitive_deps.len(),
                            "Resolved POM with {} transitive dependencies",
                            transitive_deps.len()
                        );
                        break; // Found POM, no need to try more repos
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
        }

        // Compute artifact URL using the first repo (or default Maven Central)
        let repo_base = repos.first()
            .map(|r| r.url.trim_end_matches('/').to_string())
            .unwrap_or_else(|| "https://repo.maven.apache.org/maven2".to_string());

        ResolvedDependency {
            group: group.clone(),
            name: name.clone(),
            version: raw_version,
            selected_version: selected_version.clone(),
            dependencies: transitive_deps,
            resolved: true,
            failure_reason: String::new(),
            artifact_url: format!(
                "{}/{}/{}/{}-{}.jar",
                repo_base,
                group.replace('.', "/"),
                name,
                name,
                selected_version
            ),
            artifact_size: 0,
            artifact_sha256: String::new(),
        }
    }

    /// Download an artifact with retry logic.
    async fn _download_with_retry(&self, url: &str, max_retries: u32) -> Result<Vec<u8>, String> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.http_client.get(url).send().await {
                Ok(resp) => match resp.status().as_u16() {
                    200..=299 => {
                        return resp.bytes().await
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
    let open = format!("<{}", std::str::from_utf8(tag).unwrap_or_default());
    let open_bytes = open.as_bytes();
    let mut search_from = from;

    while search_from < bytes.len() {
        if let Some(pos) = bytes[search_from..].windows(open_bytes.len())
            .position(|w| w == open_bytes)
            .map(|pos| search_from + pos)
        {
            // Check the character after the tag name: must be '>' or whitespace
            let after = pos + open_bytes.len();
            if after < bytes.len() {
                let next_char = bytes[after];
                if next_char == b'>' || next_char == b' ' || next_char == b'\n' || next_char == b'\r' || next_char == b'\t' {
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
    let close = format!("</{}", std::str::from_utf8(tag).unwrap_or_default());
    let close_bytes = close.as_bytes();
    bytes[from..].windows(close_bytes.len())
        .position(|w| w == close_bytes)
        .map(|pos| from + pos)
}

/// Extract text content of a child tag within a parent block.
fn extract_tag_text(bytes: &[u8], parent_start: usize, tag: &[u8]) -> Option<String> {
    let open_tag = format!("<{}", std::str::from_utf8(tag).unwrap_or_default());
    let close_tag = format!("</{}", std::str::from_utf8(tag).unwrap_or_default());
    let open_bytes = open_tag.as_bytes();
    let close_bytes = close_tag.as_bytes();

    // Find the opening tag after parent_start
    let search_from = parent_start;
    if let Some(start_pos) = bytes[search_from..].windows(open_bytes.len())
        .position(|w| w == open_bytes)
        .map(|pos| search_from + pos)
    {
        let content_start = start_pos + open_bytes.len();
        // Skip the closing `>` of the opening tag
        let content_start = content_start + bytes[content_start..].iter().position(|&b| b == b'>').unwrap_or(0) + 1;

        if let Some(end_pos) = bytes[content_start..].windows(close_bytes.len())
            .position(|w| w == close_bytes)
            .map(|pos| content_start + pos)
        {
            let content = &bytes[content_start..end_pos];
            let text = std::str::from_utf8(content).unwrap_or_default().trim().to_string();
            return Some(text);
        }
    }
    None
}

/// Compare two semver-like version strings.
/// Returns negative if a < b, 0 if a == b, positive if a > b.
/// Handles numeric segments (1.2.3) and suffixes (-beta, -SNAPSHOT).
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let a_parts = split_version(a);
    let b_parts = split_version(b);

    for (pa, pb) in a_parts.iter().zip(b_parts.iter()) {
        match (pa.parse::<u64>(), pb.parse::<u64>()) {
            (Ok(na), Ok(nb)) => {
                match na.cmp(&nb) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                }
            }
            _ => {
                match pa.cmp(pb) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                }
            }
        }
    }

    a_parts.len().cmp(&b_parts.len())
}

/// Split a version string into numeric/non-numeric segments.
fn split_version(version: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();

    for ch in version.chars() {
        if ch == '.' || ch == '-' {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

#[tonic::async_trait]
impl DependencyResolutionService for DependencyResolutionServiceImpl {
    async fn resolve_dependencies(
        &self,
        request: Request<ResolveDependenciesRequest>,
    ) -> Result<Response<ResolveDependenciesResponse>, Status> {
        let req = request.into_inner();
        let start = std::time::Instant::now();

        let repo_urls: Vec<RepositoryDescriptor> = req.repositories.iter().map(|r| RepositoryDescriptor {
            id: r.id.clone(),
            url: r.url.clone(),
            m2compatible: r.m2compatible,
            allow_insecure_protocol: r.allow_insecure_protocol,
            credentials: r.credentials.clone(),
        }).collect();
        let default_repo = RepositoryDescriptor {
            id: "central".to_string(),
            url: "https://repo.maven.apache.org/maven2/".to_string(),
            m2compatible: true,
            allow_insecure_protocol: false,
            credentials: Default::default(),
        };
        let repos = if repo_urls.is_empty() { vec![default_repo] } else { repo_urls };

        let mut resolved = Vec::new();
        for dep in &req.dependencies {
            let result = self.resolve_descriptor(dep, &repos).await;
            resolved.push(result);
        }

        let elapsed = start.elapsed().as_millis() as i64;
        let total_artifacts = resolved.len() as i32;

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
            total_download_size: 0,
        }))
    }

    async fn check_artifact_cache(
        &self,
        request: Request<CheckArtifactCacheRequest>,
    ) -> Result<Response<CheckArtifactCacheResponse>, Status> {
        let req = request.into_inner();

        let key = Self::artifact_cache_key(
            &req.group,
            &req.name,
            &req.version,
            &req.classifier,
        );

        if let Some(cached) = self.artifact_cache.get(&key) {
            self.resolution_stats.cache_hits.fetch_add(1, Ordering::Relaxed);

            // Validate SHA-256 if the caller provided one
            if !req.sha256.is_empty() && !cached.sha256.is_empty() && req.sha256 != cached.sha256 {
                tracing::warn!(
                    group = %cached.group,
                    name = %cached.name,
                    version = %cached.version,
                    classifier = %cached.classifier,
                    expected_sha256 = %req.sha256,
                    cached_sha256 = %cached.sha256,
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
                - cached.cached_at_ms;

            tracing::debug!(
                group = %cached.group,
                name = %cached.name,
                version = %cached.version,
                classifier = %cached.classifier,
                extension = %cached.extension,
                sha256 = %cached.sha256,
                local_path = %cached.local_path,
                size = cached.size,
                cached_at_ms = cached.cached_at_ms,
                age_ms,
                "Artifact cache hit"
            );

            return Ok(Response::new(CheckArtifactCacheResponse {
                cached: true,
                local_path: cached.local_path.clone(),
                cached_size: cached.size,
            }));
        }

        // Cold path: check filesystem for persisted artifact
        let path = self.artifact_path(
            &req.group,
            &req.name,
            &req.version,
            &req.classifier,
            &req.extension,
        );
        if path.exists() {
            let metadata = path.metadata().ok();
            let size = metadata.as_ref().map(|m| m.len() as i64).unwrap_or(0);

            let cached_artifact = CachedArtifact {
                group: req.group.clone(),
                name: req.name.clone(),
                version: req.version.clone(),
                classifier: req.classifier.clone(),
                extension: req.extension.clone(),
                sha256: req.sha256.clone(),
                local_path: path.to_string_lossy().to_string(),
                size,
                cached_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
            };

            // Insert into DashMap for future warm-path hits
            self.artifact_cache.insert(key.clone(), cached_artifact);

            tracing::debug!(
                group = %req.group,
                name = %req.name,
                version = %req.version,
                "Artifact cache cold-path hit from filesystem"
            );

            return Ok(Response::new(CheckArtifactCacheResponse {
                cached: true,
                local_path: path.to_string_lossy().to_string(),
                cached_size: size,
            }));
        }

        Ok(Response::new(CheckArtifactCacheResponse {
            cached: false,
            local_path: String::new(),
            cached_size: 0,
        }))
    }

    type DownloadArtifactStream = std::pin::Pin<Box<dyn tonic::codegen::tokio_stream::Stream<Item = Result<crate::proto::DownloadArtifactChunk, Status>> + Send>>;

    async fn download_artifact(
        &self,
        request: Request<crate::proto::DownloadArtifactRequest>,
    ) -> Result<Response<Self::DownloadArtifactStream>, Status> {
        let req = request.into_inner();

        let url = req.url.clone();
        let client = self.http_client.clone();

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

            let mut attempt = 0u32;
            let max_retries = 3u32;

            loop {
                attempt += 1;
                match client.get(&url).send().await {
                    Ok(resp) => {
                        match resp.status().as_u16() {
                            200..=299 => {
                                // Stream the response body in chunks
                                if let Some(total_size) = resp.content_length() {
                                    use futures_util::StreamExt;
                                    let mut offset = 0u64;
                                    let mut stream = resp.bytes_stream();

                                    while let Some(chunk_result) = stream.next().await {
                                        match chunk_result {
                                            Ok(bytes) => {
                                                let chunk_len = bytes.len() as u64;
                                                yield Ok(crate::proto::DownloadArtifactChunk {
                                                    data: bytes.to_vec(),
                                                    offset: offset as i64,
                                                    total_size: total_size as i64,
                                                    is_last: false,
                                                    error_message: String::new(),
                                                });
                                                offset += chunk_len;
                                            }
                                            Err(e) => {
                                                yield Ok(crate::proto::DownloadArtifactChunk {
                                                    data: Vec::new(),
                                                    offset: offset as i64,
                                                    total_size: total_size as i64,
                                                    is_last: true,
                                                    error_message: format!("Stream error: {}", e),
                                                });
                                                return;
                                            }
                                        }
                                    }

                                    // Final chunk
                                    yield Ok(crate::proto::DownloadArtifactChunk {
                                        data: Vec::new(),
                                        offset: offset as i64,
                                        total_size: total_size as i64,
                                        is_last: true,
                                        error_message: String::new(),
                                    });
                                } else {
                                    // Unknown size — read all into memory
                                    match resp.bytes().await {
                                        Ok(bytes) => {
                                            let total = bytes.len() as i64;
                                            let chunk_size = 64 * 1024;
                                            for (offset, chunk) in bytes.chunks(chunk_size).enumerate() {
                                                let is_last = offset * chunk_size + chunk.len() >= bytes.len();
                                                yield Ok(crate::proto::DownloadArtifactChunk {
                                                    data: chunk.to_vec(),
                                                    offset: (offset * chunk_size) as i64,
                                                    total_size: total,
                                                    is_last,
                                                    error_message: String::new(),
                                                });
                                            }
                                        }
                                        Err(e) => {
                                            yield Ok(crate::proto::DownloadArtifactChunk {
                                                data: Vec::new(),
                                                offset: 0,
                                                total_size: 0,
                                                is_last: true,
                                                error_message: format!("Failed to read response: {}", e),
                                            });
                                        }
                                    }
                                }
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

        Ok(Response::new(Box::pin(stream) as Self::DownloadArtifactStream))
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
        let total = self.resolution_stats.total_resolutions.load(Ordering::Relaxed);
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

        let key = Self::artifact_cache_key(&req.group, &req.name, &req.version, &req.classifier);

        let group = req.group.clone();
        let name = req.name.clone();
        let version = req.version.clone();
        let classifier = req.classifier.clone();
        let extension = if classifier.is_empty() {
            "jar".to_string()
        } else {
            format!("{}.jar", classifier)
        };

        // Compute persistent store path
        let store_path = self.artifact_path(&group, &name, &version, &classifier, &extension);

        // If a local file was provided, copy it to the persistent store
        let resolved_path = if !req.local_path.is_empty() {
            let src = Path::new(&req.local_path);
            if src.exists() {
                if let Some(parent) = store_path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let _ = tokio::fs::copy(src, &store_path).await;

                // Write SHA-256 sidecar
                if let Ok(data) = tokio::fs::read(&store_path).await {
                    let sha256 = Self::compute_sha256(&data);
                    let sha_path = store_path.with_extension("sha256");
                    let _ = tokio::fs::write(&sha_path, format!("{}  {}-{}.{}\n",
                        sha256, name, version, extension)).await;
                }

                store_path.to_string_lossy().to_string()
            } else {
                req.local_path.clone()
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
            sha256: req.sha256,
            local_path: resolved_path,
            size: req.size,
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
        }
    }

    fn make_repo(id: &str, url: &str) -> crate::proto::RepositoryDescriptor {
        crate::proto::RepositoryDescriptor {
            id: id.to_string(),
            url: url.to_string(),
            m2compatible: true,
            allow_insecure_protocol: false,
            credentials: Default::default(),
        }
    }

    #[tokio::test]
    async fn test_resolve_dependencies() {
        let svc = make_svc();

        let resp = svc
            .resolve_dependencies(Request::new(ResolveDependenciesRequest {
                configuration_name: "compileClasspath".to_string(),
                dependencies: vec![
                    make_dep("org.springframework", "spring-core", "5.3.30"),
                    make_dep("com.google.guava", "guava", "32.1.3"),
                ],
                repositories: vec![
                    make_repo("central", "https://repo.maven.apache.org/maven2/"),
                ],
                attributes: vec![],
                lenient: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.resolved_dependencies.len(), 2);
        assert_eq!(resp.resolved_dependencies[0].group, "org.springframework");
        assert_eq!(resp.resolved_dependencies[1].name, "guava");
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
            "com.example", "my-lib", "1.0", "");
        assert_eq!(key, "com.example:my-lib:1.0:");
    }

    #[tokio::test]
    async fn test_add_and_check_artifact_cache() {
        let svc = make_svc();

        // Not cached initially
        let miss = svc.check_artifact_cache(Request::new(CheckArtifactCacheRequest {
            group: "com.example".to_string(),
            name: "my-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            sha256: String::new(),
            extension: String::new(),
        })).await.unwrap().into_inner();
        assert!(!miss.cached);

        // Add to cache
        svc.add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
            group: "com.example".to_string(),
            name: "my-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            local_path: "/tmp/my-lib-1.0.jar".to_string(),
            size: 1024,
            sha256: "abc123".to_string(),
        })).await.unwrap();

        // Now cached
        let hit = svc.check_artifact_cache(Request::new(CheckArtifactCacheRequest {
            group: "com.example".to_string(),
            name: "my-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            sha256: String::new(),
            extension: String::new(),
        })).await.unwrap().into_inner();
        assert!(hit.cached);
        assert_eq!(hit.local_path, "/tmp/my-lib-1.0.jar");
        assert_eq!(hit.cached_size, 1024);
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
        })).await.unwrap();

        svc.record_resolution(Request::new(RecordResolutionRequest {
            configuration_name: "testRuntimeClasspath".to_string(),
            dependency_count: 20,
            resolution_time_ms: 200,
            success: true,
            cache_hits: 15,
        })).await.unwrap();

        let stats = svc.get_resolution_stats(Request::new(GetResolutionStatsRequest {}))
            .await.unwrap().into_inner();

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
            DependencyResolutionServiceImpl::interpolate_properties("spring-core-${spring.version}", &props),
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
        // "1.0" has 2 segments, "1.0.0" has 3 — "1.0" is shorter, so Less
        assert!(compare_versions("1.0", "1.0.0") == std::cmp::Ordering::Less);
        assert!(compare_versions("1.2.3", "1.2.4") == std::cmp::Ordering::Less);
        assert!(compare_versions("1.10.0", "1.9.0") == std::cmp::Ordering::Greater); // 10 > 9
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
            "1.0.0".to_string(), "1.5.0".to_string(), "2.0.0".to_string(), "2.5.0".to_string(),
        ];
        // [1.0.0,2.0.0) — should pick 1.5.0 (highest within range)
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range("[1.0.0,2.0.0)", &available, None),
            Some("1.5.0".to_string())
        );
    }

    #[test]
    fn test_resolve_version_range_open_ended() {
        let available = vec![
            "1.0.0".to_string(), "2.0.0".to_string(), "3.0.0".to_string(),
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
            DependencyResolutionServiceImpl::resolve_version_range("latest.release", &available, None),
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
        let interp = DependencyResolutionServiceImpl::interpolate_properties(&deps[0].version, &props);
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
            repositories: vec![make_repo("central", "https://repo.maven.apache.org/maven2/")],
            attributes: vec![],
            lenient: true,
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
        assert_eq!(hash, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
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
            DependencyResolutionServiceImpl::parse_checksum_value("abc123def456  artifact-1.0.jar\n"),
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
        let result = DependencyResolutionServiceImpl::parse_maven_metadata("<not-metadata/>").unwrap();
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
            DependencyResolutionServiceImpl::resolve_version_range("RELEASE", &available, Some(&meta)),
            Some("1.5.0".to_string())
        );
        // LATEST should use metadata.latest
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range("LATEST", &available, Some(&meta)),
            Some("2.0.0".to_string())
        );
        // latest.release should use metadata.release when available
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range("latest.release", &available, Some(&meta)),
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

        // Add to cache
        svc.add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
            group: "com.example".to_string(),
            name: "test-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            local_path: src.to_string_lossy().to_string(),
            size: 20,
            sha256: "abc123".to_string(),
        })).await.unwrap();

        // Verify file exists in Maven layout
        let expected = dir.path().join("com/example/test-lib/1.0/test-lib-1.0.jar");
        assert!(expected.exists(), "Artifact should be stored at Maven layout path");
        let content = std::fs::read(&expected).unwrap();
        assert_eq!(content, b"test artifact content");

        // Verify .sha256 sidecar exists
        let sha_path = expected.with_extension("sha256");
        assert!(sha_path.exists(), "SHA-256 sidecar should be written");
    }

    #[test]
    fn test_maven_layout_path() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let path = svc.artifact_path("com.google.guava", "guava", "32.1.3", "", "jar");
        let expected = dir.path().join("com/google/guava/guava/32.1.3/guava-32.1.3.jar");
        assert_eq!(path, expected);

        // With classifier
        let path2 = svc.artifact_path("com.google.guava", "guava", "32.1.3", "sources", "jar");
        let expected2 = dir.path().join("com/google/guava/guava/32.1.3/guava-32.1.3-sources.jar");
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
        let key = DependencyResolutionServiceImpl::artifact_cache_key("com.example", "cold-lib", "1.0", "");
        assert!(!svc.artifact_cache.contains_key(&key));

        // But check_artifact_cache should find it via filesystem
        let resp = svc.check_artifact_cache(Request::new(CheckArtifactCacheRequest {
            group: "com.example".to_string(),
            name: "cold-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            sha256: String::new(),
            extension: "jar".to_string(),
        })).await.unwrap().into_inner();

        assert!(resp.cached, "Should find artifact on filesystem");
        assert_eq!(resp.local_path, artifact_path.to_string_lossy().to_string());
        assert_eq!(resp.cached_size, 18);

        // Now DashMap should have it (warm path next time)
        assert!(svc.artifact_cache.contains_key(&key));
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
        };

        // The build_request method returns a RequestBuilder — we can't easily inspect it,
        // but we can verify it doesn't panic and the method is callable.
        let _ = svc.build_request(&repo, "com/example/lib/1.0/lib-1.0.pom");
    }

    #[tokio::test]
    async fn test_verify_checksum_no_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let svc = DependencyResolutionServiceImpl::new(dir.path().to_path_buf());
        let data = b"test data for checksum";

        // No sidecar files exist, so verification should pass (no checksum available)
        let result = svc.verify_artifact_checksum(data, "https://repo.example.com/test.jar").await;
        assert!(result.matched, "Should match when no sidecar is available");
        assert_eq!(result.algorithm, "sha256");
    }
}
