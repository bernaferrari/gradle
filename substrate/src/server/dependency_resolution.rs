use std::sync::atomic::{AtomicI64, Ordering};

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

/// Rust-native dependency resolution service.
/// Resolves dependency graphs and manages artifact caching.
pub struct DependencyResolutionServiceImpl {
    artifact_cache: DashMap<String, CachedArtifact>,
    resolution_stats: ResolutionStats,
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
        }
    }

    fn artifact_cache_key(group: &str, name: &str, version: &str, classifier: &str) -> String {
        format!("{}:{}:{}:{}", group, name, version, classifier)
    }

    fn resolve_descriptor(&self, dep: &DependencyDescriptor) -> ResolvedDependency {
        // In a full implementation, this would:
        // 1. Query Maven Central / repositories via reqwest
        // 2. Parse POM files for transitive dependencies
        // 3. Resolve version conflicts
        // 4. Download and verify artifacts
        // For now, return a resolved marker
        ResolvedDependency {
            group: dep.group.clone(),
            name: dep.name.clone(),
            version: dep.version.clone(),
            selected_version: dep.version.clone(),
            dependencies: Vec::new(),
            resolved: true,
            failure_reason: String::new(),
            artifact_url: format!(
                "https://repo.maven.apache.org/maven2/{}/{}/{}/{}-{}.jar",
                dep.group.replace('.', "/"),
                dep.name,
                dep.version,
                dep.name,
                dep.version
            ),
            artifact_size: 0,
            artifact_sha256: String::new(),
        }
    }
}

#[tonic::async_trait]
impl DependencyResolutionService for DependencyResolutionServiceImpl {
    async fn resolve_dependencies(
        &self,
        request: Request<ResolveDependenciesRequest>,
    ) -> Result<Response<ResolveDependenciesResponse>, Status> {
        let req = request.into_inner();
        let start = std::time::Instant::now();

        let resolved: Vec<ResolvedDependency> = req
            .dependencies
            .into_iter()
            .map(|dep| self.resolve_descriptor(&dep))
            .collect();

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
        let _req = request.into_inner();

        // In a full implementation, this would stream the artifact bytes
        // from the repository using reqwest. For now, return an empty stream.
        let stream = async_stream::stream! {
            yield Ok(crate::proto::DownloadArtifactChunk {
                data: Vec::new().into(),
                offset: 0,
                total_size: 0,
                is_last: true,
                error_message: "Artifact download not yet implemented".to_string(),
            });
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
}
