use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use super::build_event_stream::BuildEventStreamServiceImpl;
use super::build_plan_ir::from_proto;
use super::build_plan_shadow::{capture_and_persist_shadow_from_jvm, BuildPlanShadowStore};
use super::scopes::{BuildId, ScopeRegistry, SessionId};
use super::typed_scopes::ScopeGuard;
use crate::client::jvm_host_bridge::JvmHostBridge;
use crate::proto::{
    bootstrap_service_server::BootstrapService, BuildEventMessage, CompleteBuildRequest,
    CompleteBuildResponse, GetSubstrateInfoRequest, GetSubstrateInfoResponse, HealthCheckRequest,
    HealthCheckResponse, InitBuildRequest, InitBuildResponse, RefreshBuildPlanShadowRequest,
    RefreshBuildPlanShadowResponse, SubstrateServiceInfo,
};
use crate::SERVER_VERSION;

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
    /// RAII guards for active builds. When a guard is dropped (removed from this map),
    /// it automatically calls `ScopeRegistry::cleanup_build()` to release all
    /// scope-tracked state for that build.
    scope_guards: DashMap<BuildId, ScopeGuard>,
    request_counts: DashMap<String, AtomicI64>,
    start_time: Instant,
    health_status: std::sync::atomic::AtomicBool,
    scope_registry: Option<Arc<ScopeRegistry>>,
    jvm_bridge: Option<Arc<JvmHostBridge>>,
    build_plan_shadow_store: Option<Arc<BuildPlanShadowStore>>,
    event_stream: Option<Arc<BuildEventStreamServiceImpl>>,
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
            scope_guards: DashMap::new(),
            request_counts: DashMap::new(),
            start_time: Instant::now(),
            health_status: std::sync::atomic::AtomicBool::new(true),
            scope_registry: None,
            jvm_bridge: None,
            build_plan_shadow_store: None,
            event_stream: None,
        }
    }

    pub fn with_scope_registry(scope_registry: Arc<ScopeRegistry>) -> Self {
        Self {
            sessions: DashMap::new(),
            scope_guards: DashMap::new(),
            request_counts: DashMap::new(),
            start_time: Instant::now(),
            health_status: std::sync::atomic::AtomicBool::new(true),
            scope_registry: Some(scope_registry),
            jvm_bridge: None,
            build_plan_shadow_store: None,
            event_stream: None,
        }
    }

    pub fn with_scope_registry_and_shadow(
        scope_registry: Arc<ScopeRegistry>,
        jvm_bridge: Arc<JvmHostBridge>,
        build_plan_shadow_store: Arc<BuildPlanShadowStore>,
    ) -> Self {
        Self {
            sessions: DashMap::new(),
            scope_guards: DashMap::new(),
            request_counts: DashMap::new(),
            start_time: Instant::now(),
            health_status: std::sync::atomic::AtomicBool::new(true),
            scope_registry: Some(scope_registry),
            jvm_bridge: Some(jvm_bridge),
            build_plan_shadow_store: Some(build_plan_shadow_store),
            event_stream: None,
        }
    }

    /// Set the event stream for build lifecycle event emission.
    pub fn with_event_stream(mut self, event_stream: Arc<BuildEventStreamServiceImpl>) -> Self {
        self.event_stream = Some(event_stream);
        self
    }

    fn increment_requests(service: &str, counts: &DashMap<String, AtomicI64>) -> i64 {
        counts
            .entry(service.to_string())
            .or_insert_with(|| AtomicI64::new(0))
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

        // Register build in scope registry. When the caller provides a session_id
        // (e.g., from a Gradle session), use it directly. When session_id is empty
        // (e.g., RustBootstrapClient which does not send one), synthesize a session
        // from the build_id so the build is always registered and downstream services
        // like DagExecutorServiceImpl can validate scope membership.
        if let Some(ref registry) = self.scope_registry {
            let session_id = if req.session_id.is_empty() {
                SessionId::from(format!("__synth__{}", build_id_str))
            } else {
                SessionId::from(req.session_id.clone())
            };
            registry.register_build(session_id.clone(), build_id.clone());
            let guard = ScopeGuard::new(Arc::clone(registry), build_id.clone());
            self.scope_guards.insert(build_id.clone(), guard);
            tracing::debug!(
                build_id = %build_id_str,
                session_id = %session_id,
                synthetic = req.session_id.is_empty(),
                "Registered build in scope registry with RAII guard"
            );
        }

        // Shadow capture path: once a build is initialized and the JVM host is connected,
        // materialize and persist a canonical Build Plan IR artifact keyed by build_id.
        if let (Some(jvm_bridge), Some(shadow_store)) =
            (&self.jvm_bridge, &self.build_plan_shadow_store)
        {
            let jvm_bridge = Arc::clone(jvm_bridge);
            let shadow_store = Arc::clone(shadow_store);
            let build_id_for_shadow = req.build_id.clone();
            tokio::spawn(async move {
                for attempt in 1..=20 {
                    match capture_and_persist_shadow_from_jvm(
                        &jvm_bridge,
                        &shadow_store,
                        &build_id_for_shadow,
                    )
                    .await
                    {
                        Ok(Some(path)) => {
                            tracing::info!(
                                build_id = %build_id_for_shadow,
                                attempt,
                                artifact = %path.display(),
                                "Persisted per-build JVM->Rust build plan shadow artifact"
                            );
                            return;
                        }
                        Ok(None) => {
                            tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                        }
                        Err(error) => {
                            tracing::warn!(
                                build_id = %build_id_for_shadow,
                                attempt,
                                error = %error,
                                "Failed capturing per-build plan shadow from JVM host"
                            );
                            tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                        }
                    }
                }
                tracing::info!(
                    build_id = %build_id_for_shadow,
                    "No per-build JVM shadow plan captured within retry window"
                );
            });
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

        // Emit build_start event to the event stream for cross-service fan-out
        if let Some(ref event_stream) = self.event_stream {
            let mut props = std::collections::HashMap::new();
            props.insert("project_dir".to_string(), project_dir);
            props.insert("parallelism".to_string(), parallelism.to_string());
            event_stream.emit_event(BuildEventMessage {
                build_id: build_id_str.clone(),
                timestamp_ms: now_ms(),
                event_type: "build_start".to_string(),
                event_id: format!("bootstrap-{}", build_id_str),
                properties: props,
                display_name: "Build".to_string(),
                parent_id: String::new(),
            });
        }

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

        // Emit build_finish event to the event stream
        if let Some(ref event_stream) = self.event_stream {
            let mut props = std::collections::HashMap::new();
            props.insert("outcome".to_string(), req.outcome.clone());
            props.insert("duration_ms".to_string(), req.duration_ms.to_string());
            let event_id = format!("bootstrap-finish-{}", req.build_id);
            event_stream.emit_event(BuildEventMessage {
                build_id: req.build_id.clone(),
                timestamp_ms: now_ms(),
                event_type: "build_finish".to_string(),
                event_id,
                properties: props,
                display_name: "Build".to_string(),
                parent_id: String::new(),
            });
            event_stream.cleanup_build(&build_id);
        }

        // Clean up scope registry: dropping the ScopeGuard triggers RAII cleanup
        // via ScopeRegistry::cleanup_build(). This also cleans up tree associations.
        if let Some((_id, _guard)) = self.scope_guards.remove(&build_id) {
            tracing::debug!(build_id = %req.build_id, "Dropped scope guard, build cleaned up");
        } else if let Some(ref registry) = self.scope_registry {
            // Fallback: no guard was created (e.g., no session_id was provided)
            registry.cleanup_build(&build_id);
        }

        Ok(Response::new(CompleteBuildResponse { acknowledged: true }))
    }

    async fn refresh_build_plan_shadow(
        &self,
        request: Request<RefreshBuildPlanShadowRequest>,
    ) -> Result<Response<RefreshBuildPlanShadowResponse>, Status> {
        let req = request.into_inner();
        if req.build_id.is_empty() {
            return Err(Status::invalid_argument("build_id must not be empty"));
        }

        let Some(shadow_store) = self.build_plan_shadow_store.as_ref() else {
            return Ok(Response::new(RefreshBuildPlanShadowResponse {
                build_id: req.build_id,
                refreshed: false,
                artifact_path: String::new(),
                error_message: "build plan shadow store is not configured".to_string(),
            }));
        };

        if let Some(inline_plan) = req.inline_plan.as_ref() {
            let mut plan = from_proto(inline_plan);
            plan.build_id = req.build_id.clone();
            let source = plan
                .metadata
                .get("source")
                .filter(|value| !value.is_empty())
                .cloned()
                .unwrap_or_else(|| "task-graph-listener-inline".to_string());
            plan.metadata.insert("source".to_string(), source.clone());
            match shadow_store.persist_plan(&plan, &source) {
                Ok(path) => {
                    let artifact_path = path.display().to_string();
                    tracing::info!(
                        build_id = %req.build_id,
                        artifact = %artifact_path,
                        task_count = plan.tasks.len(),
                        dependency_count = plan.dependencies.len(),
                        "Refreshed inline JVM->Rust build plan shadow artifact"
                    );
                    return Ok(Response::new(RefreshBuildPlanShadowResponse {
                        build_id: req.build_id,
                        refreshed: true,
                        artifact_path,
                        error_message: String::new(),
                    }));
                }
                Err(error) => {
                    tracing::warn!(
                        build_id = %req.build_id,
                        error = %error,
                        "Failed persisting inline JVM->Rust build plan shadow artifact"
                    );
                    return Ok(Response::new(RefreshBuildPlanShadowResponse {
                        build_id: req.build_id,
                        refreshed: false,
                        artifact_path: String::new(),
                        error_message: error.to_string(),
                    }));
                }
            }
        }

        let Some(jvm_bridge) = self.jvm_bridge.as_ref() else {
            return Ok(Response::new(RefreshBuildPlanShadowResponse {
                build_id: req.build_id,
                refreshed: false,
                artifact_path: String::new(),
                error_message: "JVM host bridge is not configured".to_string(),
            }));
        };

        match capture_and_persist_shadow_from_jvm(jvm_bridge, shadow_store, &req.build_id).await {
            Ok(Some(path)) => {
                let artifact_path = path.display().to_string();
                tracing::info!(
                    build_id = %req.build_id,
                    artifact = %artifact_path,
                    "Refreshed JVM->Rust build plan shadow artifact"
                );
                Ok(Response::new(RefreshBuildPlanShadowResponse {
                    build_id: req.build_id,
                    refreshed: true,
                    artifact_path,
                    error_message: String::new(),
                }))
            }
            Ok(None) => Ok(Response::new(RefreshBuildPlanShadowResponse {
                build_id: req.build_id,
                refreshed: false,
                artifact_path: String::new(),
                error_message: "JVM build model unavailable".to_string(),
            })),
            Err(error) => {
                tracing::warn!(
                    build_id = %req.build_id,
                    error = %error,
                    "Failed refreshing JVM->Rust build plan shadow artifact"
                );
                Ok(Response::new(RefreshBuildPlanShadowResponse {
                    build_id: req.build_id,
                    refreshed: false,
                    artifact_path: String::new(),
                    error_message: error.to_string(),
                }))
            }
        }
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
            "control",
            "hash",
            "cache",
            "exec",
            "work",
            "execution-plan",
            "execution-history",
            "cache-orchestration",
            "file-fingerprint",
            "value-snapshot",
            "task-graph",
            "configuration",
            "plugin",
            "build-operations",
            "bootstrap",
            "dependency-resolution",
            "file-watch",
            "configuration-cache",
            "toolchain",
            "build-event-stream",
            "worker-process",
            "build-layout",
            "build-result",
            "problem-reporting",
            "resource-management",
            "build-comparison",
            "console",
            "test-execution",
            "artifact-publishing",
            "build-init",
            "incremental-compilation",
            "build-metrics",
            "garbage-collection",
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

        assert!(svc
            .sessions
            .contains_key(&BuildId::from("build-123".to_string())));

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
        assert!(!svc
            .sessions
            .contains_key(&BuildId::from("build-123".to_string())));
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
        assert!(svc
            .sessions
            .contains_key(&BuildId::from("zero-para".to_string())));

        // Verify the session stored the zero parallelism
        let session = svc
            .sessions
            .get(&BuildId::from("zero-para".to_string()))
            .unwrap();
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
            "hash",
            "cache",
            "exec",
            "work",
            "bootstrap",
            "control",
            "configuration",
            "file-watch",
            "dependency-resolution",
            "artifact-publishing",
            "worker-process",
            "build-event-stream",
            "console",
            "plugin",
            "test-execution",
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
                svc_info.service_name, svc_info.status
            );
        }

        // Each service should have exactly 1 request served (first call)
        for svc_info in &resp.services {
            assert_eq!(
                svc_info.requests_served, 1,
                "Service '{}' should have 1 request on first call, got {}",
                svc_info.service_name, svc_info.requests_served
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

    // -----------------------------------------------------------------------
    // Scope registration tests (nki.2): production-path bootstrap→DAG flow
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_scope_registration_with_explicit_session_id() {
        let registry = Arc::new(ScopeRegistry::new());
        let svc = BootstrapServiceImpl::with_scope_registry(Arc::clone(&registry));

        svc.init_build(Request::new(InitBuildRequest {
            build_id: "build-explicit".to_string(),
            project_dir: "/tmp".to_string(),
            start_time_ms: 0,
            requested_parallelism: 4,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: "session-42".to_string(),
        }))
        .await
        .unwrap();

        assert_eq!(
            registry.session_for_build(&BuildId::from("build-explicit".to_string())),
            Some(SessionId::from("session-42".to_string()))
        );
        assert!(svc
            .scope_guards
            .contains_key(&BuildId::from("build-explicit".to_string())));
    }

    #[tokio::test]
    async fn test_scope_registration_synthesizes_session_when_empty() {
        let registry = Arc::new(ScopeRegistry::new());
        let svc = BootstrapServiceImpl::with_scope_registry(Arc::clone(&registry));

        svc.init_build(Request::new(InitBuildRequest {
            build_id: "build-synth".to_string(),
            project_dir: "/tmp".to_string(),
            start_time_ms: 0,
            requested_parallelism: 4,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: String::new(),
        }))
        .await
        .unwrap();

        let session = registry
            .session_for_build(&BuildId::from("build-synth".to_string()))
            .expect("build should be registered even with empty session_id");
        assert!(
            session.0.starts_with("__synth__"),
            "synthesized session should have __synth__ prefix, got: {}",
            session.0
        );
        assert!(svc
            .scope_guards
            .contains_key(&BuildId::from("build-synth".to_string())));
    }

    #[tokio::test]
    async fn test_scope_cleanup_on_complete_build() {
        let registry = Arc::new(ScopeRegistry::new());
        let svc = BootstrapServiceImpl::with_scope_registry(Arc::clone(&registry));

        svc.init_build(Request::new(InitBuildRequest {
            build_id: "build-cleanup".to_string(),
            project_dir: "/tmp".to_string(),
            start_time_ms: 0,
            requested_parallelism: 4,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: "session-cleanup".to_string(),
        }))
        .await
        .unwrap();

        assert!(registry
            .session_for_build(&BuildId::from("build-cleanup".to_string()))
            .is_some());

        svc.complete_build(Request::new(CompleteBuildRequest {
            build_id: "build-cleanup".to_string(),
            outcome: "SUCCESS".to_string(),
            duration_ms: 100,
        }))
        .await
        .unwrap();

        assert!(
            registry
                .session_for_build(&BuildId::from("build-cleanup".to_string()))
                .is_none(),
            "scope should be cleaned up after complete_build"
        );
        assert!(
            !svc.scope_guards
                .contains_key(&BuildId::from("build-cleanup".to_string())),
            "scope guard should be removed after complete_build"
        );
    }

    #[tokio::test]
    async fn test_no_scope_registration_without_scope_registry() {
        let svc = BootstrapServiceImpl::new();

        let resp = svc
            .init_build(Request::new(InitBuildRequest {
                build_id: "build-no-reg".to_string(),
                project_dir: "/tmp".to_string(),
                start_time_ms: 0,
                requested_parallelism: 4,
                system_properties: Default::default(),
                requested_features: vec![],
                session_id: "session-irrelevant".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.build_id, "build-no-reg");
        assert!(svc.scope_guards.is_empty());
    }

    #[tokio::test]
    async fn test_dag_rejects_unregistered_build_with_scope_registry() {
        use crate::proto::dag_executor_service_server::DagExecutorService;
        use crate::proto::StartBuildRequest;
        use crate::server::dag_executor::DagExecutorServiceImpl;
        use crate::server::execution_plan::ExecutionPlanServiceImpl;
        use crate::server::task_graph::TaskGraphServiceImpl;
        use crate::server::work::WorkerScheduler;

        let registry = Arc::new(ScopeRegistry::new());
        let dag = DagExecutorServiceImpl::new(
            Arc::new(WorkerScheduler::new(4)),
            Arc::new(TaskGraphServiceImpl::new()),
            Arc::new(ExecutionPlanServiceImpl::default()),
            Vec::new(),
        )
        .with_scope_registry(Arc::clone(&registry));

        assert!(registry
            .session_for_build(&BuildId::from("never-registered".to_string()))
            .is_none());

        let result: Result<Response<crate::proto::StartBuildResponse>, Status> = dag
            .start_build(Request::new(StartBuildRequest {
                build_id: "never-registered".to_string(),
                ..Default::default()
            }))
            .await;

        let err = result.expect_err("start_build should reject unregistered scoped builds");
        assert_eq!(err.code(), tonic::Code::NotFound);
        assert!(registry
            .session_for_build(&BuildId::from("never-registered".to_string()))
            .is_none());
    }

    #[tokio::test]
    async fn test_dag_accepts_registered_build_after_bootstrap_init() {
        use crate::proto::dag_executor_service_server::DagExecutorService;
        use crate::proto::StartBuildRequest;
        use crate::server::dag_executor::DagExecutorServiceImpl;
        use crate::server::execution_plan::ExecutionPlanServiceImpl;
        use crate::server::task_graph::TaskGraphServiceImpl;
        use crate::server::work::WorkerScheduler;

        let registry = Arc::new(ScopeRegistry::new());

        let bootstrap = BootstrapServiceImpl::with_scope_registry(Arc::clone(&registry));
        let dag = DagExecutorServiceImpl::new(
            Arc::new(WorkerScheduler::new(4)),
            Arc::new(TaskGraphServiceImpl::new()),
            Arc::new(ExecutionPlanServiceImpl::default()),
            Vec::new(),
        )
        .with_scope_registry(Arc::clone(&registry));

        bootstrap
            .init_build(Request::new(InitBuildRequest {
                build_id: "build-integrated".to_string(),
                project_dir: "/tmp".to_string(),
                start_time_ms: 0,
                requested_parallelism: 4,
                system_properties: Default::default(),
                requested_features: vec![],
                session_id: String::new(),
            }))
            .await
            .unwrap();

        assert!(registry
            .session_for_build(&BuildId::from("build-integrated".to_string()))
            .is_some());

        let result: Result<Response<crate::proto::StartBuildResponse>, Status> = dag
            .start_build(Request::new(StartBuildRequest {
                build_id: "build-integrated".to_string(),
                ..Default::default()
            }))
            .await;

        assert!(
            result.is_ok(),
            "start_build should accept a build registered by bootstrap, got: {:?}",
            result
        );

        bootstrap
            .complete_build(Request::new(CompleteBuildRequest {
                build_id: "build-integrated".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 100,
            }))
            .await
            .unwrap();

        assert!(registry
            .session_for_build(&BuildId::from("build-integrated".to_string()))
            .is_none());
    }
}
