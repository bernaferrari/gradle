use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gradle_substrate_daemon::client::jvm_host::JvmHostClient;
use gradle_substrate_daemon::client::jvm_host_bridge::JvmHostBridge;
use gradle_substrate_daemon::proto::bootstrap_service_server::BootstrapService;
use gradle_substrate_daemon::proto::jvm_host_service_server::{JvmHostService, JvmHostServiceServer};
use gradle_substrate_daemon::proto::{
    InitBuildRequest,
    EvaluateScriptRequest, EvaluateScriptResponse, GetBuildEnvironmentRequest,
    GetBuildEnvironmentResponse, GetBuildModelRequest, GetBuildModelResponse, ProjectModel,
    ResolveConfigRequest, ResolveConfigResponse,
};
use gradle_substrate_daemon::server::bootstrap::BootstrapServiceImpl;
use gradle_substrate_daemon::server::build_plan_shadow::{
    capture_and_persist_shadow_from_jvm, verify_shadow_against_jvm, BuildPlanShadowStore,
};
use gradle_substrate_daemon::server::scopes::ScopeRegistry;
use tokio::net::UnixListener;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

struct MockJvmHostService;

#[tonic::async_trait]
impl JvmHostService for MockJvmHostService {
    async fn evaluate_script(
        &self,
        _request: Request<EvaluateScriptRequest>,
    ) -> Result<Response<EvaluateScriptResponse>, Status> {
        Ok(Response::new(EvaluateScriptResponse {
            success: true,
            error_message: String::new(),
            applied_plugins: Vec::new(),
        }))
    }

    async fn get_build_model(
        &self,
        request: Request<GetBuildModelRequest>,
    ) -> Result<Response<GetBuildModelResponse>, Status> {
        let build_id = request.into_inner().build_id;
        Ok(Response::new(GetBuildModelResponse {
            projects: vec![
                ProjectModel {
                    path: ":".to_string(),
                    name: "root".to_string(),
                    build_file: "/repo/build.gradle.kts".to_string(),
                    subprojects: vec![":app".to_string()],
                },
                ProjectModel {
                    path: ":app".to_string(),
                    name: format!("app-{}", build_id),
                    build_file: "/repo/app/build.gradle.kts".to_string(),
                    subprojects: vec![],
                },
            ],
        }))
    }

    async fn resolve_configuration(
        &self,
        _request: Request<ResolveConfigRequest>,
    ) -> Result<Response<ResolveConfigResponse>, Status> {
        Ok(Response::new(ResolveConfigResponse {
            success: true,
            artifacts: Vec::new(),
            error_message: String::new(),
        }))
    }

    async fn get_build_environment(
        &self,
        _request: Request<GetBuildEnvironmentRequest>,
    ) -> Result<Response<GetBuildEnvironmentResponse>, Status> {
        Ok(Response::new(GetBuildEnvironmentResponse {
            java_version: "21.0.4".to_string(),
            java_home: "/jdk/21".to_string(),
            gradle_version: "9.0.0".to_string(),
            os_name: "Linux".to_string(),
            os_arch: "amd64".to_string(),
            available_processors: 8,
            max_memory_bytes: 4_000_000_000,
            system_properties: HashMap::new(),
        }))
    }
}

async fn spawn_mock_server() -> (String, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("jvm-host.sock");

    let uds = UnixListener::bind(&socket_path).unwrap();
    let stream = tokio_stream::wrappers::UnixListenerStream::new(uds);

    tokio::spawn(async move {
        Server::builder()
            .add_service(JvmHostServiceServer::new(MockJvmHostService))
            .serve_with_incoming(stream)
            .await
            .unwrap();
    });

    (socket_path.to_string_lossy().to_string(), temp_dir)
}

