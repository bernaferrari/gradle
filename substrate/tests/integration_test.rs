use std::sync::Arc;

use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::Request;

use gradle_substrate_daemon::proto::*;
use gradle_substrate_daemon::server::{
    artifact_publishing::ArtifactPublishingServiceImpl,
    bootstrap::BootstrapServiceImpl,
    cache_orchestration::BuildCacheOrchestrationServiceImpl,
    build_comparison::BuildComparisonServiceImpl,
    build_event_stream::BuildEventStreamServiceImpl,
    build_init::BuildInitServiceImpl,
    build_layout::BuildLayoutServiceImpl,
    build_operations::BuildOperationsServiceImpl,
    build_result::BuildResultServiceImpl,
    cache::CacheServiceImpl,
    config_cache::ConfigurationCacheServiceImpl,
    configuration::ConfigurationServiceImpl,
    console::ConsoleServiceImpl,
    control::ControlServiceImpl,
    dependency_resolution::DependencyResolutionServiceImpl,
    execution_history::ExecutionHistoryServiceImpl,
    execution_plan::ExecutionPlanServiceImpl,
    exec::ExecServiceImpl,
    file_fingerprint::FileFingerprintServiceImpl,
    file_watch::FileWatchServiceImpl,
    hash::HashServiceImpl,
    incremental_compilation::IncrementalCompilationServiceImpl,
    plugin::PluginServiceImpl,
    problem_reporting::ProblemReportingServiceImpl,
    resource_management::ResourceManagementServiceImpl,
    task_graph::TaskGraphServiceImpl,
    test_execution::TestExecutionServiceImpl,
    toolchain::ToolchainServiceImpl,
    value_snapshot::ValueSnapshotServiceImpl,
    worker_process::WorkerProcessServiceImpl,
    work::{WorkerScheduler, WorkServiceImpl},
};

/// Spawns a full gRPC server on a temp Unix socket and returns the socket path.
/// The returned tempfile::TempDir must be held for the lifetime of the server.
async fn spawn_test_server() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("test.sock");
    let socket_path_str = socket_path.to_string_lossy().to_string();

    let cache_dir = dir.path().join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();

    let history_dir = dir.path().join("history");
    std::fs::create_dir_all(&history_dir).unwrap();

    let config_cache_dir = dir.path().join("config-cache");
    std::fs::create_dir_all(&config_cache_dir).unwrap();

    let toolchain_dir = dir.path().join("toolchains");
    std::fs::create_dir_all(&toolchain_dir).unwrap();

    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    let control = ControlServiceImpl::new(shutdown_tx);
    let hash = HashServiceImpl;
    let cache = CacheServiceImpl::new(cache_dir);
    let exec = ExecServiceImpl::new();
    let work_scheduler = Arc::new(WorkerScheduler::new(4));
    let work = WorkServiceImpl::new(work_scheduler.clone());
    let execution_plan = ExecutionPlanServiceImpl::new(work_scheduler);
    let execution_history = ExecutionHistoryServiceImpl::new(history_dir);
    let cache_orchestration = BuildCacheOrchestrationServiceImpl::new();
    let file_fingerprint = FileFingerprintServiceImpl::new();
    let value_snapshot = ValueSnapshotServiceImpl::new();
    let task_graph = TaskGraphServiceImpl::new();
    let configuration = ConfigurationServiceImpl::new();
    let plugin = PluginServiceImpl::new();
    let build_operations = BuildOperationsServiceImpl::new();
    let bootstrap = BootstrapServiceImpl::new();
    let dependency_resolution = DependencyResolutionServiceImpl::new();
    let file_watch = FileWatchServiceImpl::new();
    let config_cache = ConfigurationCacheServiceImpl::new(config_cache_dir);
    let toolchain = ToolchainServiceImpl::new(toolchain_dir);
    let build_event_stream = BuildEventStreamServiceImpl::new();
    let worker_process = WorkerProcessServiceImpl::new();
    let build_layout = BuildLayoutServiceImpl::new();
    let build_result = BuildResultServiceImpl::new();
    let problem_reporting = ProblemReportingServiceImpl::new();
    let resource_management = ResourceManagementServiceImpl::new();
    let build_comparison = BuildComparisonServiceImpl::new();
    let console = ConsoleServiceImpl::new();
    let test_execution = TestExecutionServiceImpl::new();
    let artifact_publishing = ArtifactPublishingServiceImpl::new();
    let build_init = BuildInitServiceImpl::new();
    let incremental_compilation = IncrementalCompilationServiceImpl::new();

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Spawn server in background
    tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(control_service_server::ControlServiceServer::new(control))
            .add_service(hash_service_server::HashServiceServer::new(hash))
            .add_service(cache_service_server::CacheServiceServer::new(cache))
            .add_service(exec_service_server::ExecServiceServer::new(exec))
            .add_service(work_service_server::WorkServiceServer::new(work))
            .add_service(execution_plan_service_server::ExecutionPlanServiceServer::new(execution_plan))
            .add_service(execution_history_service_server::ExecutionHistoryServiceServer::new(execution_history))
            .add_service(build_cache_orchestration_service_server::BuildCacheOrchestrationServiceServer::new(cache_orchestration))
            .add_service(file_fingerprint_service_server::FileFingerprintServiceServer::new(file_fingerprint))
            .add_service(value_snapshot_service_server::ValueSnapshotServiceServer::new(value_snapshot))
            .add_service(task_graph_service_server::TaskGraphServiceServer::new(task_graph))
            .add_service(configuration_service_server::ConfigurationServiceServer::new(configuration))
            .add_service(plugin_service_server::PluginServiceServer::new(plugin))
            .add_service(build_operations_service_server::BuildOperationsServiceServer::new(build_operations))
            .add_service(bootstrap_service_server::BootstrapServiceServer::new(bootstrap))
            .add_service(dependency_resolution_service_server::DependencyResolutionServiceServer::new(dependency_resolution))
            .add_service(file_watch_service_server::FileWatchServiceServer::new(file_watch))
            .add_service(configuration_cache_service_server::ConfigurationCacheServiceServer::new(config_cache))
            .add_service(toolchain_service_server::ToolchainServiceServer::new(toolchain))
            .add_service(build_event_stream_service_server::BuildEventStreamServiceServer::new(build_event_stream))
            .add_service(worker_process_service_server::WorkerProcessServiceServer::new(worker_process))
            .add_service(build_layout_service_server::BuildLayoutServiceServer::new(build_layout))
            .add_service(build_result_service_server::BuildResultServiceServer::new(build_result))
            .add_service(problem_reporting_service_server::ProblemReportingServiceServer::new(problem_reporting))
            .add_service(resource_management_service_server::ResourceManagementServiceServer::new(resource_management))
            .add_service(build_comparison_service_server::BuildComparisonServiceServer::new(build_comparison))
            .add_service(console_service_server::ConsoleServiceServer::new(console))
            .add_service(test_execution_service_server::TestExecutionServiceServer::new(test_execution))
            .add_service(artifact_publishing_service_server::ArtifactPublishingServiceServer::new(artifact_publishing))
            .add_service(build_init_service_server::BuildInitServiceServer::new(build_init))
            .add_service(incremental_compilation_service_server::IncrementalCompilationServiceServer::new(incremental_compilation))
            .serve_with_incoming(UnixListenerStream::new(listener))
            .await;
    });

    // Wait briefly for server to be ready
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    (socket_path_str, dir)
}

