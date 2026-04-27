use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gradle_substrate_daemon::client::jvm_host::JvmHostClient;
use gradle_substrate_daemon::client::jvm_host_bridge::JvmHostBridge;
use gradle_substrate_daemon::proto::bootstrap_service_server::BootstrapService;
use gradle_substrate_daemon::proto::dag_executor_service_server::DagExecutorService;
use gradle_substrate_daemon::proto::jvm_host_service_server::{
    JvmHostService, JvmHostServiceServer,
};
use gradle_substrate_daemon::proto::{
    BuildPlan, BuildPlanTask, BuildPlanTaskDiagnostic, BuildPlanTaskInputSpec,
    BuildPlanTaskOutputSpec, EvaluateScriptRequest, EvaluateScriptResponse, ExecuteTaskRequest,
    ExecuteTaskResponse, GetBuildEnvironmentRequest, GetBuildEnvironmentResponse,
    GetBuildModelRequest, GetBuildModelResponse, GetBuildPlanRequest, GetBuildPlanResponse,
    InitBuildRequest, ProjectModel, RefreshBuildPlanShadowRequest, ResolveConfigRequest,
    ResolveConfigResponse, RunBuildRequest,
};
use gradle_substrate_daemon::server::bootstrap::BootstrapServiceImpl;
use gradle_substrate_daemon::server::build_plan_ir::BUILD_PLAN_SCHEMA_VERSION;
use gradle_substrate_daemon::server::build_plan_shadow::{
    BuildPlanShadowStore, capture_and_persist_shadow_from_jvm, verify_shadow_against_jvm,
};
use gradle_substrate_daemon::server::dag_executor::DagExecutorServiceImpl;
use gradle_substrate_daemon::server::execution_history::ExecutionHistoryServiceImpl;
use gradle_substrate_daemon::server::execution_plan::ExecutionPlanServiceImpl;
use gradle_substrate_daemon::server::scopes::ScopeRegistry;
use gradle_substrate_daemon::server::task_graph::TaskGraphServiceImpl;
use gradle_substrate_daemon::server::work::WorkerScheduler;
use tokio::net::UnixListener;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

struct MockJvmHostService {
    repo_root: PathBuf,
    native_ready_compile: bool,
    java_home: String,
}

fn input_spec(name: &str, value: &str) -> BuildPlanTaskInputSpec {
    BuildPlanTaskInputSpec {
        name: name.to_string(),
        kind: "value".to_string(),
        value: value.to_string(),
        normalization: "scalar".to_string(),
        optional_input: false,
    }
}

fn output_spec(name: &str, path: &str) -> BuildPlanTaskOutputSpec {
    BuildPlanTaskOutputSpec {
        name: name.to_string(),
        kind: "path".to_string(),
        path: path.to_string(),
    }
}

fn diagnostic(code: &str) -> BuildPlanTaskDiagnostic {
    BuildPlanTaskDiagnostic {
        severity: "info".to_string(),
        code: code.to_string(),
        message: "mock execution contract".to_string(),
        source: "mock-jvm-task-model".to_string(),
    }
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
            action_kind: "jvm-task".to_string(),
            input_specs: vec![input_spec("source", "mock-jvm-task-model")],
            output_specs: Vec::new(),
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: vec![diagnostic("mock-task-contract")],
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
            action_kind: "compile".to_string(),
            input_specs: vec![
                input_spec("source", "mock-jvm-task-model"),
                input_spec("taskType", "JavaCompile"),
            ],
            output_specs: vec![output_spec("classes", "classes/java/main")],
            environment_inputs: vec!["JAVA_HOME".to_string()],
            system_property_inputs: vec!["java.version".to_string()],
            diagnostics: vec![diagnostic("mock-compile-contract")],
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
            action_kind: "test".to_string(),
            input_specs: vec![
                input_spec("source", "mock-jvm-task-model"),
                input_spec("taskType", "Test"),
            ],
            output_specs: vec![output_spec("results", "build/test-results/integrationTest")],
            environment_inputs: Vec::new(),
            system_property_inputs: vec!["junit.platform.output.capture.stdout".to_string()],
            diagnostics: vec![diagnostic("mock-test-contract")],
        },
    ]
}

