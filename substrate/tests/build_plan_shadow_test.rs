use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gradle_substrate_daemon::client::jvm_host::JvmHostClient;
use gradle_substrate_daemon::client::jvm_host_bridge::JvmHostBridge;
use gradle_substrate_daemon::proto::bootstrap_service_server::BootstrapService;
use gradle_substrate_daemon::proto::jvm_host_service_server::{
    JvmHostService, JvmHostServiceServer,
};
use gradle_substrate_daemon::proto::{
    BuildPlan, BuildPlanTask, EvaluateScriptRequest, EvaluateScriptResponse, ExecuteTaskRequest,
    ExecuteTaskResponse, GetBuildEnvironmentRequest, GetBuildEnvironmentResponse,
    GetBuildModelRequest, GetBuildModelResponse, GetBuildPlanRequest, GetBuildPlanResponse,
    InitBuildRequest, ProjectModel, RefreshBuildPlanShadowRequest, ResolveConfigRequest,
    ResolveConfigResponse,
};
use gradle_substrate_daemon::server::bootstrap::BootstrapServiceImpl;
use gradle_substrate_daemon::server::build_plan_ir::BUILD_PLAN_SCHEMA_VERSION;
use gradle_substrate_daemon::server::build_plan_shadow::{
    capture_and_persist_shadow_from_jvm, verify_shadow_against_jvm, BuildPlanShadowStore,
};
use gradle_substrate_daemon::server::scopes::ScopeRegistry;
use tokio::net::UnixListener;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

struct MockJvmHostService {
    repo_root: PathBuf,
}

