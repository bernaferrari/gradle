use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gradle_substrate_daemon::client::jvm_host::JvmHostClient;
use gradle_substrate_daemon::client::jvm_host_bridge::JvmHostBridge;
use gradle_substrate_daemon::proto::jvm_host_service_server::{JvmHostService, JvmHostServiceServer};
use gradle_substrate_daemon::proto::{
    EvaluateScriptRequest, EvaluateScriptResponse, GetBuildEnvironmentRequest,
    GetBuildEnvironmentResponse, GetBuildModelRequest, GetBuildModelResponse, ProjectModel,
    ResolveConfigRequest, ResolveConfigResponse,
};
use gradle_substrate_daemon::server::build_plan_shadow::{
    BuildPlanShadowStore, capture_and_persist_shadow_from_jvm, verify_shadow_against_jvm,
};
use tokio::net::UnixListener;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

struct AlternatingMockJvmHostService {
    model_calls: AtomicUsize,
}

#[tonic::async_trait]
impl JvmHostService for AlternatingMockJvmHostService {
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
        let call_number = self.model_calls.fetch_add(1, Ordering::SeqCst);

        let mut projects = vec![
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
        ];

        if call_number % 2 == 1 {
            projects.reverse();
        }

        Ok(Response::new(GetBuildModelResponse { projects }))
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
    let service = AlternatingMockJvmHostService {
        model_calls: AtomicUsize::new(0),
    };

    tokio::spawn(async move {
        Server::builder()
            .add_service(JvmHostServiceServer::new(service))
            .serve_with_incoming(stream)
            .await
            .unwrap();
    });

    (socket_path.to_string_lossy().to_string(), temp_dir)
}

#[tokio::test]
async fn shadow_capture_is_order_insensitive_across_jvm_model_ordering() {
    let (socket_path, _server_dir) = spawn_mock_server().await;
    let client = JvmHostClient::connect(&socket_path).await.unwrap();
    let bridge = JvmHostBridge::new();
    bridge.set_client(client).await;

    let cache_dir = tempfile::tempdir().unwrap();
    let store = BuildPlanShadowStore::new(PathBuf::from(cache_dir.path()));
    let build_id = "shadow-diff-ordering";

    let _ = capture_and_persist_shadow_from_jvm(&bridge, &store, build_id)
        .await
        .unwrap()
        .expect("expected first shadow artifact");
    let first = store.load_plan(build_id).unwrap().expect("first artifact");

    let _ = capture_and_persist_shadow_from_jvm(&bridge, &store, build_id)
        .await
        .unwrap()
        .expect("expected second shadow artifact");
    let second = store.load_plan(build_id).unwrap().expect("second artifact");

    assert_eq!(first.fingerprint_sha256, second.fingerprint_sha256);

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
async fn shadow_capture_is_partitioned_by_build_id() {
    let (socket_path, _server_dir) = spawn_mock_server().await;
    let client = JvmHostClient::connect(&socket_path).await.unwrap();
    let bridge = Arc::new(JvmHostBridge::new());
    bridge.set_client(client).await;

    let cache_dir = tempfile::tempdir().unwrap();
    let store = Arc::new(BuildPlanShadowStore::new(PathBuf::from(cache_dir.path())));
    let build_ids = ["shadow-build-a", "shadow-build-b"];

    for build_id in build_ids {
        let _ = capture_and_persist_shadow_from_jvm(&bridge, &store, build_id)
            .await
            .unwrap()
            .expect("expected shadow artifact");
        let artifact = store.load_plan(build_id).unwrap().expect("artifact");
        assert_eq!(artifact.plan.build_id, build_id);

        let report = verify_shadow_against_jvm(&bridge, &store, build_id)
            .await
            .unwrap();
        assert!(
            report.is_match(),
            "expected no mismatches for {}: {:?}",
            build_id,
            report.mismatches
        );
    }

    let path_a = store.artifact_path_for_build_id(build_ids[0]);
    let path_b = store.artifact_path_for_build_id(build_ids[1]);
    assert_ne!(path_a, path_b);
}