async fn connect(socket_path: &str) -> Channel {
    let path = socket_path.to_string();
    Endpoint::from_shared("http://localhost".to_string())
        .unwrap()
        .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
            let path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(&path).await?;
                let io = hyper_util::rt::TokioIo::new(stream);
                Ok::<_, std::io::Error>(io)
            }
        }))
        .await
        .unwrap()
}

fn make_test_file(dir: &std::path::Path, name: &str, content: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().to_string()
}

fn make_prop_map(pairs: Vec<(&str, &str)>) -> std::collections::HashMap<String, String> {
    pairs.into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ============================================================
// Test 1: Hash service end-to-end
// ============================================================

#[tokio::test]
async fn test_hash_service_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let dir = tempfile::tempdir().unwrap();
    let file_path = make_test_file(dir.path(), "test.txt", b"Hello, integration test!");

    let response = client
        .hash_batch(Request::new(HashBatchRequest {
            files: vec![FileToHash {
                absolute_path: file_path.clone(),
                length: 0,
                last_modified: 0,
            }],
            algorithm: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].absolute_path, file_path);
    assert!(!response.results[0].hash_bytes.is_empty());
    assert!(!response.results[0].error);

    // Verify hash matches direct computation
    let direct_hash = gradle_substrate_daemon::server::hash::hash_file_md5(
        std::path::Path::new(&file_path),
    )
    .unwrap();
    assert_eq!(response.results[0].hash_bytes, direct_hash);
}

// ============================================================
// Test 2: Execution plan predict end-to-end
// ============================================================

#[tokio::test]
async fn test_execution_plan_predict_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = execution_plan_service_client::ExecutionPlanServiceClient::new(channel);

    let response = client
        .predict_outcome(Request::new(PredictOutcomeRequest {
            work: Some(WorkMetadata {
                work_identity: ":compileJava".to_string(),
                display_name: "compileJava".to_string(),
                implementation_class: "com.example.JavaCompile".to_string(),
                input_properties: make_prop_map(vec![("source", "src/main/java")]),
                input_file_fingerprints: make_prop_map(vec![("classpath", "abc123")]),
                caching_enabled: true,
                can_load_from_cache: true,
                has_previous_execution_state: false,
                rebuild_reasons: vec![],
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    // With caching enabled and no history, should predict FROM_CACHE or EXECUTE
    assert!(response.confidence > 0.0);
    assert!(!response.reasoning.is_empty());
}

// ============================================================
// Test 3: Execution plan resolve end-to-end
// ============================================================

#[tokio::test]
async fn test_execution_plan_resolve_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = execution_plan_service_client::ExecutionPlanServiceClient::new(channel);

    let response = client
        .resolve_plan(Request::new(ResolvePlanRequest {
            work: Some(WorkMetadata {
                work_identity: ":compileTest".to_string(),
                display_name: "compileTestJava".to_string(),
                implementation_class: "com.example.JavaCompile".to_string(),
                input_properties: make_prop_map(vec![("source", "src/test/java")]),
                input_file_fingerprints: std::collections::HashMap::new(),
                caching_enabled: false,
                can_load_from_cache: false,
                has_previous_execution_state: false,
                rebuild_reasons: vec!["Test source changed".to_string()],
            }),
            authoritative: true,
        }))
        .await
        .unwrap()
        .into_inner();

    // With rebuild reasons, should resolve to EXECUTE
    assert_eq!(response.action, PlanAction::Execute as i32);
    assert!(response.reasoning.contains("rebuild reason"));
}

// ============================================================
// Test 4: Cache orchestration end-to-end
// ============================================================

#[tokio::test]
async fn test_cache_orchestration_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client =
        build_cache_orchestration_service_client::BuildCacheOrchestrationServiceClient::new(channel);

    // Compute cache key
    let compute_resp = client
        .compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "abc123".to_string(),
            input_property_hashes: make_prop_map(vec![("source", "hash1"), ("target", "hash2")]),
            input_file_hashes: make_prop_map(vec![("classpath", "hash3")]),
            output_property_names: vec!["classes".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!compute_resp.cache_key.is_empty());
    assert!(!compute_resp.cache_key_string.is_empty());

    // Probe: should miss initially
    let probe_miss = client
        .probe_cache(Request::new(ProbeCacheRequest {
            cache_key: compute_resp.cache_key.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!probe_miss.available);

    // Store
    let store_resp = client
        .store_outputs(Request::new(StoreOutputsRequest {
            cache_key: compute_resp.cache_key.clone(),
            execution_time_ms: 500,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(store_resp.success);

    // Probe: should hit now
    let probe_hit = client
        .probe_cache(Request::new(ProbeCacheRequest {
            cache_key: compute_resp.cache_key,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(probe_hit.available);
    assert_eq!(probe_hit.location, "local");
}

// ============================================================
// Test 5: File fingerprint end-to-end
// ============================================================

#[tokio::test]
async fn test_file_fingerprint_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client =
        file_fingerprint_service_client::FileFingerprintServiceClient::new(channel);

    let dir = tempfile::tempdir().unwrap();
    let file_path = make_test_file(dir.path(), "test.txt", b"file fingerprint content");

    let response = client
        .fingerprint_files(Request::new(FingerprintFilesRequest {
            files: vec![FileToFingerprint {
                absolute_path: file_path,
                r#type: FingerprintType::FingerprintFile as i32,
            }],
            normalization_strategy: "ABSOLUTE_PATH".to_string(),
            ignore_patterns: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(response.success);
    assert_eq!(response.entries.len(), 1);
    assert!(!response.collection_hash.is_empty());
    assert_eq!(response.entries[0].size, 24); // "file fingerprint content".len()
}

// ============================================================
// Test 6: Value snapshot end-to-end
// ============================================================

#[tokio::test]
async fn test_value_snapshot_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client =
        value_snapshot_service_client::ValueSnapshotServiceClient::new(channel);

    let response = client
        .snapshot_values(Request::new(SnapshotValuesRequest {
            values: vec![PropertyValue {
                name: "source".to_string(),
                value: Some(property_value::Value::StringValue(
                    "src/main/java".to_string(),
                )),
                type_name: "java.lang.String".to_string(),
            }],
            implementation_fingerprint: "impl-123".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(response.success);
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].name, "source");
    assert!(!response.results[0].fingerprint.is_empty());
    assert!(!response.composite_hash.is_empty());
}

// ============================================================
// Test 7: Configuration end-to-end
// ============================================================

#[tokio::test]
async fn test_configuration_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = configuration_service_client::ConfigurationServiceClient::new(channel);

    // Register project
    let register_resp = client
        .register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/app".to_string(),
            properties: make_prop_map(vec![("version", "1.0.0"), ("group", "com.example")]),
            applied_plugins: vec!["java".to_string(), "application".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(register_resp.success);

    // Resolve property
    let resolve_resp = client
        .resolve_property(Request::new(ResolvePropertyRequest {
            project_path: ":app".to_string(),
            property_name: "version".to_string(),
            requested_by: "test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(resolve_resp.found);
    assert_eq!(resolve_resp.value, "1.0.0");
}

// ============================================================
// Test 8: Task graph end-to-end
// ============================================================

#[tokio::test]
async fn test_task_graph_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = task_graph_service_client::TaskGraphServiceClient::new(channel);

    // Register tasks
    let _ = client
        .register_task(Request::new(RegisterTaskRequest {
            task_path: ":compileJava".to_string(),
            depends_on: vec![":processResources".to_string()],
            task_type: "JavaCompile".to_string(),
            should_execute: true,
        }))
        .await
        .unwrap();

    let _ = client
        .register_task(Request::new(RegisterTaskRequest {
            task_path: ":processResources".to_string(),
            depends_on: vec![],
            task_type: "ProcessResources".to_string(),
            should_execute: true,
        }))
        .await
        .unwrap();

    // Resolve execution plan
    let response = client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: "test-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(response.total_tasks >= 2);
    assert!(!response.has_cycles);
}

// ============================================================
// Test 9: Multi-service sequence (hash -> plan -> orchestration)
// ============================================================

#[tokio::test]
async fn test_multi_service_sequence() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;

    // Step 1: Hash a file via direct service call
    let dir = tempfile::tempdir().unwrap();
    let file_path = make_test_file(dir.path(), "seq.txt", b"multi-service test data");
    let direct_hash = gradle_substrate_daemon::server::hash::hash_file_md5(
        std::path::Path::new(&file_path),
    )
    .unwrap();
    let file_hash: String = direct_hash.iter().map(|b| format!("{:02x}", b)).collect();
    assert!(!file_hash.is_empty());

    // Step 2: Compute cache key using the file hash
    let mut orch_client =
        build_cache_orchestration_service_client::BuildCacheOrchestrationServiceClient::new(
            channel.clone(),
        );
    let cache_key_resp = orch_client
        .compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "impl-abc".to_string(),
            input_property_hashes: std::collections::HashMap::new(),
            input_file_hashes: make_prop_map(vec![("src", &file_hash)]),
            output_property_names: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    let cache_key = cache_key_resp.cache_key;
    assert!(!cache_key.is_empty());

    // Step 3: Execution plan predict
    let mut plan_client =
        execution_plan_service_client::ExecutionPlanServiceClient::new(channel.clone());
    let predict_resp = plan_client
        .predict_outcome(Request::new(PredictOutcomeRequest {
            work: Some(WorkMetadata {
                work_identity: ":compileJava".to_string(),
                display_name: "compileJava".to_string(),
                implementation_class: "JavaCompile".to_string(),
                input_properties: std::collections::HashMap::new(),
                input_file_fingerprints: make_prop_map(vec![("src", &file_hash)]),
                caching_enabled: true,
                can_load_from_cache: true,
                has_previous_execution_state: false,
                rebuild_reasons: vec![],
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(predict_resp.confidence > 0.0);

    // Step 4: Cache orchestration probe then store
    let probe = orch_client
        .probe_cache(Request::new(ProbeCacheRequest {
            cache_key: cache_key.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!probe.available);

    let store = orch_client
        .store_outputs(Request::new(StoreOutputsRequest {
            cache_key,
            execution_time_ms: 200,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(store.success);
}

// ============================================================
// Test 10: Hash service batch with multiple files
// ============================================================

#[tokio::test]
async fn test_hash_batch_multiple_files() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let dir = tempfile::tempdir().unwrap();
    let file1 = make_test_file(dir.path(), "a.txt", b"first file content");
    let file2 = make_test_file(dir.path(), "b.txt", b"second file content");
    let file3 = make_test_file(dir.path(), "c.txt", b"third file content");

    let response = client
        .hash_batch(Request::new(HashBatchRequest {
            files: vec![
                FileToHash { absolute_path: file1, length: 0, last_modified: 0 },
                FileToHash { absolute_path: file2, length: 0, last_modified: 0 },
                FileToHash { absolute_path: file3, length: 0, last_modified: 0 },
            ],
            algorithm: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.results.len(), 3);
    for result in &response.results {
        assert!(!result.hash_bytes.is_empty());
        assert!(!result.error);
    }
}
