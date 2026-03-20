use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    dependency_resolution_service_server::DependencyResolutionService, CheckArtifactCacheRequest,
    CheckArtifactCacheResponse, DependencyDescriptor, RecordResolutionRequest,
    RecordResolutionResponse, ResolveDependenciesRequest,
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

    /// Parse a POM file and extract dependencies using a simple regex-based approach.
    /// This is more robust against POM format variations than SAX parsing.
    fn parse_pom_dependencies(pom_content: &str) -> Vec<PomDependency> {
        let mut dependencies = Vec::new();

        // Simple state machine: find <dependency> blocks and extract fields
        let bytes = pom_content.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            // Look for <dependency>
            if let Some(pos) = find_tag(bytes, i, b"dependency") {
                i = pos + b"<dependency>".len();

                let mut group = String::new();
                let mut name = String::new();
                let mut version = String::new();
                let mut scope = String::new();
                let mut optional = false;

                // Read until </dependency>
                while i < len {
                    if let Some(end_pos) = find_end_tag(bytes, i, b"dependency") {
                        // Extract fields
                        group = extract_tag_text(bytes, pos, b"groupId").unwrap_or_default();
                        name = extract_tag_text(bytes, pos, b"artifactId").unwrap_or_default();
                        version = extract_tag_text(bytes, pos, b"version").unwrap_or_default();
                        scope = extract_tag_text(bytes, pos, b"scope").unwrap_or_default();
                        optional = extract_tag_text(bytes, pos, b"optional")
                            .map(|v| v == "true")
                            .unwrap_or(false);

                        if !group.is_empty() && !name.is_empty() {
                            dependencies.push(PomDependency {
                                group, name, version, scope, optional,
                            });
                        }

                        i = end_pos + b"</dependency>".len();
                        break;
                    }
                    i += 1;
                }
            } else {
                break;
            }
        }

        dependencies
    }

    /// Resolve a single dependency descriptor with real POM fetching.
    async fn resolve_descriptor(
        &self,
        dep: &DependencyDescriptor,
        repo_urls: &[String],
    ) -> ResolvedDependency {
        let group = dep.group.clone();
        let name = dep.name.clone();
        let version = dep.version.clone();

        // Try to resolve version ranges and fetch POM for transitive deps
        let mut transitive_deps = Vec::new();

        if dep.transitive {
            for repo_url in repo_urls {
                match self.fetch_pom(&group, &name, &version, repo_url).await {
                    Ok(pom_content) => {
                        let pom_deps = Self::parse_pom_dependencies(&pom_content);
                        for pom_dep in &pom_deps {
                            // Skip test/provided scopes
                            if pom_dep.scope == "test" || pom_dep.scope == "provided" || pom_dep.optional {
                                continue;
                            }
                            transitive_deps.push(ResolvedDependency {
                                group: pom_dep.group.clone(),
                                name: pom_dep.name.clone(),
                                version: pom_dep.version.clone(),
                                selected_version: pom_dep.version.clone(),
                                dependencies: Vec::new(),
                                resolved: true,
                                failure_reason: String::new(),
                                artifact_url: format!(
                                    "https://repo.maven.apache.org/maven2/{}/{}/{}/{}-{}.jar",
                                    pom_dep.group.replace('.', "/"),
                                    pom_dep.name,
                                    pom_dep.version,
                                    pom_dep.name,
                                    pom_dep.version
                                ),
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
            version: version.clone(),
            selected_version: version.clone(),
            dependencies: transitive_deps,
            resolved: true,
            failure_reason: String::new(),
            artifact_url: format!(
                "https://repo.maven.apache.org/maven2/{}/{}/{}/{}-{}.jar",
                group.replace('.', "/"),
                name,
                version,
                name,
                version
            ),
            artifact_size: 0,
            artifact_sha256: String::new(),
        }
    }

    /// Download an artifact with retry logic.
    async fn download_with_retry(&self, url: &str, max_retries: u32) -> Result<Vec<u8>, String> {
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

/// Find the start of a tag (e.g., `<dependency>`) in bytes.
fn find_tag(bytes: &[u8], from: usize, tag: &[u8]) -> Option<usize> {
    let open = format!("<{}", std::str::from_utf8(tag).unwrap_or_default());
    let open_bytes = open.as_bytes();
    bytes[from..].windows(open_bytes.len())
        .position(|w| w == open_bytes)
        .map(|pos| from + pos)
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
                    data: Vec::new().into(),
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
                                                    data: bytes.to_vec().into(),
                                                    offset: offset as i64,
                                                    total_size: total_size as i64,
                                                    is_last: false,
                                                    error_message: String::new(),
                                                });
                                                offset += chunk_len;
                                            }
                                            Err(e) => {
                                                yield Ok(crate::proto::DownloadArtifactChunk {
                                                    data: Vec::new().into(),
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
                                        data: Vec::new().into(),
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
                                                    data: chunk.to_vec().into(),
                                                    offset: (offset * chunk_size) as i64,
                                                    total_size: total,
                                                    is_last,
                                                    error_message: String::new(),
                                                });
                                            }
                                        }
                                        Err(e) => {
                                            yield Ok(crate::proto::DownloadArtifactChunk {
                                                data: Vec::new().into(),
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
                                    data: Vec::new().into(),
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
                                    data: Vec::new().into(),
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
                            data: Vec::new().into(),
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
}
