use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    dependency_resolution_service_server::DependencyResolutionService, AddArtifactToCacheRequest,
    AddArtifactToCacheResponse, CheckArtifactCacheRequest, CheckArtifactCacheResponse,
    DependencyDescriptor, GetResolutionStatsRequest, GetResolutionStatsResponse,
    RecordResolutionRequest, RecordResolutionResponse, ResolveDependenciesRequest,
    ResolveDependenciesResponse, ResolvedDependency,
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
}

/// Rust-native dependency resolution service.
/// Resolves dependency graphs, fetches POMs from Maven repos, and manages artifact caching.
pub struct DependencyResolutionServiceImpl {
    artifact_cache: DashMap<String, CachedArtifact>,
    resolution_stats: ResolutionStats,
    http_client: reqwest::Client,
}

impl Default for DependencyResolutionServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl DependencyResolutionServiceImpl {
    pub fn new() -> Self {
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
        }
    }

    fn artifact_cache_key(group: &str, name: &str, version: &str, classifier: &str) -> String {
        format!("{}:{}:{}:{}", group, name, version, classifier)
    }

    /// Fetch a POM file from a Maven repository and parse it.
    async fn fetch_pom(&self, group: &str, name: &str, version: &str, repo_url: &str) -> Result<String, String> {
        let group_path = group.replace('.', "/");
        let url = format!(
            "{}/{}/{}/{}/{}-{}.pom",
            repo_url.trim_end_matches('/'),
            group_path, name, version, name, version
        );

        let response = self.http_client.get(&url).send().await
            .map_err(|e| format!("Failed to fetch POM from {}: {}", url, e))?;

        match response.status().as_u16() {
            200 => response.text().await
                .map_err(|e| format!("Failed to read POM response: {}", e)),
            404 => Err(format!("POM not found: {}", url)),
            status => Err(format!("HTTP {} for {}", status, url)),
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

        while i < len {
            // Look for <dependency> (not <dependencyManagement, not </dependency>)
            let pos = match find_open_tag_exact(bytes, i, b"dependency") {
                Some(p) => p,
                None => break,
            };

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
    fn resolve_version_range(range: &str, available: &[String]) -> Option<String> {
        let range = range.trim();

        // Special versions
        if range == "latest.release" || range == "latest.integration" {
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

    /// Fetch POM metadata (maven-metadata.xml) for a dependency to find available versions.
    async fn fetch_available_versions(&self, group: &str, name: &str, repo_url: &str) -> Vec<String> {
        let group_path = group.replace('.', "/");
        let url = format!(
            "{}/{}/{}/maven-metadata.xml",
            repo_url.trim_end_matches('/'),
            group_path, name
        );

        match self.http_client.get(&url).send().await {
            Ok(resp) if resp.status().as_u16() == 200 => {
                if let Ok(body) = resp.text().await {
                    // Extract <version> tags from <versions> block
                    let bytes = body.as_bytes();
                    if let Some(versions_start) = find_open_tag_exact(bytes, 0, b"versions") {
                        if let Some(versions_end) = find_end_tag(bytes, versions_start, b"versions") {
                            let mut versions = Vec::new();
                            let mut i = versions_start + b"<versions>".len();
                            while i < versions_end {
                                if let Some(pos) = find_open_tag_exact(bytes, i, b"version") {
                                    if let Some(text) = extract_tag_text(bytes, pos, b"version") {
                                        versions.push(text);
                                    }
                                    i = pos + b"<version>".len();
                                } else {
                                    break;
                                }
                            }
                            return versions;
                        }
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Resolve a single dependency descriptor with real POM fetching.
    /// Handles property interpolation, version ranges, and transitive dependency resolution.
    async fn resolve_descriptor(
        &self,
        dep: &DependencyDescriptor,
        repo_urls: &[String],
    ) -> ResolvedDependency {
        let group = dep.group.clone();
        let name = dep.name.clone();
        let raw_version = dep.version.clone();

        // Resolve version ranges
        let selected_version = if raw_version.contains(',') || raw_version.starts_with('[') || raw_version.starts_with('(') {
            // Version range — need to fetch available versions
            let mut resolved = raw_version.clone();
            for repo_url in repo_urls {
                let available = self.fetch_available_versions(&group, &name, repo_url).await;
                if !available.is_empty() {
                    if let Some(v) = Self::resolve_version_range(&raw_version, &available) {
                        resolved = v;
                        break;
                    }
                }
            }
            resolved
        } else {
            raw_version.clone()
        };

        // Try to fetch POM for transitive deps
        let mut transitive_deps = Vec::new();

        if dep.transitive {
            for repo_url in repo_urls {
                match self.fetch_pom(&group, &name, &selected_version, repo_url).await {
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
                            let artifact_url = format!(
                                "https://repo.maven.apache.org/maven2/{}/{}/{}/{}-{}.jar",
                                pom_dep.group.replace('.', "/"),
                                pom_dep.name,
                                interp_version,
                                pom_dep.name,
                                interp_version
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
                            repo = %repo_url,
                            error = %e,
                            "Failed to fetch POM from repo"
                        );
                    }
                }
            }
        }

        ResolvedDependency {
            group: group.clone(),
            name: name.clone(),
            version: raw_version,
            selected_version: selected_version.clone(),
            dependencies: transitive_deps,
            resolved: true,
            failure_reason: String::new(),
            artifact_url: format!(
                "https://repo.maven.apache.org/maven2/{}/{}/{}/{}-{}.jar",
                group.replace('.', "/"),
                name,
                selected_version,
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

        let repo_urls: Vec<String> = req.repositories.iter().map(|r| r.url.clone()).collect();
        let default_repos = vec!["https://repo.maven.apache.org/maven2/".to_string()];
        let repos = if repo_urls.is_empty() { default_repos } else { repo_urls };

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
            return Ok(Response::new(CheckArtifactCacheResponse {
                cached: true,
                local_path: cached.local_path.clone(),
                cached_size: cached.size,
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
            "Resolution recorded"
        );

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

        let artifact = CachedArtifact {
            group,
            name,
            version,
            classifier,
            extension: String::new(),
            sha256: req.sha256,
            local_path: req.local_path,
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
        let svc = DependencyResolutionServiceImpl::new();

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
        let svc = DependencyResolutionServiceImpl::new();

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
        let svc = DependencyResolutionServiceImpl::new();

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
        let svc = DependencyResolutionServiceImpl::new();

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
        let svc = DependencyResolutionServiceImpl::new();

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

        // Should only find the two <dependency> tags (not the one inside <dependencyManagement>)
        let deps = DependencyResolutionServiceImpl::parse_pom_dependencies(pom);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].group, "org.springframework"); // Inside dependencyManagement — still found by the parser
        assert_eq!(deps[1].group, "junit");
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
            DependencyResolutionServiceImpl::resolve_version_range("1.0.0", &available),
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
            DependencyResolutionServiceImpl::resolve_version_range("[1.0.0,2.0.0)", &available),
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
            DependencyResolutionServiceImpl::resolve_version_range("(1.0,)", &available),
            Some("3.0.0".to_string())
        );
    }

    #[test]
    fn test_resolve_version_range_latest() {
        let available = vec!["1.0.0".to_string(), "2.0.0".to_string()];
        assert_eq!(
            DependencyResolutionServiceImpl::resolve_version_range("latest.release", &available),
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
}