fn native_ready_compile_task(repo_root: &std::path::Path, java_home: &str) -> BuildPlanTask {
    let source_file = repo_root
        .join("app")
        .join("src")
        .join("main")
        .join("java")
        .join("org")
        .join("gradle")
        .join("substrate")
        .join("corpus")
        .join("HelloCaptured.java");
    let output_dir = repo_root
        .join("app")
        .join("build")
        .join("classes")
        .join("java")
        .join("main");
    BuildPlanTask {
        path: ":app:compileJava".to_string(),
        project_path: ":app".to_string(),
        implementation_id: "org.gradle.api.tasks.compile.JavaCompile".to_string(),
        depends_on: Vec::new(),
        inputs: HashMap::from([
            ("source".to_string(), "mock-jvm-task-model".to_string()),
            ("taskType".to_string(), "JavaCompile".to_string()),
            ("nativeCandidate".to_string(), "true".to_string()),
            ("java_home".to_string(), java_home.to_string()),
        ]),
        outputs: vec![output_dir.to_string_lossy().into_owned()],
        worker_isolation: "process".to_string(),
        should_run_after: Vec::new(),
        must_run_after: Vec::new(),
        finalized_by: Vec::new(),
        cacheability: "declared-outputs".to_string(),
        local_state: Vec::new(),
        destroyables: Vec::new(),
        action_kind: "compile".to_string(),
        input_specs: vec![
            input_spec("source", "mock-jvm-task-model"),
            input_spec("taskType", "JavaCompile"),
            input_spec("nativeCandidate", "true"),
            input_spec("java_home", java_home),
            input_spec("release", "17"),
            BuildPlanTaskInputSpec {
                name: "source0".to_string(),
                kind: "source".to_string(),
                value: source_file.to_string_lossy().into_owned(),
                normalization: "absolute-path".to_string(),
                optional_input: false,
            },
        ],
        output_specs: vec![BuildPlanTaskOutputSpec {
            name: "classes".to_string(),
            kind: "directory".to_string(),
            path: output_dir.to_string_lossy().into_owned(),
        }],
        environment_inputs: vec!["JAVA_HOME".to_string()],
        system_property_inputs: vec!["java.version".to_string()],
        diagnostics: vec![diagnostic("native-ready-compile-contract")],
    }
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
                tasks: if self.native_ready_compile {
                    vec![native_ready_compile_task(&self.repo_root, &self.java_home)]
                } else {
                    mock_build_plan_tasks()
                },
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
    let java_dir = root
        .join("app")
        .join("src")
        .join("main")
        .join("java")
        .join("org")
        .join("gradle")
        .join("substrate")
        .join("corpus");
    std::fs::create_dir_all(&java_dir).unwrap();
    std::fs::write(
        java_dir.join("HelloCaptured.java"),
        r#"
            package org.gradle.substrate.corpus;

            public class HelloCaptured {
                public String message() {
                    return "native-from-shadow";
                }
            }
        "#,
    )
    .unwrap();
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
    spawn_mock_server_with_native_ready_compile(false, String::new()).await
}

async fn spawn_mock_server_with_native_ready_compile(
    native_ready_compile: bool,
    java_home: String,
) -> (String, tempfile::TempDir, PathBuf) {
    let temp_dir = tempfile::tempdir().unwrap();
    let repo_root = temp_dir.path().join("repo");
    create_mock_repo(&repo_root);
    let socket_path = temp_dir.path().join("jvm-host.sock");

    let uds = UnixListener::bind(&socket_path).unwrap();
    let stream = tokio_stream::wrappers::UnixListenerStream::new(uds);
    let service = MockJvmHostService {
        repo_root: repo_root.clone(),
        native_ready_compile,
        java_home,
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
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("taskInputSpecCount")
            .map(String::as_str),
        Some("5")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("taskOutputSpecCount")
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        loaded
            .plan
            .metadata
            .get("taskDiagnosticCount")
            .map(String::as_str),
        Some("3")
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
    assert_eq!(compile_java.action_kind, "compile");
    assert_eq!(
        compile_java.environment_inputs,
        vec!["JAVA_HOME".to_string()]
    );
    assert_eq!(
        compile_java.system_property_inputs,
        vec!["java.version".to_string()]
    );
    assert_eq!(compile_java.output_specs.len(), 1);
    assert!(
        compile_java
            .input_specs
            .iter()
            .any(|input| input.name == "taskType" && input.value == "JavaCompile")
    );

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
async fn refreshed_native_ready_shadow_plan_runs_compile_java_without_jvm_fallback() {
    let java_home = match std::env::var("JAVA_HOME") {
        Ok(value) => value,
        Err(_) => return,
    };
    let (socket_path, _tmp_server_dir, repo_root) =
        spawn_mock_server_with_native_ready_compile(true, java_home).await;
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

    let build_id = "build-native-shadow-java";
    bootstrap
        .init_build(Request::new(InitBuildRequest {
            build_id: build_id.to_string(),
            project_dir: repo_root.to_string_lossy().into_owned(),
            start_time_ms: 1,
            requested_parallelism: 1,
            system_properties: HashMap::new(),
            requested_features: Vec::new(),
            session_id: "sess-native-shadow".to_string(),
        }))
        .await
        .unwrap();

    let refreshed = bootstrap
        .refresh_build_plan_shadow(Request::new(RefreshBuildPlanShadowRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(refreshed.refreshed, "{}", refreshed.error_message);

    let history = Arc::new(ExecutionHistoryServiceImpl::new(
        cache_dir.path().join("history"),
    ));
    let task_graph = Arc::new(TaskGraphServiceImpl::with_history_and_shadow(
        history,
        Arc::clone(&store),
    ));
    let dag = DagExecutorServiceImpl::new(
        Arc::new(WorkerScheduler::new(1)),
        task_graph,
        Arc::new(ExecutionPlanServiceImpl::default()),
        Vec::new(),
    );

    let response = dag
        .run_build(Request::new(RunBuildRequest {
            build_id: build_id.to_string(),
            max_parallelism: 1,
            task_filter: Vec::new(),
            task_contexts: HashMap::new(),
            allow_jvm_forwarding: false,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.final_status, "COMPLETED");
    assert_eq!(response.plan_source, "build-plan-shadow");
    assert_eq!(response.total_tasks, 1);
    assert_eq!(response.tasks_succeeded, 1);
    assert_eq!(response.tasks_forwarded_to_jvm, 0);
    assert_eq!(response.task_details[0].task_path, ":app:compileJava");
    assert_eq!(response.task_details[0].task_type, "JavaCompile");
    assert_eq!(response.task_details[0].execution_mode, "native");
    assert!(
        repo_root
            .join("app")
            .join("build")
            .join("classes")
            .join("java")
            .join("main")
            .join("org")
            .join("gradle")
            .join("substrate")
            .join("corpus")
            .join("HelloCaptured.class")
            .exists(),
        "captured JVM-host JavaCompile contract should produce class output through Rust"
    );
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
