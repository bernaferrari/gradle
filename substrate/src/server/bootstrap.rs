use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    bootstrap_service_server::BootstrapService, CompleteBuildRequest, CompleteBuildResponse,
    GetSubstrateInfoRequest, GetSubstrateInfoResponse, HealthCheckRequest,
    HealthCheckResponse, InitBuildRequest, InitBuildResponse, SubstrateServiceInfo,
};
use crate::SERVER_VERSION;

/// Active build session.
struct BuildSession {
    #[allow(dead_code)]
    project_dir: String,
    start_time: Instant,
    #[allow(dead_code)]
    requested_parallelism: i32,
    #[allow(dead_code)]
    requested_features: Vec<String>,
}

/// Rust-native bootstrap service.
/// Coordinates Gradle initialization and provides the final JVM-Rust handoff.
pub struct BootstrapServiceImpl {
    sessions: DashMap<String, BuildSession>,
    request_counts: DashMap<String, AtomicI64>,
    start_time: Instant,
    health_status: std::sync::atomic::AtomicBool,
}

impl Default for BootstrapServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl BootstrapServiceImpl {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            request_counts: DashMap::new(),
            start_time: Instant::now(),
            health_status: std::sync::atomic::AtomicBool::new(true),
        }
    }

    fn increment_requests(service: &str, counts: &DashMap<String, AtomicI64>) -> i64 {
        counts
            .entry(service.to_string())
            .or_insert_with(|| AtomicI64::new(0))
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }
}

#[tonic::async_trait]
impl BootstrapService for BootstrapServiceImpl {
    async fn init_build(
        &self,
        request: Request<InitBuildRequest>,
    ) -> Result<Response<InitBuildResponse>, Status> {
        let req = request.into_inner();
        let project_dir = req.project_dir.clone();

        self.sessions.insert(
            req.build_id.clone(),
            BuildSession {
                project_dir: req.project_dir,
                start_time: Instant::now(),
                requested_parallelism: req.requested_parallelism,
                requested_features: req.requested_features,
            },
        );

        tracing::info!(
            build_id = %req.build_id,
            project_dir = %project_dir,
            parallelism = req.requested_parallelism,
            "Build session initialized"
        );

        Ok(Response::new(InitBuildResponse {
            build_id: req.build_id,
            substrate_version: SERVER_VERSION.to_string(),
            protocol_version: crate::PROTOCOL_VERSION.to_string(),
            max_parallelism: req.requested_parallelism,
        }))
    }

    async fn complete_build(
        &self,
        request: Request<CompleteBuildRequest>,
    ) -> Result<Response<CompleteBuildResponse>, Status> {
        let req = request.into_inner();

        if let Some((_key, session)) = self.sessions.remove(&req.build_id) {
            let duration = session.start_time.elapsed().as_millis() as i64;
            tracing::info!(
                build_id = %req.build_id,
                outcome = %req.outcome,
                duration_ms = duration,
                "Build completed"
            );
        }

        Ok(Response::new(CompleteBuildResponse { acknowledged: true }))
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let uptime = self.start_time.elapsed().as_secs();

        Ok(Response::new(HealthCheckResponse {
            healthy: self.health_status.load(Ordering::Relaxed),
            version: SERVER_VERSION.to_string(),
            uptime: format!("{}s", uptime),
            active_builds: self.sessions.len() as i64,
        }))
    }

    async fn get_substrate_info(
        &self,
        _request: Request<GetSubstrateInfoRequest>,
    ) -> Result<Response<GetSubstrateInfoResponse>, Status> {
        let all_services = [
            "control", "hash", "cache", "exec", "work",
            "execution-plan", "execution-history", "cache-orchestration",
            "file-fingerprint", "value-snapshot", "task-graph",
            "configuration", "plugin", "build-operations", "bootstrap",
            "dependency-resolution", "file-watch", "configuration-cache",
            "toolchain", "build-event-stream", "worker-process",
            "build-layout", "build-result", "problem-reporting",
            "resource-management", "build-comparison", "console",
            "test-execution", "artifact-publishing", "build-init",
            "incremental-compilation", "build-metrics", "garbage-collection",
        ];

        let services: Vec<SubstrateServiceInfo> = all_services
            .iter()
            .map(|&name| SubstrateServiceInfo {
                service_name: name.to_string(),
                status: "active".to_string(),
                requests_served: Self::increment_requests(name, &self.request_counts),
            })
            .collect();

        let total: i64 = services.iter().map(|s| s.requests_served).sum();

        Ok(Response::new(GetSubstrateInfoResponse {
            daemon_version: SERVER_VERSION.to_string(),
            services,
            total_requests: total,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_and_complete() {
        let svc = BootstrapServiceImpl::new();

        let resp = svc
            .init_build(Request::new(InitBuildRequest {
                build_id: "build-123".to_string(),
                project_dir: "/tmp/app".to_string(),
                start_time_ms: 0,
                requested_parallelism: 4,
                system_properties: Default::default(),
                requested_features: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.build_id, "build-123");
        assert_eq!(resp.max_parallelism, 4);

        assert!(svc.sessions.contains_key("build-123"));

        let resp2 = svc
            .complete_build(Request::new(CompleteBuildRequest {
                build_id: "build-123".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp2.acknowledged);
        assert!(!svc.sessions.contains_key("build-123"));
    }

    #[tokio::test]
    async fn test_health_check() {
        let svc = BootstrapServiceImpl::new();

        let resp = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.healthy);
        assert!(!resp.version.is_empty());
    }

    #[tokio::test]
    async fn test_substrate_info() {
        let svc = BootstrapServiceImpl::new();

        let resp = svc
            .get_substrate_info(Request::new(GetSubstrateInfoRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.daemon_version.is_empty());
        assert!(resp.services.len() >= 31);
        assert!(resp.total_requests > 0);
    }

    #[tokio::test]
    async fn test_multiple_build_sessions() {
        let svc = BootstrapServiceImpl::new();

        for id in &["build-1", "build-2", "build-3"] {
            svc.init_build(Request::new(InitBuildRequest {
                build_id: id.to_string(),
                project_dir: "/tmp/app".to_string(),
                start_time_ms: 0,
                requested_parallelism: 4,
                system_properties: Default::default(),
                requested_features: vec![],
            }))
            .await
            .unwrap();
        }

        assert_eq!(svc.sessions.len(), 3);

        let health = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(health.active_builds, 3);

        // Complete one
        svc.complete_build(Request::new(CompleteBuildRequest {
            build_id: "build-2".to_string(),
            outcome: "SUCCESS".to_string(),
            duration_ms: 1000,
        }))
        .await
        .unwrap();

        assert_eq!(svc.sessions.len(), 2);
    }

    #[tokio::test]
    async fn test_complete_nonexistent_build() {
        let svc = BootstrapServiceImpl::new();

        // Completing a build that was never initialized should not fail
        let resp = svc
            .complete_build(Request::new(CompleteBuildRequest {
                build_id: "nonexistent".to_string(),
                outcome: "FAILED".to_string(),
                duration_ms: 100,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.acknowledged);
    }

    #[tokio::test]
    async fn test_init_build_returns_protocol_version() {
        let svc = BootstrapServiceImpl::new();

        let resp = svc
            .init_build(Request::new(InitBuildRequest {
                build_id: "v-test".to_string(),
                project_dir: "/tmp".to_string(),
                start_time_ms: 0,
                requested_parallelism: 8,
                system_properties: Default::default(),
                requested_features: vec!["configuration-cache".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.substrate_version.is_empty());
        assert!(!resp.protocol_version.is_empty());
        assert_eq!(resp.max_parallelism, 8);
    }
}