#[tokio::test]
async fn capture_and_persist_shadow_build_plan_artifact() {
    let (socket_path, _tmp_server_dir) = spawn_mock_server().await;
    let client = JvmHostClient::connect(&socket_path).await.unwrap();

    let bridge = JvmHostBridge::new();
    bridge.set_client(client).await;

    let cache_dir = tempfile::tempdir().unwrap();
    let store = BuildPlanShadowStore::new(PathBuf::from(cache_dir.path()));
    let artifact_path = capture_and_persist_shadow_from_jvm(&bridge, &store, "build-it")
        .await
        .unwrap()
        .expect("expected shadow artifact to be persisted");

    assert!(artifact_path.exists());

    let loaded = store.load_plan("build-it").unwrap().unwrap();
    assert_eq!(loaded.plan.build_id, "build-it");
    assert_eq!(loaded.source, "jvm-host-shadow");
    assert!(!loaded.fingerprint_sha256.is_empty());
    assert_eq!(loaded.plan.toolchains.len(), 1);

    let report = verify_shadow_against_jvm(&bridge, &store, "build-it")
        .await
        .unwrap();
    assert!(
        report.is_match(),
        "expected no mismatches, got: {:?}",
        report.mismatches
    );
}

#[tokio::test]
async fn bootstrap_init_build_persists_per_build_shadow_artifact() {
    let (socket_path, _tmp_server_dir) = spawn_mock_server().await;
    let client = JvmHostClient::connect(&socket_path).await.unwrap();

    let bridge = Arc::new(JvmHostBridge::new());
    bridge.set_client(client).await;

    let cache_dir = tempfile::tempdir().unwrap();
    let store = Arc::new(BuildPlanShadowStore::new(PathBuf::from(cache_dir.path())));
    let scope_registry = Arc::new(ScopeRegistry::new());
    let bootstrap = BootstrapServiceImpl::with_scope_registry_and_shadow(
        scope_registry,
        Arc::clone(&bridge),
        Arc::clone(&store),
    );

    let build_id = "build-bootstrap-shadow";
    let _ = bootstrap
        .init_build(Request::new(InitBuildRequest {
            build_id: build_id.to_string(),
            project_dir: "/repo".to_string(),
            start_time_ms: 1,
            requested_parallelism: 1,
            system_properties: HashMap::new(),
            requested_features: Vec::new(),
            session_id: "sess-1".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    let mut loaded = None;
    for _ in 0..20 {
        loaded = store.load_plan(build_id).unwrap();
        if loaded.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let artifact = loaded.expect("expected per-build shadow artifact to exist");
    assert_eq!(artifact.plan.build_id, build_id);
    assert_eq!(artifact.source, "jvm-host-shadow");

    let report = verify_shadow_against_jvm(&bridge, &store, build_id)
        .await
        .unwrap();
    assert!(
        report.is_match(),
        "expected no mismatches, got: {:?}",
        report.mismatches
    );
}

#[tokio::test]
async fn detect_shadow_mismatch_after_manual_mutation() {
    let (socket_path, _tmp_server_dir) = spawn_mock_server().await;
    let client = JvmHostClient::connect(&socket_path).await.unwrap();

    let bridge = JvmHostBridge::new();
    bridge.set_client(client).await;

    let cache_dir = tempfile::tempdir().unwrap();
    let store = BuildPlanShadowStore::new(PathBuf::from(cache_dir.path()));
    let build_id = "build-mutated";

    let _ = capture_and_persist_shadow_from_jvm(&bridge, &store, build_id)
        .await
        .unwrap()
        .expect("expected initial artifact");

    let artifact_path = store.artifact_path_for_build_id(build_id);
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&artifact_path).unwrap())
        .unwrap();
    json["plan"]["projects"][0]["name"] = serde_json::Value::String("tampered".to_string());
    std::fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&json).unwrap(),
    )
    .unwrap();

    let report = verify_shadow_against_jvm(&bridge, &store, build_id)
        .await
        .unwrap();
    assert!(
        !report.is_match(),
        "expected mismatches after mutation, report={:?}",
        report
    );
}