fn mock_build_plan_tasks() -> Vec<BuildPlanTask> {
    vec![
        BuildPlanTask {
            path: ":lint".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.DefaultTask".to_string(),
            depends_on: vec![":check".to_string()],
            inputs: HashMap::from([
                ("source".to_string(), "mock-jvm-task-model".to_string()),
                ("projectGroup".to_string(), "org.example.shadow".to_string()),
                ("projectVersion".to_string(), "1.0.0".to_string()),
                ("projectRepositoryTypes".to_string(), "maven".to_string()),
                ("shouldRunAfter".to_string(), ":test".to_string()),
            ]),
            outputs: Vec::new(),
            worker_isolation: "compat-jvm".to_string(),
            should_run_after: vec![":test".to_string()],
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "unknown".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
        },
        BuildPlanTask {
            path: ":app:compileJava".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "org.gradle.api.tasks.compile.JavaCompile".to_string(),
            depends_on: vec![":app:generateSources".to_string()],
            inputs: HashMap::from([
                ("source".to_string(), "mock-jvm-task-model".to_string()),
                ("taskType".to_string(), "JavaCompile".to_string()),
                (
                    "mustRunAfter".to_string(),
                    ":app:processResources".to_string(),
                ),
                ("finalizedBy".to_string(), ":app:check".to_string()),
                ("nativeCandidate".to_string(), "true".to_string()),
            ]),
            outputs: vec!["classes/java/main".to_string()],
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: vec![":app:processResources".to_string()],
            finalized_by: vec![":app:check".to_string()],
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
        },
        BuildPlanTask {
            path: ":app:integrationTest".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "org.gradle.api.tasks.testing.Test".to_string(),
            depends_on: vec![":app:test".to_string()],
            inputs: HashMap::from([
                ("source".to_string(), "mock-jvm-task-model".to_string()),
                ("taskType".to_string(), "Test".to_string()),
                ("enabled".to_string(), "false".to_string()),
                ("shouldRunAfter".to_string(), ":app:compileJava".to_string()),
                (
                    "projectDeclaredDependencyCount".to_string(),
                    "1".to_string(),
                ),
                (
                    "projectDependencyConfigurations".to_string(),
                    "implementation".to_string(),
                ),
            ]),
            outputs: vec!["build/test-results/integrationTest".to_string()],
            worker_isolation: "process".to_string(),
            should_run_after: vec![":app:compileJava".to_string()],
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
        },
    ]
}

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
                    build_file: self
                        .repo_root
                        .join("build.gradle.kts")
                        .to_string_lossy()
                        .into_owned(),
                    subprojects: vec![":app".to_string()],
                },
                ProjectModel {
                    path: ":app".to_string(),
                    name: format!("app-{}", build_id),
                    build_file: self
                        .repo_root
                        .join("app")
                        .join("build.gradle.kts")
                        .to_string_lossy()
                        .into_owned(),
                    subprojects: vec![],
                },
            ],
        }))
    }

    async fn get_build_plan(
        &self,
        request: Request<GetBuildPlanRequest>,
    ) -> Result<Response<GetBuildPlanResponse>, Status> {
        Ok(Response::new(GetBuildPlanResponse {
            success: true,
            error_message: String::new(),
            plan: Some(BuildPlan {
                schema_version: BUILD_PLAN_SCHEMA_VERSION,
                build_id: request.into_inner().build_id,
                projects: Vec::new(),
                tasks: mock_build_plan_tasks(),
                dependencies: Vec::new(),
                toolchains: Vec::new(),
                metadata: HashMap::from([("provider".to_string(), "mock-build-plan".to_string())]),
            }),
            source: "mock-jvm-host".to_string(),
        }))
    }

    async fn resolve_configuration(
        &self,
        request: Request<ResolveConfigRequest>,
    ) -> Result<Response<ResolveConfigResponse>, Status> {
        let req = request.into_inner();
        let artifacts = match req.configuration_name.as_str() {
            "compileClasspath" => vec![
                gradle_substrate_daemon::proto::ResolvedArtifact {
                    group: "org.example".to_string(),
                    name: "core-lib".to_string(),
                    version: "1.0.0".to_string(),
                    configuration: req.configuration_name.clone(),
                },
                gradle_substrate_daemon::proto::ResolvedArtifact {
                    group: "org.example".to_string(),
                    name: format!("{}-impl", req.project_path.trim_start_matches(':')),
                    version: "1.0.0".to_string(),
                    configuration: req.configuration_name.clone(),
                },
            ],
            "runtimeClasspath" => vec![gradle_substrate_daemon::proto::ResolvedArtifact {
                group: "org.example".to_string(),
                name: "runtime-lib".to_string(),
                version: "2.0.0".to_string(),
                configuration: req.configuration_name.clone(),
            }],
            _ => Vec::new(),
        };

        Ok(Response::new(ResolveConfigResponse {
            success: true,
            artifacts,
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

    async fn execute_task(
        &self,
        request: Request<ExecuteTaskRequest>,
    ) -> Result<Response<ExecuteTaskResponse>, Status> {
        Ok(Response::new(ExecuteTaskResponse {
            success: false,
            outcome: "UNSUPPORTED".to_string(),
            error_message: format!(
                "Mock JVM host does not execute {}",
                request.into_inner().task_path
            ),
            duration_ms: 0,
            execution_mode: "mock-jvm-unsupported".to_string(),
        }))
    }
}

fn create_mock_repo(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(
        root.join("build.gradle.kts"),
        r#"
            plugins {
                java
            }

            repositories {
                mavenCentral()
            }

            group = "org.example.shadow"
            version = "1.0.0"

            java {
                sourceCompatibility = JavaVersion.VERSION_17
                targetCompatibility = JavaVersion.VERSION_17
            }

            tasks.register("lint") {
                dependsOn("check")
                shouldRunAfter("test")
            }
        "#,
    )
    .unwrap();
    std::fs::write(
        root.join("app").join("build.gradle.kts"),
        r#"
            dependencies {
                implementation("org.example:feature-lib:1.2.3")
            }

            tasks.register<JavaCompile>("compileJava") {
                dependsOn("generateSources")
                mustRunAfter("processResources")
                finalizedBy("check")
                destinationDirectory = layout.buildDirectory.dir("classes/java/main")
            }

            tasks.register<Test>("integrationTest") {
                dependsOn("test")
                shouldRunAfter("compileJava")
                outputs.dir("build/test-results/integrationTest")
                enabled = false
            }
        "#,
    )
    .unwrap();
}

async fn spawn_mock_server() -> (String, tempfile::TempDir, PathBuf) {
    let temp_dir = tempfile::tempdir().unwrap();
    let repo_root = temp_dir.path().join("repo");
    create_mock_repo(&repo_root);
    let socket_path = temp_dir.path().join("jvm-host.sock");

    let uds = UnixListener::bind(&socket_path).unwrap();
    let stream = tokio_stream::wrappers::UnixListenerStream::new(uds);
    let service = MockJvmHostService {
        repo_root: repo_root.clone(),
    };

    tokio::spawn(async move {
        Server::builder()
            .add_service(JvmHostServiceServer::new(service))
            .serve_with_incoming(stream)
            .await
            .unwrap();
    });

    (
        socket_path.to_string_lossy().to_string(),
        temp_dir,
        repo_root,
    )
}

#[tokio::test]
async fn capture_and_persist_shadow_build_plan_artifact() {
    let (socket_path, _tmp_server_dir, _repo_root) = spawn_mock_server().await;
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
    assert_eq!(loaded.plan.toolchains[0].version, "17");
    assert!(
        loaded.plan.tasks.iter().any(|task| task.path == ":lint"),
        "expected root task declarations to be captured"
    );
    assert!(
        loaded
            .plan
            .tasks
            .iter()
            .any(|task| task.path == ":app:integrationTest"),
        "expected subproject task declarations to be captured"
    );
    assert!(
        loaded
            .plan
            .dependencies
            .iter()
            .any(|dep| dep.configuration == "compileClasspath"),
        "expected shadow plan to capture resolved dependencies"
    );
    let expected_dependency_count = loaded.plan.dependencies.len().to_string();
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("dependencyCount")
            .map(String::as_str),
        Some(expected_dependency_count.as_str())
    );
    let expected_task_count = loaded.plan.tasks.len().to_string();
    assert_eq!(
        loaded.plan.metadata.get("taskCount").map(String::as_str),
        Some(expected_task_count.as_str())
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("parsedBuildScriptCount")
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("taskDependencyEdgeCount")
            .map(String::as_str),
        Some("3")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("taskSoftDependencyEdgeCount")
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("taskMustRunAfterEdgeCount")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("taskFinalizerEdgeCount")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("disabledTaskCount")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("dependencyConfigurationCount")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("groupAssignmentCount")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("versionAssignmentCount")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("typedTaskCount")
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("declaredTaskOutputCount")
            .map(String::as_str),
        Some("2")
    );

    let lint_task = loaded
        .plan
        .tasks
        .iter()
        .find(|task| task.path == ":lint")
        .expect("expected lint task");
    assert_eq!(
        lint_task.inputs.get("projectGroup").map(String::as_str),
        Some("org.example.shadow")
    );
    assert_eq!(
        lint_task.inputs.get("projectVersion").map(String::as_str),
        Some("1.0.0")
    );
    assert_eq!(
        lint_task
            .inputs
            .get("projectRepositoryTypes")
            .map(String::as_str),
        Some("maven")
    );
    let integration_test = loaded
        .plan
        .tasks
        .iter()
        .find(|task| task.path == ":app:integrationTest")
        .expect("expected integrationTest task");
    assert_eq!(
        integration_test
            .inputs
            .get("projectDeclaredDependencyCount")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        integration_test
            .inputs
            .get("projectDependencyConfigurations")
            .map(String::as_str),
        Some("implementation")
    );
    assert_eq!(
        integration_test.inputs.get("taskType").map(String::as_str),
        Some("Test")
    );
    assert_eq!(integration_test.worker_isolation, "process");
    assert_eq!(
        integration_test
            .inputs
            .get("shouldRunAfter")
            .map(String::as_str),
        Some(":app:compileJava")
    );
    assert_eq!(
        integration_test.outputs,
        vec!["build/test-results/integrationTest".to_string()]
    );

    let compile_java = loaded
        .plan
        .tasks
        .iter()
        .find(|task| task.path == ":app:compileJava")
        .expect("expected compileJava task");
    assert_eq!(
        compile_java.implementation_id,
        "org.gradle.api.tasks.compile.JavaCompile"
    );
    assert_eq!(compile_java.worker_isolation, "process");
    assert_eq!(
        compile_java.inputs.get("taskType").map(String::as_str),
        Some("JavaCompile")
    );
    assert_eq!(
        compile_java.inputs.get("mustRunAfter").map(String::as_str),
        Some(":app:processResources")
    );
    assert_eq!(
        compile_java.inputs.get("finalizedBy").map(String::as_str),
        Some(":app:check")
    );
    assert_eq!(
        compile_java
            .inputs
            .get("nativeCandidate")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(compile_java.outputs, vec!["classes/java/main".to_string()]);

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
    let (socket_path, _tmp_server_dir, repo_root) = spawn_mock_server().await;
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
            project_dir: repo_root.to_string_lossy().into_owned(),
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
async fn bootstrap_refresh_build_plan_shadow_rewrites_selected_graph_artifact() {
    let (socket_path, _tmp_server_dir, repo_root) = spawn_mock_server().await;
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

    let build_id = "build-refresh-shadow";
    bootstrap
        .init_build(Request::new(InitBuildRequest {
            build_id: build_id.to_string(),
            project_dir: repo_root.to_string_lossy().into_owned(),
            start_time_ms: 1,
            requested_parallelism: 1,
            system_properties: HashMap::new(),
            requested_features: Vec::new(),
            session_id: "sess-refresh".to_string(),
        }))
        .await
        .unwrap();

    let response = bootstrap
        .refresh_build_plan_shadow(Request::new(RefreshBuildPlanShadowRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.build_id, build_id);
    assert!(response.refreshed, "{}", response.error_message);
    assert!(!response.artifact_path.is_empty());

    let artifact = store
        .load_plan(build_id)
        .unwrap()
        .expect("expected refreshed shadow artifact");
    assert_eq!(artifact.plan.build_id, build_id);
    assert_eq!(
        artifact
            .plan
            .metadata
            .get("jvmHost.provider")
            .map(String::as_str),
        Some("mock-build-plan")
    );
    assert_eq!(
        artifact.plan.metadata.get("taskSource").map(String::as_str),
        Some("jvm-host-build-plan")
    );
    assert_eq!(artifact.plan.tasks.len(), mock_build_plan_tasks().len());
}

#[tokio::test]
async fn detect_shadow_mismatch_after_manual_mutation() {
    let (socket_path, _tmp_server_dir, _repo_root) = spawn_mock_server().await;
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
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&artifact_path).unwrap()).unwrap();
    json["plan"]["projects"][0]["name"] = serde_json::Value::String("tampered".to_string());
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();

    let report = verify_shadow_against_jvm(&bridge, &store, build_id)
        .await
        .unwrap();
    assert!(
        !report.is_match(),
        "expected mismatches after mutation, report={:?}",
        report
    );
}
