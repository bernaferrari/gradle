use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    bootstrap_service_server::BootstrapService, CompleteBuildRequest, CompleteBuildResponse,
    GetSubstrateInfoRequest, GetSubstrateInfoResponse, HealthCheckRequest,
    HealthCheckResponse, InitBuildRequest, InitBuildResponse, SubstrateServiceInfo,
};
use crate::SERVER_VERSION;
use super::scopes::{BuildId, ScopeRegistry, SessionId};

/// Active build session.
struct BuildSession {
    project_dir: String,
    start_time: Instant,
    start_time_ms: i64,
    requested_parallelism: i32,
    requested_features: Vec<String>,
    system_properties: std::collections::HashMap<String, String>,
}

/// Rust-native bootstrap service.
/// Coordinates Gradle initialization and provides the final JVM-Rust handoff.
pub struct BootstrapServiceImpl {
    sessions: DashMap<BuildId, BuildSession>,
    request_counts: DashMap<String, AtomicI64>,
    start_time: Instant,
    health_status: std::sync::atomic::AtomicBool,
    scope_registry: Option<Arc<ScopeRegistry>>,
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
            scope_registry: None,
        }
    }

    pub fn with_scope_registry(scope_registry: Arc<ScopeRegistry>) -> Self {
        Self {
            sessions: DashMap::new(),
            request_counts: DashMap::new(),
            start_time: Instant::now(),
            health_status: std::sync::atomic::AtomicBool::new(true),
            scope_registry: Some(scope_registry),
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

        // Validate inputs
        if req.build_id.is_empty() {
            return Err(Status::invalid_argument("build_id must not be empty"));
        }
        if req.project_dir.is_empty() {
            return Err(Status::invalid_argument("project_dir must not be empty"));
        }
        if req.requested_parallelism < 0 {
            return Err(Status::invalid_argument(
                "requested_parallelism must be non-negative",
            ));
        }

        let project_dir = req.project_dir.clone();
        let features_list: Vec<String> = req.requested_features.clone();
        let sys_prop_count = req.system_properties.len();
        let build_id = BuildId::from(req.build_id.clone());
        let build_id_str = req.build_id.clone();
        let parallelism = req.requested_parallelism;
        let client_start_time_ms = req.start_time_ms;

        self.sessions.insert(
            build_id.clone(),
            BuildSession {
                project_dir: req.project_dir,
                start_time: Instant::now(),
                start_time_ms: client_start_time_ms,
                requested_parallelism: parallelism,
                requested_features: req.requested_features,
                system_properties: req.system_properties,
            },
        );

        // Register build in scope registry if session_id is provided
        if let Some(ref registry) = self.scope_registry {
            if !req.session_id.is_empty() {
                registry.register_build(
                    SessionId::from(req.session_id.clone()),
                    build_id.clone(),
                );
                tracing::debug!(
                    build_id = %build_id_str,
                    session_id = %req.session_id,
                    "Registered build in scope registry"
                );
            }
        }

        tracing::info!(
            build_id = %build_id_str,
            project_dir = %project_dir,
            parallelism = parallelism,
            features = ?features_list,
            system_properties_count = sys_prop_count,
            client_start_time_ms = client_start_time_ms,
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
        let build_id = BuildId::from(req.build_id.clone());

        if let Some((_key, session)) = self.sessions.remove(&build_id) {
            let server_duration_ms = session.start_time.elapsed().as_millis() as i64;
            let features_list = session
                .requested_features
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>();
            let sys_prop_count = session.system_properties.len();

            tracing::info!(
                build_id = %req.build_id,
                project_dir = %session.project_dir,
                outcome = %req.outcome,
                server_duration_ms = server_duration_ms,
                client_reported_duration_ms = req.duration_ms,
                requested_parallelism = session.requested_parallelism,
                features = ?features_list,
                system_properties_count = sys_prop_count,
                client_start_time_ms = session.start_time_ms,
                "Build completed"
            );
        } else {
            tracing::warn!(
                build_id = %req.build_id,
                outcome = %req.outcome,
                client_reported_duration_ms = req.duration_ms,
                "CompleteBuild called for unknown session"
            );
        }

        // Clean up scope registry
        if let Some(ref registry) = self.scope_registry {
            registry.cleanup_build(&build_id);
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
                session_id: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.build_id, "build-123");
        assert_eq!(resp.max_parallelism, 4);

        assert!(svc.sessions.contains_key(&BuildId::from("build-123".to_string())));

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
        assert!(!svc.sessions.contains_key(&BuildId::from("build-123".to_string())));
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
                session_id: String::new(),
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
                session_id: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.substrate_version.is_empty());
        assert!(!resp.protocol_version.is_empty());
        assert_eq!(resp.max_parallelism, 8);
    }

    #[tokio::test]
    async fn test_duplicate_init_build() {
        let svc = BootstrapServiceImpl::new();

        svc.init_build(Request::new(InitBuildRequest {
            build_id: "dup-build".to_string(),
            project_dir: "/tmp/app".to_string(),
            start_time_ms: 0,
            requested_parallelism: 4,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: String::new(),
        }))
        .await
        .unwrap();

        // Re-initializing same build_id should overwrite
        svc.init_build(Request::new(InitBuildRequest {
            build_id: "dup-build".to_string(),
            project_dir: "/tmp/app2".to_string(),
            start_time_ms: 0,
            requested_parallelism: 8,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: String::new(),
        }))
        .await
        .unwrap();

        assert_eq!(svc.sessions.len(), 1);
        let health = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(health.active_builds, 1);
    }

    #[tokio::test]
    async fn test_complete_same_build_twice() {
        let svc = BootstrapServiceImpl::new();

        svc.init_build(Request::new(InitBuildRequest {
            build_id: "twice-build".to_string(),
            project_dir: "/tmp".to_string(),
            start_time_ms: 0,
            requested_parallelism: 4,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: String::new(),
        }))
        .await
        .unwrap();

        // First complete removes the session
        svc.complete_build(Request::new(CompleteBuildRequest {
            build_id: "twice-build".to_string(),
            outcome: "SUCCESS".to_string(),
            duration_ms: 100,
        }))
        .await
        .unwrap();

        assert_eq!(svc.sessions.len(), 0);

        // Second complete should succeed (no-op)
        let resp = svc
            .complete_build(Request::new(CompleteBuildRequest {
                build_id: "twice-build".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 200,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.acknowledged);
    }

    #[tokio::test]
    async fn test_substrate_info_increments() {
        let svc = BootstrapServiceImpl::new();

        let resp1 = svc
            .get_substrate_info(Request::new(GetSubstrateInfoRequest {}))
            .await
            .unwrap()
            .into_inner();

        let total1 = resp1.total_requests;

        // Call again — should increment
        let resp2 = svc
            .get_substrate_info(Request::new(GetSubstrateInfoRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert!(resp2.total_requests > total1);
    }

    #[tokio::test]
    async fn test_health_check_active_builds_after_init_and_complete() {
        let svc = BootstrapServiceImpl::new();

        // Initially zero active builds
        let health0 = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(health0.active_builds, 0);

        // Init two builds
        svc.init_build(Request::new(InitBuildRequest {
            build_id: "hc-build-1".to_string(),
            project_dir: "/tmp/a".to_string(),
            start_time_ms: 0,
            requested_parallelism: 2,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: String::new(),
        }))
        .await
        .unwrap();

        svc.init_build(Request::new(InitBuildRequest {
            build_id: "hc-build-2".to_string(),
            project_dir: "/tmp/b".to_string(),
            start_time_ms: 0,
            requested_parallelism: 2,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: String::new(),
        }))
        .await
        .unwrap();

        // Should report 2 active builds
        let health1 = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(health1.active_builds, 2);

        // Complete one build
        svc.complete_build(Request::new(CompleteBuildRequest {
            build_id: "hc-build-1".to_string(),
            outcome: "SUCCESS".to_string(),
            duration_ms: 3000,
        }))
        .await
        .unwrap();

        // Should report 1 active build
        let health2 = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(health2.active_builds, 1);

        // Complete the other build
        svc.complete_build(Request::new(CompleteBuildRequest {
            build_id: "hc-build-2".to_string(),
            outcome: "SUCCESS".to_string(),
            duration_ms: 4000,
        }))
        .await
        .unwrap();

        // Should report 0 active builds
        let health3 = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(health3.active_builds, 0);
    }

    #[tokio::test]
    async fn test_init_build_with_zero_parallelism() {
        let svc = BootstrapServiceImpl::new();

        let resp = svc
            .init_build(Request::new(InitBuildRequest {
                build_id: "zero-para".to_string(),
                project_dir: "/tmp/sequential".to_string(),
                start_time_ms: 0,
                requested_parallelism: 0,
                system_properties: Default::default(),
                requested_features: vec![],
                session_id: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.build_id, "zero-para");
        assert_eq!(resp.max_parallelism, 0);
        assert!(svc.sessions.contains_key(&BuildId::from("zero-para".to_string())));

        // Verify the session stored the zero parallelism
        let session = svc.sessions.get(&BuildId::from("zero-para".to_string())).unwrap();
        assert_eq!(session.requested_parallelism, 0);
    }

    #[tokio::test]
    async fn test_substrate_info_lists_expected_services() {
        let svc = BootstrapServiceImpl::new();

        let resp = svc
            .get_substrate_info(Request::new(GetSubstrateInfoRequest {}))
            .await
            .unwrap()
            .into_inner();

        let expected_core_services = [
            "hash", "cache", "exec", "work", "bootstrap", "control",
            "configuration", "file-watch", "dependency-resolution",
            "artifact-publishing", "worker-process", "build-event-stream",
            "console", "plugin", "test-execution",
        ];

        // Collect the service names returned
        let service_names: Vec<&str> = resp
            .services
            .iter()
            .map(|s| s.service_name.as_str())
            .collect();

        // Every expected service must be present
        for expected in &expected_core_services {
            assert!(
                service_names.contains(expected),
                "Expected service '{}' not found in {:?}",
                expected,
                service_names
            );
        }

        // All services should report "active" status
        for svc_info in &resp.services {
            assert_eq!(
                svc_info.status, "active",
                "Service '{}' should be active, got '{}'",
                svc_info.service_name,
                svc_info.status
            );
        }

        // Each service should have exactly 1 request served (first call)
        for svc_info in &resp.services {
            assert_eq!(
                svc_info.requests_served, 1,
                "Service '{}' should have 1 request on first call, got {}",
                svc_info.service_name,
                svc_info.requests_served
            );
        }
    }

    #[tokio::test]
    async fn test_complete_build_reduces_active_count() {
        let svc = BootstrapServiceImpl::new();

        // Init three builds
        for id in &["dec-a", "dec-b", "dec-c"] {
            svc.init_build(Request::new(InitBuildRequest {
                build_id: id.to_string(),
                project_dir: "/tmp".to_string(),
                start_time_ms: 0,
                requested_parallelism: 4,
                system_properties: Default::default(),
                requested_features: vec![],
                session_id: String::new(),
            }))
            .await
            .unwrap();
        }

        assert_eq!(svc.sessions.len(), 3);

        let health_before = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(health_before.active_builds, 3);

        // Complete two builds
        for id in &["dec-a", "dec-c"] {
            svc.complete_build(Request::new(CompleteBuildRequest {
                build_id: id.to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 500,
            }))
            .await
            .unwrap();
        }

        assert_eq!(svc.sessions.len(), 1);

        let health_after = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(health_after.active_builds, 1);
        assert_eq!(health_before.active_builds - health_after.active_builds, 2);

        // Complete the last one and verify zero
        svc.complete_build(Request::new(CompleteBuildRequest {
            build_id: "dec-b".to_string(),
            outcome: "SUCCESS".to_string(),
            duration_ms: 600,
        }))
        .await
        .unwrap();

        let health_final = svc
            .health_check(Request::new(HealthCheckRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(health_final.active_builds, 0);
    }
}
