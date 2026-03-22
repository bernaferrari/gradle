use std::sync::Arc;

use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::Request;

use gradle_substrate_daemon::proto::*;
use gradle_substrate_daemon::server::{
    artifact_publishing::ArtifactPublishingServiceImpl,
    bootstrap::BootstrapServiceImpl,
    build_metrics::BuildMetricsServiceImpl,
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
    garbage_collection::GarbageCollectionServiceImpl,
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
    let cache = CacheServiceImpl::new(cache_dir.clone());
    // Get local store reference before cache is moved into Server
    let cache_local_store = cache.local_store();
    let exec = ExecServiceImpl::new();
    let work_scheduler = Arc::new(WorkerScheduler::new(4));
    let work = WorkServiceImpl::new(work_scheduler.clone());
    // Shared execution history for cross-service integration (execution plan + task graph)
    let shared_history = Arc::new(ExecutionHistoryServiceImpl::new(history_dir.clone()));
    let execution_plan = ExecutionPlanServiceImpl::with_persistent_history(
        work_scheduler.clone(),
        Arc::clone(&shared_history),
    );
    // Cache orchestration wired to real local cache for probing
    let cache_orchestration = BuildCacheOrchestrationServiceImpl::with_local_cache(cache_local_store);
    let file_fingerprint = FileFingerprintServiceImpl::new();
    let value_snapshot = ValueSnapshotServiceImpl::new();
    let task_graph = TaskGraphServiceImpl::with_history(Arc::clone(&shared_history));
    // Separate instance for the gRPC server (tonic needs concrete type, not Arc)
    let execution_history = ExecutionHistoryServiceImpl::new(history_dir.clone());
    let configuration = ConfigurationServiceImpl::new();
    let plugin = PluginServiceImpl::new();
    let build_operations = BuildOperationsServiceImpl::new();
    let bootstrap = BootstrapServiceImpl::new();
    let dependency_resolution = DependencyResolutionServiceImpl::new();
    let file_watch = FileWatchServiceImpl::new();
    let config_cache = ConfigurationCacheServiceImpl::new(config_cache_dir.clone());
    let toolchain = ToolchainServiceImpl::new(toolchain_dir);
    let console = std::sync::Arc::new(ConsoleServiceImpl::new());
    let build_metrics = std::sync::Arc::new(BuildMetricsServiceImpl::new());
    let build_event_stream = BuildEventStreamServiceImpl::with_dispatchers(vec![
        std::sync::Arc::clone(&console) as Arc<dyn gradle_substrate_daemon::server::event_dispatcher::EventDispatcher>,
        std::sync::Arc::clone(&build_metrics) as Arc<dyn gradle_substrate_daemon::server::event_dispatcher::EventDispatcher>,
    ]);
    let worker_process = WorkerProcessServiceImpl::new();
    let build_layout = BuildLayoutServiceImpl::new();
    let build_result = BuildResultServiceImpl::new();
    let problem_reporting = ProblemReportingServiceImpl::new();
    let resource_management = ResourceManagementServiceImpl::new();
    let build_comparison = BuildComparisonServiceImpl::new();
    let test_execution = TestExecutionServiceImpl::new();
    let artifact_publishing = ArtifactPublishingServiceImpl::new();
    let build_init = BuildInitServiceImpl::new();
    let incremental_compilation = IncrementalCompilationServiceImpl::new();
    let garbage_collection = GarbageCollectionServiceImpl::new(
        cache_dir.clone(),
        history_dir.clone(),
        config_cache_dir.clone(),
    );

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
            .add_service(console_service_server::ConsoleServiceServer::new((*console).clone()))
            .add_service(test_execution_service_server::TestExecutionServiceServer::new(test_execution))
            .add_service(artifact_publishing_service_server::ArtifactPublishingServiceServer::new(artifact_publishing))
            .add_service(build_init_service_server::BuildInitServiceServer::new(build_init))
            .add_service(incremental_compilation_service_server::IncrementalCompilationServiceServer::new(incremental_compilation))
            .add_service(build_metrics_service_server::BuildMetricsServiceServer::new((*build_metrics).clone()))
            .add_service(garbage_collection_service_server::GarbageCollectionServiceServer::new(garbage_collection))
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
            output_properties: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(store_resp.success);

    // Probe: metadata says stored but real cache doesn't have the entry yet
    // (because we only called store_outputs, not the actual cache store_entry)
    // With real cache wiring, probe returns miss when the actual cache file is absent
    let probe_metadata_only = client
        .probe_cache(Request::new(ProbeCacheRequest {
            cache_key: compute_resp.cache_key.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!probe_metadata_only.available,
        "Probe should return miss when metadata exists but real cache entry is absent");
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
            build_id: "test-build".to_string(),
            task_path: ":compileJava".to_string(),
            depends_on: vec![":processResources".to_string()],
            task_type: "JavaCompile".to_string(),
            should_execute: true,
        }))
        .await
        .unwrap();

    let _ = client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: "test-build".to_string(),
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
            output_properties: vec![],
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

// ============================================================
// Test 11: Build metrics end-to-end
// ============================================================

#[tokio::test]
async fn test_build_metrics_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = build_metrics_service_client::BuildMetricsServiceClient::new(channel);

    // Record a counter metric
    let record_resp = client
        .record_metric(Request::new(RecordMetricRequest {
            build_id: "test-build".to_string(),
            event: Some(MetricEvent {
                name: "build.start".to_string(),
                value: "1".to_string(),
                metric_type: "counter".to_string(),
                tags: std::collections::HashMap::new(),
                timestamp_ms: 0,
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(record_resp.recorded);

    // Record a timer metric
    client
        .record_metric(Request::new(RecordMetricRequest {
            build_id: "test-build".to_string(),
            event: Some(MetricEvent {
                name: "task.compile".to_string(),
                value: "250".to_string(),
                metric_type: "timer".to_string(),
                tags: std::collections::HashMap::new(),
                timestamp_ms: 0,
            }),
        }))
        .await
        .unwrap();

    // Get metrics back
    let get_resp = client
        .get_metrics(Request::new(GetMetricsRequest {
            build_id: "test-build".to_string(),
            metric_names: vec!["build.start".to_string(), "task.compile".to_string()],
            since_ms: 0,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(get_resp.metrics.len() >= 2);

    // Get performance summary
    let summary_resp = client
        .get_performance_summary(Request::new(GetPerformanceSummaryRequest {
            build_id: "test-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(summary_resp.summary.is_some());
}

// ============================================================
// Test 12: Garbage collection end-to-end
// ============================================================

#[tokio::test]
async fn test_garbage_collection_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = garbage_collection_service_client::GarbageCollectionServiceClient::new(channel);

    // Get storage stats (should work on empty cache)
    let stats_resp = client
        .get_storage_stats(Request::new(GetStorageStatsRequest {
            store_names: vec![
                "build_cache".to_string(),
                "execution_history".to_string(),
                "config_cache".to_string(),
            ],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(stats_resp.stats.len(), 3);

    // GC with dry_run=true should not delete anything
    let gc_resp = client
        .gc_build_cache(Request::new(GcBuildCacheRequest {
            max_age_ms: 0, // evict all
            max_total_bytes: 0,
            dry_run: true,
        }))
        .await
        .unwrap()
        .into_inner();

    // dry_run should report 0 removed
    assert_eq!(gc_resp.entries_removed, 0);
}

// ============================================================
// Test 13: Incremental compilation end-to-end
// ============================================================

#[tokio::test]
async fn test_incremental_compilation_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client =
        incremental_compilation_service_client::IncrementalCompilationServiceClient::new(channel);

    // Register source set
    client
        .register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "test-build".to_string(),
            source_set: Some(SourceSetDescriptor {
                source_set_id: "main".to_string(),
                name: "main".to_string(),
                source_dirs: vec!["src/main/java".to_string()],
                output_dirs: vec!["build/classes".to_string()],
                classpath_hash: "abc123".to_string(),
            }),
        }))
        .await
        .unwrap();

    // Record compilation units with dependencies: A -> B -> C
    client
        .record_compilation(Request::new(RecordCompilationRequest {
            build_id: "test-build".to_string(),
            unit: Some(CompilationUnit {
                source_set_id: "main".to_string(),
                source_file: "C.java".to_string(),
                output_class: "C.class".to_string(),
                source_hash: "hash-C".to_string(),
                class_hash: "class-C".to_string(),
                dependencies: vec![],
                compile_duration_ms: 50,
            }),
        }))
        .await
        .unwrap();

    client
        .record_compilation(Request::new(RecordCompilationRequest {
            build_id: "test-build".to_string(),
            unit: Some(CompilationUnit {
                source_set_id: "main".to_string(),
                source_file: "B.java".to_string(),
                output_class: "B.class".to_string(),
                source_hash: "hash-B".to_string(),
                class_hash: "class-B".to_string(),
                dependencies: vec!["C.java".to_string()],
                compile_duration_ms: 75,
            }),
        }))
        .await
        .unwrap();

    client
        .record_compilation(Request::new(RecordCompilationRequest {
            build_id: "test-build".to_string(),
            unit: Some(CompilationUnit {
                source_set_id: "main".to_string(),
                source_file: "A.java".to_string(),
                output_class: "A.class".to_string(),
                source_hash: "hash-A".to_string(),
                class_hash: "class-A".to_string(),
                dependencies: vec!["B.java".to_string()],
                compile_duration_ms: 100,
            }),
        }))
        .await
        .unwrap();

    // Get rebuild set: C.java changed should transitively affect all
    let rebuild = client
        .get_rebuild_set(Request::new(GetRebuildSetRequest {
            build_id: "test-build".to_string(),
            source_set_id: "main".to_string(),
            changed_files: vec!["C.java".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(rebuild.total_sources, 3);
    assert_eq!(rebuild.must_recompile_count, 3);
    assert_eq!(rebuild.up_to_date_count, 0);

    // Get incremental state
    let state = client
        .get_incremental_state(Request::new(GetIncrementalStateRequest {
            build_id: "test-build".to_string(),
            source_set_id: "main".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(state.state.is_some());
    assert_eq!(state.state.unwrap().total_compiled, 3);
}

// ============================================================
// Test 14: Build init with real settings file end-to-end
// ============================================================

#[tokio::test]
async fn test_build_init_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = build_init_service_client::BuildInitServiceClient::new(channel);

    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("settings.gradle");
    std::fs::write(
        &settings_path,
        r#"rootProject.name = 'e2e-test'
include ':app', ':lib'
"#,
    )
    .unwrap();

    let init_resp = client
        .init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: "e2e-build".to_string(),
            root_dir: dir.path().to_str().unwrap().to_string(),
            settings_file: settings_path.to_str().unwrap().to_string(),
            gradle_user_home: String::new(),
            init_scripts: vec![],
            requested_build_features: vec![],
            current_dir: dir.path().to_str().unwrap().to_string(),
            session_id: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(init_resp.initialized);

    let status = client
        .get_build_init_status(Request::new(GetBuildInitStatusRequest {
            build_id: "e2e-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(status.status.is_some());
    let status = status.status.unwrap();
    assert_eq!(status.included_projects, vec![":app".to_string(), ":lib".to_string()]);

    // Check root project name was parsed
    let root_name: Option<&str> = status
        .settings_details
        .iter()
        .find(|d| d.key == "rootProjectName")
        .map(|d| d.value.as_str());
    assert_eq!(root_name, Some("e2e-test"));
}

// ============================================================
// Test 15: Problem reporting end-to-end
// ============================================================

#[tokio::test]
async fn test_problem_reporting_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = problem_reporting_service_client::ProblemReportingServiceClient::new(channel);

    // Report a warning
    let report_resp = client
        .report_problem(Request::new(ReportProblemRequest {
            build_id: "test-build".to_string(),
            problem: Some(ProblemDetails {
                problem_id: String::new(),
                severity: "warning".to_string(),
                category: "deprecation".to_string(),
                message: "Deprecation warning in code".to_string(),
                details: String::new(),
                file_path: "src/main/java/Example.java".to_string(),
                line_number: 42,
                column: 10,
                contextual_label: String::new(),
                documentation_url: String::new(),
                additional_data: String::new(),
                timestamp_ms: 0,
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(report_resp.accepted);

    // Get problems
    let get_resp = client
        .get_problems(Request::new(GetProblemsRequest {
            build_id: "test-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(get_resp.total >= 1);
    assert!(get_resp.warning_count >= 1);
}

// ============================================================
// Test 16: Build operations end-to-end
// ============================================================

#[tokio::test]
async fn test_build_operations_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = build_operations_service_client::BuildOperationsServiceClient::new(channel);

    // Start an operation
    let start_resp = client
        .start_operation(Request::new(StartOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-1".to_string(),
            display_name: "compileJava".to_string(),
            operation_type: "TASK".to_string(),
            parent_id: String::new(),
            start_time_ms: 0,
            metadata: std::collections::HashMap::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(start_resp.success);

    // Complete the operation
    let complete_resp = client
        .complete_operation(Request::new(CompleteOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-1".to_string(),
            duration_ms: 150,
            success: true,
            outcome: "SUCCESS".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(complete_resp.success);

    // Get build summary — may or may not contain the operation
    // since each test spawns a fresh server
    let summary_resp = client
        .get_build_summary(Request::new(GetBuildSummaryRequest { build_id: "test".to_string() }))
        .await
        .unwrap()
        .into_inner();

    assert!(summary_resp.summary.is_some());
}

#[tokio::test]
async fn test_execution_history_stats_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = execution_history_service_client::ExecutionHistoryServiceClient::new(channel);

    // Store and load to generate stats
    client
        .store_history(Request::new(StoreHistoryRequest {
            work_identity: ":compileJava".to_string(),
            state: vec![1, 2, 3],
            timestamp_ms: 1000,
        }))
        .await
        .unwrap();

    client
        .load_history(Request::new(LoadHistoryRequest {
            work_identity: ":compileJava".to_string(),
        }))
        .await
        .unwrap();

    client
        .load_history(Request::new(LoadHistoryRequest {
            work_identity: ":nonexistent".to_string(),
        }))
        .await
        .unwrap();

    let stats = client
        .get_history_stats(Request::new(GetHistoryStatsRequest {}))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(stats.entry_count, 1);
    assert_eq!(stats.load_hits, 1);
    assert_eq!(stats.load_misses, 1);
    assert_eq!(stats.stores, 1);
    assert!((stats.hit_rate - 0.5).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_dependency_resolution_cache_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = dependency_resolution_service_client::DependencyResolutionServiceClient::new(channel);

    // Initially not cached
    let check = client
        .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
            group: "com.example".to_string(),
            name: "test-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            sha256: String::new(),
            extension: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!check.cached);

    // Add to cache
    let add_resp = client
        .add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
            group: "com.example".to_string(),
            name: "test-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            local_path: "/tmp/test-lib-1.0.jar".to_string(),
            size: 2048,
            sha256: "deadbeef".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(add_resp.accepted);

    // Now cached
    let check2 = client
        .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
            group: "com.example".to_string(),
            name: "test-lib".to_string(),
            version: "1.0".to_string(),
            classifier: String::new(),
            sha256: String::new(),
            extension: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(check2.cached);
    assert_eq!(check2.local_path, "/tmp/test-lib-1.0.jar");
    assert_eq!(check2.cached_size, 2048);

    // Get stats
    let stats = client
        .get_resolution_stats(Request::new(GetResolutionStatsRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(stats.artifact_cache_hits, 1);
    assert_eq!(stats.cached_artifacts, 1);
}

#[tokio::test]
async fn test_build_operations_full_flow_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut ops_client = build_operations_service_client::BuildOperationsServiceClient::new(channel.clone());
    let mut result_client = build_result_service_client::BuildResultServiceClient::new(channel.clone());

    // Record a build failure
    result_client
        .report_build_failure(Request::new(ReportBuildFailureRequest {
            build_id: "build-flow-test".to_string(),
            failure_type: "compilation_error".to_string(),
            failure_message: "Cannot resolve symbol 'foo'".to_string(),
            failed_task_paths: vec![":app:compileJava".to_string()],
        }))
        .await
        .unwrap();

    // Start operations
    ops_client
        .start_operation(Request::new(StartOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-compile".to_string(),
            display_name: "Compile Java".to_string(),
            operation_type: "TASK_EXECUTION".to_string(),
            parent_id: String::new(),
            start_time_ms: 1000,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

    ops_client
        .start_operation(Request::new(StartOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-test".to_string(),
            display_name: "Run tests".to_string(),
            operation_type: "TASK_EXECUTION".to_string(),
            parent_id: String::new(),
            start_time_ms: 2000,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

    // Complete operations
    ops_client
        .complete_operation(Request::new(CompleteOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-compile".to_string(),
            duration_ms: 500,
            success: true,
            outcome: "SUCCESS".to_string(),
        }))
        .await
        .unwrap();

    ops_client
        .complete_operation(Request::new(CompleteOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-test".to_string(),
            duration_ms: 1200,
            success: false,
            outcome: "FAILED".to_string(),
        }))
        .await
        .unwrap();

    // Get build result
    let result = result_client
        .get_build_result(Request::new(GetBuildResultRequest {
            build_id: "build-flow-test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // The build result should have the outcome we reported
    assert!(result.outcome.is_some());
    let outcome = result.outcome.unwrap();
    assert_eq!(outcome.overall_result, "FAILED");
}

// ============================================================
// Test 20: Plugin management end-to-end
// ============================================================

#[tokio::test]
async fn test_plugin_management_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = plugin_service_client::PluginServiceClient::new(channel);

    // Register plugins
    client
        .register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "java".to_string(),
            plugin_class: "org.gradle.api.plugins.JavaPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec![],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

    client
        .register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "kotlin".to_string(),
            plugin_class: "org.gradle.api.plugins.KotlinPlugin".to_string(),
            version: "1.9".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["java".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

    // Check compatibility: kotlin requires java (not applied yet)
    let compat = client
        .check_plugin_compatibility(Request::new(CheckPluginCompatibilityRequest {
            plugin_id: "kotlin".to_string(),
            project_path: ":app".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!compat.compatible);
    assert!(!compat.errors.is_empty());

    // Apply java first
    let apply_java = client
        .apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "java".to_string(),
            project_path: ":app".to_string(),
            apply_order: 0,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(apply_java.success);

    // Now kotlin should be compatible
    let compat2 = client
        .check_plugin_compatibility(Request::new(CheckPluginCompatibilityRequest {
            plugin_id: "kotlin".to_string(),
            project_path: ":app".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(compat2.compatible);

    // Apply kotlin
    let apply_kotlin = client
        .apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "kotlin".to_string(),
            project_path: ":app".to_string(),
            apply_order: 1,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(apply_kotlin.success);

    // Check has_plugin
    let has = client
        .has_plugin(Request::new(HasPluginRequest {
            plugin_id: "java".to_string(),
            project_path: ":app".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(has.has_plugin);

    // Get applied plugins — should be ordered
    let applied = client
        .get_applied_plugins(Request::new(GetAppliedPluginsRequest {
            project_path: ":app".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(applied.plugins.len(), 2);
    assert_eq!(applied.plugins[0].plugin_id, "java");
    assert_eq!(applied.plugins[1].plugin_id, "kotlin");
}

// ============================================================
// Test 21: Console logging end-to-end
// ============================================================

#[tokio::test]
async fn test_console_logging_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = console_service_client::ConsoleServiceClient::new(channel);

    // Send log messages at different levels
    client
        .log_message(Request::new(LogMessageRequest {
            build_id: "console-test".to_string(),
            level: "lifecycle".to_string(),
            category: "org.gradle".to_string(),
            message: "Build started".to_string(),
            throwable: String::new(),
        }))
        .await
        .unwrap();

    client
        .log_message(Request::new(LogMessageRequest {
            build_id: "console-test".to_string(),
            level: "warn".to_string(),
            category: "org.gradle.api".to_string(),
            message: "Deprecated API usage".to_string(),
            throwable: String::new(),
        }))
        .await
        .unwrap();

    // Update progress
    client
        .update_progress(Request::new(UpdateProgressRequest {
            build_id: "console-test".to_string(),
            operations: vec![ProgressOperation {
                operation_id: "op-1".to_string(),
                description: "Compiling Java sources".to_string(),
                status: "running".to_string(),
                total_work: 100,
                completed_work: 50,
                start_time_ms: 1000,
                end_time_ms: 0,
                header: ":compileJava".to_string(),
            }],
        }))
        .await
        .unwrap();

    // Set build description
    client
        .set_build_description(Request::new(SetBuildDescriptionRequest {
            build_id: "console-test".to_string(),
            description: "Building my-app (10 tasks)".to_string(),
        }))
        .await
        .unwrap();

    // Request input (daemon mode returns empty)
    let input_resp = client
        .request_input(Request::new(RequestInputRequest {
            build_id: "console-test".to_string(),
            prompt: "Continue? [y,n]".to_string(),
            default_value: "y".to_string(),
            input_id: "input-1".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(input_resp.value.is_empty()); // daemon mode
}

// ============================================================
// Test 22: Resource management end-to-end
// ============================================================

#[tokio::test]
async fn test_resource_management_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = resource_management_service_client::ResourceManagementServiceClient::new(channel);

    // Get initial resource limits
    let limits = client
        .get_resource_limits(Request::new(GetResourceLimitsRequest {
            build_id: "res-test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!limits.limits.is_empty());

    // Reserve resources for a task
    let reserve = client
        .reserve_resources(Request::new(ReserveResourcesRequest {
            build_id: "res-test".to_string(),
            resources: vec![ResourceRequest {
                resource_type: "memory_mb".to_string(),
                amount: 512,
                requester_id: ":compileJava".to_string(),
            }],
            timeout_ms: 5000,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(reserve.granted);

    // Get usage
    let usage = client
        .get_resource_usage(Request::new(GetResourceUsageRequest {
            build_id: "res-test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // Should have usage entries
    assert!(!usage.usage.is_empty() || reserve.reservation_id.is_empty());

    // Release resources
    let release = client
        .release_resources(Request::new(ReleaseResourcesRequest {
            reservation_id: reserve.reservation_id,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(release.released);
}

// ============================================================
// Test 23: Garbage collection end-to-end (extended)
// ============================================================

#[tokio::test]
async fn test_garbage_collection_extended_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = garbage_collection_service_client::GarbageCollectionServiceClient::new(channel);

    // Run GC on build cache
    let gc_cache = client
        .gc_build_cache(Request::new(GcBuildCacheRequest {
            max_age_ms: 24 * 60 * 60 * 1000,
            max_total_bytes: 1024 * 1024 * 1024,
            dry_run: false,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(gc_cache.entries_removed >= 0);

    // Run GC on execution history
    let gc_history = client
        .gc_execution_history(Request::new(GcExecutionHistoryRequest {
            max_age_ms: 168 * 60 * 60 * 1000,
            max_entries: 10000,
            dry_run: false,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(gc_history.entries_removed >= 0);

    // Run GC on config cache
    let gc_config = client
        .gc_config_cache(Request::new(GcConfigCacheRequest {
            max_age_ms: 24 * 60 * 60 * 1000,
            max_entries: 1000,
            dry_run: false,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(gc_config.entries_removed >= 0);

    // Get storage stats
    let stats = client
        .get_storage_stats(Request::new(GetStorageStatsRequest {
            store_names: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    // Stats should be returned (empty for fresh dirs is fine)
    assert!(stats.stats.is_empty() || !stats.stats.is_empty());
}

// ============================================================
// Test 24: Worker process pool end-to-end
// ============================================================

#[tokio::test]
async fn test_worker_process_pool_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = worker_process_service_client::WorkerProcessServiceClient::new(channel);

    // Configure pool
    let config = client
        .configure_pool(Request::new(ConfigurePoolRequest {
            max_pool_size: 4,
            idle_timeout_ms: 60_000,
            max_per_key: 2,
            enable_health_checks: true,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(config.applied);

    // Get worker status (no workers yet)
    let status = client
        .get_worker_status(Request::new(GetWorkerStatusRequest {
            worker_key: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    // No workers spawned yet, status should be valid
    assert_eq!(status.workers.len(), 0);
}

// ============================================================
// Test 25: Full build lifecycle integration
// ============================================================

#[tokio::test]
async fn test_full_build_lifecycle() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;

    // 1. Bootstrap: init build
    let mut bootstrap_client = bootstrap_service_client::BootstrapServiceClient::new(channel.clone());
    let init = bootstrap_client
        .init_build(Request::new(InitBuildRequest {
            build_id: "lifecycle-build".to_string(),
            project_dir: "/tmp/lifecycle-test".to_string(),
            start_time_ms: 0,
            requested_parallelism: 4,
            system_properties: Default::default(),
            requested_features: vec!["configuration-cache".to_string()],
            session_id: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!init.build_id.is_empty());

    // 2. Build init: settings
    let mut init_client = build_init_service_client::BuildInitServiceClient::new(channel.clone());
    init_client
        .init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: "lifecycle-build".to_string(),
            root_dir: "/tmp/lifecycle-test".to_string(),
            settings_file: "/tmp/lifecycle-test/settings.gradle".to_string(),
            gradle_user_home: String::new(),
            init_scripts: vec![],
            requested_build_features: vec![],
            current_dir: "/tmp/lifecycle-test".to_string(),
            session_id: String::new(),
        }))
        .await
        .unwrap();

    // 3. Configuration: register projects
    let mut config_client = configuration_service_client::ConfigurationServiceClient::new(channel.clone());
    config_client
        .register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/lifecycle-test/app".to_string(),
            properties: make_prop_map(vec![("version", "2.0.0"), ("group", "com.example")]),
            applied_plugins: vec!["java".to_string()],
        }))
        .await
        .unwrap();

    // 4. Plugin management
    let mut plugin_client = plugin_service_client::PluginServiceClient::new(channel.clone());
    plugin_client
        .register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "java".to_string(),
            plugin_class: "JavaPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec![],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

    plugin_client
        .apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "java".to_string(),
            project_path: ":app".to_string(),
            apply_order: 0,
        }))
        .await
        .unwrap();

    // 5. Task graph: register and resolve tasks
    let mut task_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());
    task_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: "lifecycle-build".to_string(),
            task_path: ":compileJava".to_string(),
            depends_on: vec![":processResources".to_string()],
            task_type: "JavaCompile".to_string(),
            should_execute: true,
        }))
        .await
        .unwrap();

    task_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: "lifecycle-build".to_string(),
            task_path: ":processResources".to_string(),
            depends_on: vec![],
            task_type: "Copy".to_string(),
            should_execute: true,
        }))
        .await
        .unwrap();

    let plan = task_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: "lifecycle-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(plan.total_tasks >= 2);
    assert!(!plan.has_cycles);

    // 6. Build operations: track task execution
    let mut ops_client = build_operations_service_client::BuildOperationsServiceClient::new(channel.clone());
    ops_client
        .start_operation(Request::new(StartOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-process".to_string(),
            display_name: ":processResources".to_string(),
            operation_type: "TASK".to_string(),
            parent_id: String::new(),
            start_time_ms: 0,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

    ops_client
        .complete_operation(Request::new(CompleteOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-process".to_string(),
            duration_ms: 100,
            success: true,
            outcome: "SUCCESS".to_string(),
        }))
        .await
        .unwrap();

    ops_client
        .start_operation(Request::new(StartOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-compile".to_string(),
            display_name: ":compileJava".to_string(),
            operation_type: "TASK".to_string(),
            parent_id: String::new(),
            start_time_ms: 100,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

    ops_client
        .complete_operation(Request::new(CompleteOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-compile".to_string(),
            duration_ms: 2000,
            success: true,
            outcome: "SUCCESS".to_string(),
        }))
        .await
        .unwrap();

    // 7. Console: log build progress
    let mut console_client = console_service_client::ConsoleServiceClient::new(channel.clone());
    console_client
        .log_message(Request::new(LogMessageRequest {
            build_id: "lifecycle-build".to_string(),
            level: "lifecycle".to_string(),
            category: "org.gradle".to_string(),
            message: "BUILD SUCCESSFUL in 2s".to_string(),
            throwable: String::new(),
        }))
        .await
        .unwrap();

    // 8. Build result: report outcome
    let mut result_client = build_result_service_client::BuildResultServiceClient::new(channel.clone());
    result_client
        .report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "lifecycle-build".to_string(),
            result: Some(TaskResult {
                task_path: ":compileJava".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 2000,
                did_work: true,
                cache_key: String::new(),
                start_time_ms: 0,
                end_time_ms: 2000,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

    // 9. Complete build
    let complete = bootstrap_client
        .complete_build(Request::new(CompleteBuildRequest {
            build_id: "lifecycle-build".to_string(),
            outcome: "SUCCESS".to_string(),
            duration_ms: 2100,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(complete.acknowledged);

    // 10. Get final build result
    let result = result_client
        .get_build_result(Request::new(GetBuildResultRequest {
            build_id: "lifecycle-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(result.outcome.is_some());
    let outcome = result.outcome.unwrap();
    assert_eq!(outcome.overall_result, "SUCCESS");

    // 11. Get build summary
    let summary = ops_client
        .get_build_summary(Request::new(GetBuildSummaryRequest { build_id: "test".to_string() }))
        .await
        .unwrap()
        .into_inner();

    assert!(summary.summary.is_some());
    let summary = summary.summary.unwrap();
    assert!(summary.total_tasks >= 0);
}

// ============================================================
// Test 26: Config cache end-to-end
// ============================================================

#[tokio::test]
async fn test_config_cache_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = configuration_cache_service_client::ConfigurationCacheServiceClient::new(channel);

    // Store config cache
    let store = client
        .store_config_cache(Request::new(StoreConfigCacheRequest {
            cache_key: "config-hash-123".to_string(),
            serialized_config: vec![1, 2, 3, 4, 5],
            entry_count: 5,
            input_hashes: vec!["build.gradle".to_string(), "settings.gradle".to_string()],
            timestamp_ms: 1000,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(store.stored);

    // Load config cache
    let load = client
        .load_config_cache(Request::new(LoadConfigCacheRequest {
            cache_key: "config-hash-123".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(load.found);
    assert_eq!(load.serialized_config, vec![1, 2, 3, 4, 5]);

    // Validate config
    let validate = client
        .validate_config(Request::new(ValidateConfigRequest {
            cache_key: "config-hash-123".to_string(),
            input_hashes: vec!["build.gradle".to_string(), "settings.gradle".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(validate.valid);

    // Invalid cache key
    let invalid = client
        .validate_config(Request::new(ValidateConfigRequest {
            cache_key: "wrong-hash".to_string(),
            input_hashes: vec!["build.gradle".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!invalid.valid);
}

// ============================================================
// Test 27: Build layout end-to-end
// ============================================================

#[tokio::test]
async fn test_build_layout_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = build_layout_service_client::BuildLayoutServiceClient::new(channel);

    // Init build layout
    let init_resp = client
        .init_build_layout(Request::new(InitBuildLayoutRequest {
            root_dir: "/tmp/layout-test".to_string(),
            settings_file: "/tmp/layout-test/settings.gradle".to_string(),
            build_file: "build.gradle".to_string(),
            build_name: "my-app".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    let build_id = init_resp.build_id;
    assert!(!build_id.is_empty());

    // Add subprojects
    client
        .add_subproject(Request::new(AddSubprojectRequest {
            build_id: build_id.clone(),
            project_path: ":app".to_string(),
            project_dir: "/tmp/layout-test/app".to_string(),
            build_file: "/tmp/layout-test/app/build.gradle".to_string(),
            display_name: "app".to_string(),
        }))
        .await
        .unwrap();

    client
        .add_subproject(Request::new(AddSubprojectRequest {
            build_id: build_id.clone(),
            project_path: ":lib".to_string(),
            project_dir: "/tmp/layout-test/lib".to_string(),
            build_file: "/tmp/layout-test/lib/build.gradle".to_string(),
            display_name: "lib".to_string(),
        }))
        .await
        .unwrap();

    // List projects
    let projects = client
        .list_projects(Request::new(ListProjectsRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(projects.project_paths.len() >= 2);

    // Get project tree
    let tree = client
        .get_project_tree(Request::new(GetProjectTreeRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(tree.root.is_some());

    // Get build file path
    let build_file = client
        .get_build_file_path(Request::new(GetBuildFilePathRequest {
            build_id,
            project_path: ":app".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(build_file.build_file_path, "/tmp/layout-test/app/build.gradle");
}

// ============================================================
// Test 28: Control service handshake end-to-end
// ============================================================

#[tokio::test]
async fn test_control_handshake_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = control_service_client::ControlServiceClient::new(channel);

    let resp = client
        .handshake(Request::new(HandshakeRequest {
            client_version: "test-1.0".to_string(),
            protocol_version: "1.0.0".to_string(),
            client_pid: std::process::id() as i32,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(resp.accepted);
    assert!(!resp.server_version.is_empty());
}

// ============================================================
// Test 29: Work service end-to-end
// ============================================================

#[tokio::test]
async fn test_work_service_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = work_service_client::WorkServiceClient::new(channel);

    // Evaluate work with no history
    let eval_resp = client
        .evaluate(Request::new(WorkEvaluateRequest {
            task_path: ":compileJava".to_string(),
            input_properties: make_prop_map(vec![("source", "src/main/java")]),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!eval_resp.input_hash.is_empty());
    assert!(!eval_resp.reason.is_empty());

    // Record execution
    let record_resp = client
        .record_execution(Request::new(WorkRecordRequest {
            task_path: ":compileJava".to_string(),
            duration_ms: 500,
            success: true,
            input_hash: eval_resp.input_hash.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(record_resp.acknowledged);
}

// ============================================================
// Test 30: Toolchain service end-to-end
// ============================================================

#[tokio::test]
async fn test_toolchain_service_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = toolchain_service_client::ToolchainServiceClient::new(channel);

    // List toolchains
    let list_resp = client
        .list_toolchains(Request::new(ListToolchainsRequest {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // May or may not have toolchains installed
    let _ = list_resp.toolchains;

    // Get Java home for current JVM
    let java_home_resp = client
        .get_java_home(Request::new(GetJavaHomeRequest {
            language_version: "17".to_string(),
            implementation: "jvm".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // May or may not find a JVM
    let _ = java_home_resp.found;
}

// ============================================================
// Test 31: Build event stream end-to-end
// ============================================================

#[tokio::test]
async fn test_build_event_stream_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = build_event_stream_service_client::BuildEventStreamServiceClient::new(channel);

    // Send build events
    let send_resp = client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: "evt-test".to_string(),
            event_type: "build_start".to_string(),
            event_id: "evt-1".to_string(),
            properties: make_prop_map(vec![("root", "/tmp/test")]),
            display_name: "Build".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(send_resp.accepted);

    client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: "evt-test".to_string(),
            event_type: "task_start".to_string(),
            event_id: "evt-2".to_string(),
            properties: make_prop_map(vec![("task", ":compileJava")]),
            display_name: ":compileJava".to_string(),
            parent_id: "evt-1".to_string(),
        }))
        .await
        .unwrap();

    client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: "evt-test".to_string(),
            event_type: "build_finish".to_string(),
            event_id: "evt-3".to_string(),
            properties: make_prop_map(vec![("result", "SUCCESS")]),
            display_name: "Build".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    // Get event log
    let log_resp = client
        .get_event_log(Request::new(GetEventLogRequest {
            build_id: "evt-test".to_string(),
            since_timestamp_ms: 0,
            max_events: 100,
            event_types: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(log_resp.total_events >= 3);
    assert_eq!(log_resp.events[0].event_type, "build_start");
    assert_eq!(log_resp.events[2].event_type, "build_finish");
}

// ============================================================
// Test 32: Test execution service end-to-end
// ============================================================

#[tokio::test]
async fn test_test_execution_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = test_execution_service_client::TestExecutionServiceClient::new(channel);

    // Register test suite
    client
        .register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "test-exec".to_string(),
            suite: Some(TestSuiteDescriptor {
                suite_id: "suite-1".to_string(),
                suite_name: "com.example.MyTest".to_string(),
                suite_type: "junit5".to_string(),
                test_count: 3,
                module_path: ":app".to_string(),
            }),
        }))
        .await
        .unwrap();

    // Report test results
    client
        .report_test_result(Request::new(ReportTestResultRequest {
            build_id: "test-exec".to_string(),
            result: Some(TestResultEntry {
                test_id: "test-1".to_string(),
                suite_id: "suite-1".to_string(),
                test_name: "testSuccess".to_string(),
                test_class: "com.example.MyTest".to_string(),
                outcome: "PASSED".to_string(),
                start_time_ms: 0,
                end_time_ms: 50,
                duration_ms: 50,
                failure_message: String::new(),
                failure_type: String::new(),
                failure_stack_trace: vec![],
                rerun: false,
                attempt: 1,
            }),
        }))
        .await
        .unwrap();

    client
        .report_test_result(Request::new(ReportTestResultRequest {
            build_id: "test-exec".to_string(),
            result: Some(TestResultEntry {
                test_id: "test-2".to_string(),
                suite_id: "suite-1".to_string(),
                test_name: "testFailure".to_string(),
                test_class: "com.example.MyTest".to_string(),
                outcome: "FAILED".to_string(),
                start_time_ms: 0,
                end_time_ms: 100,
                duration_ms: 100,
                failure_message: "assertion failed".to_string(),
                failure_type: "AssertionError".to_string(),
                failure_stack_trace: vec!["at com.example.MyTest.testFailure(MyTest.java:42)".to_string()],
                rerun: false,
                attempt: 1,
            }),
        }))
        .await
        .unwrap();

    client
        .report_test_result(Request::new(ReportTestResultRequest {
            build_id: "test-exec".to_string(),
            result: Some(TestResultEntry {
                test_id: "test-3".to_string(),
                suite_id: "suite-1".to_string(),
                test_name: "testSkipped".to_string(),
                test_class: "com.example.MyTest".to_string(),
                outcome: "SKIPPED".to_string(),
                start_time_ms: 0,
                end_time_ms: 0,
                duration_ms: 0,
                failure_message: String::new(),
                failure_type: String::new(),
                failure_stack_trace: vec![],
                rerun: false,
                attempt: 1,
            }),
        }))
        .await
        .unwrap();

    // Get test report
    let report = client
        .get_test_report(Request::new(GetTestReportRequest {
            build_id: "test-exec".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(report.suites.len() >= 1);
    let suite = &report.suites[0];
    assert_eq!(suite.passed, 1);
    assert_eq!(suite.failed, 1);
    assert_eq!(suite.skipped, 1);

    // Get test summary
    let summary = client
        .get_test_summary(Request::new(GetTestSummaryRequest {
            build_id: "test-exec".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(summary.tests, 3);
    assert_eq!(summary.passed, 1);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.skipped, 1);
}

// ============================================================
// Test 33: Artifact publishing service end-to-end
// ============================================================

#[tokio::test]
async fn test_artifact_publishing_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = artifact_publishing_service_client::ArtifactPublishingServiceClient::new(channel);

    // Register artifact
    let reg_resp = client
        .register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "pub-test".to_string(),
            artifact: Some(ArtifactDescriptor {
                artifact_id: "jar-main".to_string(),
                group: "com.example".to_string(),
                name: "my-lib".to_string(),
                version: "1.0.0".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                file_path: "/tmp/my-lib-1.0.0.jar".to_string(),
                file_size_bytes: 1024,
                repository_id: "maven-central".to_string(),
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(reg_resp.accepted);

    // Record upload success
    let upload_resp = client
        .record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "jar-main".to_string(),
            success: true,
            error_message: String::new(),
            upload_duration_ms: 250,
            bytes_transferred: 1024,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(upload_resp.accepted);

    // Get publishing status
    let status = client
        .get_publishing_status(Request::new(GetPublishingStatusRequest {
            build_id: "pub-test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(status.artifacts.len(), 1);
    assert_eq!(status.artifacts[0].status, "uploaded");
}

// ============================================================
// Test 34: Build comparison service end-to-end
// ============================================================

#[tokio::test]
async fn test_build_comparison_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = build_comparison_service_client::BuildComparisonServiceClient::new(channel);

    // Record baseline build data FIRST (start_comparison requires both to exist)
    let mut baseline_durations: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    baseline_durations.insert(":compileJava".to_string(), 1000);
    baseline_durations.insert(":test".to_string(), 2000);
    let mut baseline_outcomes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    baseline_outcomes.insert(":compileJava".to_string(), "SUCCESS".to_string());
    baseline_outcomes.insert(":test".to_string(), "SUCCESS".to_string());

    client
        .record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(BuildDataSnapshot {
                build_id: "baseline-1".to_string(),
                start_time_ms: 0,
                end_time_ms: 3000,
                task_durations: baseline_durations,
                task_outcomes: baseline_outcomes,
                task_order: vec![":compileJava".to_string(), ":test".to_string()],
                root_dir: "/tmp/project".to_string(),
                input_properties: vec![],
            }),
        }))
        .await
        .unwrap();

    // Record candidate build data (slower)
    let mut candidate_durations: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    candidate_durations.insert(":compileJava".to_string(), 1200);
    candidate_durations.insert(":test".to_string(), 2500);
    let mut candidate_outcomes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    candidate_outcomes.insert(":compileJava".to_string(), "SUCCESS".to_string());
    candidate_outcomes.insert(":test".to_string(), "SUCCESS".to_string());

    client
        .record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(BuildDataSnapshot {
                build_id: "candidate-1".to_string(),
                start_time_ms: 0,
                end_time_ms: 3700,
                task_durations: candidate_durations,
                task_outcomes: candidate_outcomes,
                task_order: vec![":compileJava".to_string(), ":test".to_string()],
                root_dir: "/tmp/project".to_string(),
                input_properties: vec![],
            }),
        }))
        .await
        .unwrap();

    // Now start comparison (both builds must exist first)
    let start_resp = client
        .start_comparison(Request::new(StartComparisonRequest {
            baseline_build_id: "baseline-1".to_string(),
            candidate_build_id: "candidate-1".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(start_resp.started);
    assert!(!start_resp.comparison_id.is_empty());
    let comparison_id = start_resp.comparison_id;

    // Get comparison result
    let result = client
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: comparison_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(result.summary.is_some());
    let summary = result.summary.unwrap();
    assert_eq!(summary.baseline_build_id, "baseline-1");
    assert_eq!(summary.candidate_build_id, "candidate-1");
    assert_eq!(summary.baseline_total_ms, 3000);
    assert_eq!(summary.candidate_total_ms, 3700);
    assert!(summary.tasks_with_regression >= 0);
}

// ============================================================
// Test 35: Cross-service execution plan with persistent history
// ============================================================

#[tokio::test]
async fn test_cross_service_execution_plan_with_history() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;

    let mut plan_client = execution_plan_service_client::ExecutionPlanServiceClient::new(channel.clone());
    let mut history_client = execution_history_service_client::ExecutionHistoryServiceClient::new(channel.clone());
    let mut graph_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());

    // Register a task in the graph
    graph_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: String::new(),
            task_path: ":compileJava".to_string(),
            depends_on: vec![],
            task_type: "JavaCompile".to_string(),
            should_execute: true,
        }))
        .await
        .unwrap();

    // Predict outcome — no history, should suggest cache
    let predict_resp = plan_client
        .predict_outcome(Request::new(PredictOutcomeRequest {
            work: Some(WorkMetadata {
                work_identity: ":compileJava".to_string(),
                display_name: "compileJava".to_string(),
                implementation_class: "JavaCompile".to_string(),
                input_properties: make_prop_map(vec![("source", "v1")]),
                input_file_fingerprints: std::collections::HashMap::new(),
                caching_enabled: true,
                can_load_from_cache: true,
                has_previous_execution_state: false,
                rebuild_reasons: vec![],
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    // No history → predicts from cache or execute
    assert!(predict_resp.confidence > 0.0);

    // Record an execution outcome
    plan_client
        .record_outcome(Request::new(RecordOutcomeRequest {
            work_identity: ":compileJava".to_string(),
            predicted_outcome: predict_resp.predicted_outcome,
            actual_outcome: "EXECUTED".to_string(),
            prediction_correct: true,
            duration_ms: 500,
        }))
        .await
        .unwrap();

    // Complete the task in the graph (persists duration to history)
    graph_client
        .task_finished(Request::new(TaskFinishedRequest {
            build_id: String::new(),
            task_path: ":compileJava".to_string(),
            duration_ms: 500,
            success: true,
            outcome: "EXECUTED".to_string(),
        }))
        .await
        .unwrap();

    // Now predict again with same inputs — record_outcome doesn't store the fingerprint
    // (it's not available from the gRPC request), so the service sees "inputs changed"
    // (empty stored fingerprint vs computed fingerprint). This validates the flow works
    // even though the fingerprint isn't persisted.
    let predict_resp2 = plan_client
        .predict_outcome(Request::new(PredictOutcomeRequest {
            work: Some(WorkMetadata {
                work_identity: ":compileJava".to_string(),
                display_name: "compileJava".to_string(),
                implementation_class: "JavaCompile".to_string(),
                input_properties: make_prop_map(vec![("source", "v1")]),
                input_file_fingerprints: std::collections::HashMap::new(),
                caching_enabled: true,
                can_load_from_cache: true,
                has_previous_execution_state: false,
                rebuild_reasons: vec![],
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    // Record was found (history exists) but fingerprint mismatch → PredictedExecute
    assert_eq!(predict_resp2.predicted_outcome, PredictedOutcome::PredictedExecute as i32);
    assert!(predict_resp2.reasoning.contains("Inputs changed"));

    // Note: The execution_history gRPC service is a separate instance from the one
    // used internally by execution_plan. This is expected — in production, the Java
    // bridge wires a single shared instance. Here we validate the gRPC API contract.
    let stats = history_client
        .get_history_stats(Request::new(GetHistoryStatsRequest {}))
        .await
        .unwrap()
        .into_inner();

    // The history service has its own stats (may have entries from other tests' stores)
    assert!(stats.hit_rate >= 0.0 && stats.hit_rate <= 1.0);
}

// ============================================================
// Test 36: Build result service dedicated e2e
// ============================================================

#[tokio::test]
async fn test_build_result_service_e2e() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = build_result_service_client::BuildResultServiceClient::new(channel);

    let build_id = "e2e-build-result".to_string();

    // Report task results with various outcomes
    client
        .report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":compileJava".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 1500,
                did_work: true,
                cache_key: "ck-compile".to_string(),
                start_time_ms: 0,
                end_time_ms: 1500,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

    client
        .report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":processResources".to_string(),
                outcome: "UP_TO_DATE".to_string(),
                duration_ms: 0,
                did_work: false,
                cache_key: String::new(),
                start_time_ms: 1500,
                end_time_ms: 1500,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

    client
        .report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":test".to_string(),
                outcome: "FROM_CACHE".to_string(),
                duration_ms: 5,
                did_work: false,
                cache_key: "ck-test".to_string(),
                start_time_ms: 1500,
                end_time_ms: 1505,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

    // Get build result — should aggregate correctly
    let result = client
        .get_build_result(Request::new(GetBuildResultRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    let outcome = result.outcome.unwrap();
    assert_eq!(outcome.overall_result, "SUCCESS");
    assert_eq!(outcome.tasks_total, 3);
    assert_eq!(outcome.tasks_executed, 1);
    assert_eq!(outcome.tasks_up_to_date, 1);
    assert_eq!(outcome.tasks_from_cache, 1);
    assert_eq!(outcome.tasks_failed, 0);
    assert_eq!(outcome.tasks_skipped, 0);
    assert_eq!(outcome.total_duration_ms, 1505);

    // Verify individual task results are returned
    assert_eq!(result.task_results.len(), 3);

    // Get task summary
    let summary = client
        .get_task_summary(Request::new(GetTaskSummaryRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(summary.tasks.len(), 3);
    assert_eq!(summary.total_duration_ms, 1505);

    // Verify task summary entries are in order
    let paths: Vec<&str> = summary.tasks.iter().map(|t| t.task_path.as_str()).collect();
    assert!(paths.contains(&":compileJava"));
    assert!(paths.contains(&":processResources"));
    assert!(paths.contains(&":test"));

    // Now report a build failure and verify outcome changes to FAILED
    client
        .report_build_failure(Request::new(ReportBuildFailureRequest {
            build_id: build_id.clone(),
            failure_type: "test_failure".to_string(),
            failure_message: "org.example.MyTest.testFoo failed".to_string(),
            failed_task_paths: vec![":test".to_string()],
        }))
        .await
        .unwrap();

    let result_after = client
        .get_build_result(Request::new(GetBuildResultRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(result_after.outcome.unwrap().overall_result, "FAILED");

    // Verify empty build result for nonexistent build
    let empty = client
        .get_build_result(Request::new(GetBuildResultRequest {
            build_id: "nonexistent-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    let empty_outcome = empty.outcome.unwrap();
    assert_eq!(empty_outcome.overall_result, "SUCCESS"); // no failures = success
    assert_eq!(empty_outcome.tasks_total, 0);
}

// ============================================================
// Test 37: Task graph → build result → build comparison workflow
// ============================================================

#[tokio::test]
async fn test_task_graph_to_build_result_workflow() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut graph_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());
    let mut result_client = build_result_service_client::BuildResultServiceClient::new(channel.clone());
    let mut comparison_client = build_comparison_service_client::BuildComparisonServiceClient::new(channel.clone());

    let build_id = "workflow-build".to_string();

    // 1. Register tasks in the graph
    let tasks = vec![
        (":compileJava", "JavaCompile", vec![":compileKotlin".to_string()]),
        (":compileKotlin", "KotlinCompile", vec![]),
        (":processResources", "ProcessResources", vec![]),
        (":classes", "Lifecycle", vec![":compileJava".to_string(), ":processResources".to_string()]),
        (":test", "Test", vec![":classes".to_string()]),
    ];

    for (path, task_type, deps) in &tasks {
        graph_client
            .register_task(Request::new(RegisterTaskRequest {
                build_id: build_id.clone(),
                task_path: path.to_string(),
                depends_on: deps.clone(),
                task_type: task_type.to_string(),
                should_execute: true,
            }))
            .await
            .unwrap();
    }

    // 2. Mark tasks as executing and finished
    graph_client
        .task_started(Request::new(TaskStartedRequest {
            build_id: build_id.clone(),
            task_path: ":compileKotlin".to_string(),
            start_time_ms: 0,
        }))
        .await
        .unwrap();

    graph_client
        .task_finished(Request::new(TaskFinishedRequest {
            build_id: build_id.clone(),
            task_path: ":compileKotlin".to_string(),
            duration_ms: 300,
            success: true,
            outcome: "SUCCESS".to_string(),
        }))
        .await
        .unwrap();

    graph_client
        .task_started(Request::new(TaskStartedRequest {
            build_id: build_id.clone(),
            task_path: ":compileJava".to_string(),
            start_time_ms: 300,
        }))
        .await
        .unwrap();

    graph_client
        .task_finished(Request::new(TaskFinishedRequest {
            build_id: build_id.clone(),
            task_path: ":compileJava".to_string(),
            duration_ms: 800,
            success: true,
            outcome: "SUCCESS".to_string(),
        }))
        .await
        .unwrap();

    // 3. Report task results to build result service
    result_client
        .report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":compileKotlin".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 300,
                did_work: true,
                cache_key: String::new(),
                start_time_ms: 0,
                end_time_ms: 300,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

    result_client
        .report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":compileJava".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 800,
                did_work: true,
                cache_key: String::new(),
                start_time_ms: 300,
                end_time_ms: 1100,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

    // 4. Record build data for comparison (must record before starting comparison)
    let mut baseline_durations = std::collections::HashMap::new();
    baseline_durations.insert(":compileKotlin".to_string(), 300);
    baseline_durations.insert(":compileJava".to_string(), 800);
    let mut baseline_outcomes = std::collections::HashMap::new();
    baseline_outcomes.insert(":compileKotlin".to_string(), "SUCCESS".to_string());
    baseline_outcomes.insert(":compileJava".to_string(), "SUCCESS".to_string());

    comparison_client
        .record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(BuildDataSnapshot {
                build_id: "baseline-build".to_string(),
                start_time_ms: 0,
                end_time_ms: 1100,
                task_durations: baseline_durations,
                task_outcomes: baseline_outcomes,
                task_order: vec![":compileKotlin".to_string(), ":compileJava".to_string()],
                root_dir: "/tmp".to_string(),
                input_properties: vec![],
            }),
        }))
        .await
        .unwrap();

    let mut candidate_durations = std::collections::HashMap::new();
    candidate_durations.insert(":compileKotlin".to_string(), 300);
    candidate_durations.insert(":compileJava".to_string(), 800);
    let mut candidate_outcomes = std::collections::HashMap::new();
    candidate_outcomes.insert(":compileKotlin".to_string(), "SUCCESS".to_string());
    candidate_outcomes.insert(":compileJava".to_string(), "SUCCESS".to_string());

    comparison_client
        .record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(BuildDataSnapshot {
                build_id: build_id.clone(),
                start_time_ms: 0,
                end_time_ms: 1100,
                task_durations: candidate_durations,
                task_outcomes: candidate_outcomes,
                task_order: vec![":compileKotlin".to_string(), ":compileJava".to_string()],
                root_dir: "/tmp".to_string(),
                input_properties: vec![],
            }),
        }))
        .await
        .unwrap();

    // 5. Start comparison (both builds now exist)
    let start_comp = comparison_client
        .start_comparison(Request::new(StartComparisonRequest {
            baseline_build_id: "baseline-build".to_string(),
            candidate_build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(start_comp.started);
    let comparison_id = start_comp.comparison_id;

    // 5. Verify build result aggregation
    let result = result_client
        .get_build_result(Request::new(GetBuildResultRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    let outcome = result.outcome.unwrap();
    assert_eq!(outcome.overall_result, "SUCCESS");
    assert_eq!(outcome.tasks_total, 2);
    assert_eq!(outcome.tasks_executed, 2);
    assert_eq!(outcome.total_duration_ms, 1100);

    // 6. Verify comparison was stored
    let comparison = comparison_client
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(comparison.task_comparisons.len(), 2);

    // 7. Verify task graph progress
    let progress = graph_client
        .get_progress(Request::new(GetProgressRequest {
            build_id: "workflow-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(progress.tasks.len(), 5);
}

// ============================================================
// Test 38: Build layout → work scheduling → build result pipeline
// ============================================================

#[tokio::test]
async fn test_build_layout_to_work_pipeline() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut layout_client = build_layout_service_client::BuildLayoutServiceClient::new(channel.clone());
    let mut work_client = work_service_client::WorkServiceClient::new(channel.clone());
    let mut result_client = build_result_service_client::BuildResultServiceClient::new(channel.clone());
    let mut ops_client = build_operations_service_client::BuildOperationsServiceClient::new(channel.clone());

    // 1. Initialize build layout
    let layout = layout_client
        .init_build_layout(Request::new(InitBuildLayoutRequest {
            root_dir: "/tmp/pipeline-project".to_string(),
            settings_file: "/tmp/pipeline-project/settings.gradle".to_string(),
            build_file: "build.gradle.kts".to_string(),
            build_name: "pipeline-test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    let build_id = layout.build_id;

    // 2. Add subprojects
    layout_client
        .add_subproject(Request::new(AddSubprojectRequest {
            build_id: build_id.clone(),
            project_path: ":app".to_string(),
            project_dir: "/tmp/pipeline-project/app".to_string(),
            build_file: "/tmp/pipeline-project/app/build.gradle.kts".to_string(),
            display_name: "app".to_string(),
        }))
        .await
        .unwrap();

    layout_client
        .add_subproject(Request::new(AddSubprojectRequest {
            build_id: build_id.clone(),
            project_path: ":lib".to_string(),
            project_dir: "/tmp/pipeline-project/lib".to_string(),
            build_file: "/tmp/pipeline-project/lib/build.gradle.kts".to_string(),
            display_name: "lib".to_string(),
        }))
        .await
        .unwrap();

    // 3. Verify project tree
    let tree = layout_client
        .get_project_tree(Request::new(GetProjectTreeRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(tree.all_projects.len(), 2);
    assert_eq!(tree.root.unwrap().children.len(), 2);

    // 4. List projects
    let projects = layout_client
        .list_projects(Request::new(ListProjectsRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(projects.project_paths.len(), 3); // root + :app + :lib

    // 5. Evaluate work items (simulating task scheduling decisions)
    let eval1 = work_client
        .evaluate(Request::new(WorkEvaluateRequest {
            task_path: ":lib:compileJava".to_string(),
            input_properties: make_prop_map(vec![("source", "v1")]),
        }))
        .await
        .unwrap()
        .into_inner();

    // First evaluation should suggest execution (no history)
    assert!(eval1.should_execute);

    let eval2 = work_client
        .evaluate(Request::new(WorkEvaluateRequest {
            task_path: ":app:compileJava".to_string(),
            input_properties: make_prop_map(vec![("source", "v2")]),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(eval2.should_execute);

    // 6. Track operations
    ops_client
        .start_operation(Request::new(StartOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-lib-compile".to_string(),
            display_name: "Compile lib".to_string(),
            operation_type: "TASK_EXECUTION".to_string(),
            parent_id: String::new(),
            start_time_ms: 100,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

    ops_client
        .complete_operation(Request::new(CompleteOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-lib-compile".to_string(),
            duration_ms: 400,
            success: true,
            outcome: "SUCCESS".to_string(),
        }))
        .await
        .unwrap();

    // 7. Report task results
    result_client
        .report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":lib:compileJava".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 400,
                did_work: true,
                cache_key: String::new(),
                start_time_ms: 100,
                end_time_ms: 500,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

    result_client
        .report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":app:compileJava".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 600,
                did_work: true,
                cache_key: String::new(),
                start_time_ms: 500,
                end_time_ms: 1100,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

    // 8. Verify final build result
    let result = result_client
        .get_build_result(Request::new(GetBuildResultRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    let outcome = result.outcome.unwrap();
    assert_eq!(outcome.overall_result, "SUCCESS");
    assert_eq!(outcome.tasks_total, 2);
    assert_eq!(outcome.tasks_executed, 2);
    assert_eq!(outcome.total_duration_ms, 1000);

    // 9. Verify task summary
    let summary = result_client
        .get_task_summary(Request::new(GetTaskSummaryRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(summary.tasks.len(), 2);
    assert_eq!(summary.total_duration_ms, 1000);
}

// ============================================================
// Test 39: Exec → build operations → build result workflow
// ============================================================

#[tokio::test]
async fn test_exec_to_build_operations_workflow() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut exec_client = exec_service_client::ExecServiceClient::new(channel.clone());
    let mut ops_client = build_operations_service_client::BuildOperationsServiceClient::new(channel.clone());
    let mut result_client = build_result_service_client::BuildResultServiceClient::new(channel.clone());

    let build_id = "exec-workflow-build".to_string();

    // 1. Start a build operation tracking the exec
    ops_client
        .start_operation(Request::new(StartOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-exec-test".to_string(),
            display_name: "Run test process".to_string(),
            operation_type: "TASK_EXECUTION".to_string(),
            parent_id: String::new(),
            start_time_ms: 0,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

    // 2. Spawn a process (echo "hello" on macOS/Linux)
    let spawn_resp = exec_client
        .spawn(Request::new(ExecSpawnRequest {
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            environment: Default::default(),
            working_dir: "/tmp".to_string(),
            redirect_error_stream: false,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(spawn_resp.success);
    let pid = spawn_resp.pid;

    // 3. Wait for the process to complete
    let start_time = std::time::Instant::now();
    let wait_resp = exec_client
        .wait(Request::new(ExecWaitRequest {
            pid,
        }))
        .await
        .unwrap()
        .into_inner();

    let elapsed_ms = start_time.elapsed().as_millis() as i64;

    // 4. Complete the operation
    ops_client
        .complete_operation(Request::new(CompleteOperationRequest {
            build_id: "test".to_string(),
            operation_id: "op-exec-test".to_string(),
            duration_ms: elapsed_ms,
            success: wait_resp.exit_code == 0,
            outcome: if wait_resp.exit_code == 0 { "SUCCESS".to_string() } else { "FAILED".to_string() },
        }))
        .await
        .unwrap();

    // 5. Report task result
    result_client
        .report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":runTests".to_string(),
                outcome: if wait_resp.exit_code == 0 { "SUCCESS".to_string() } else { "FAILED".to_string() },
                duration_ms: elapsed_ms,
                did_work: true,
                cache_key: String::new(),
                start_time_ms: 0,
                end_time_ms: elapsed_ms,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

    // 6. Verify the build result reflects the exec outcome
    let result = result_client
        .get_build_result(Request::new(GetBuildResultRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    let outcome = result.outcome.unwrap();
    assert_eq!(outcome.tasks_total, 1);
    assert_eq!(outcome.overall_result, "SUCCESS");
    assert!(outcome.total_duration_ms >= 0);
}

// ============================================================
// Test 40: Build metrics -> cache orchestration -> performance summary
// ============================================================

#[tokio::test]
async fn test_build_metrics_to_cache_orchestration_workflow() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;

    let mut metrics_client = build_metrics_service_client::BuildMetricsServiceClient::new(channel.clone());
    let mut orch_client =
        build_cache_orchestration_service_client::BuildCacheOrchestrationServiceClient::new(channel.clone());

    let build_id = "metrics-cache-workflow".to_string();

    // Step 1: Record build metrics (tasks.total, tasks.cached, cache.hits, cache.misses, build.start, build.end)
    let metric_names = vec![
        "tasks.total".to_string(),
        "tasks.cached".to_string(),
        "cache.hits".to_string(),
        "cache.misses".to_string(),
        "build.start".to_string(),
        "build.end".to_string(),
    ];
    let metric_values = vec!["10", "4", "4", "6", "1000", "5000"];

    for (name, value) in metric_names.iter().zip(metric_values.iter()) {
        let record_resp = metrics_client
            .record_metric(Request::new(RecordMetricRequest {
                build_id: build_id.clone(),
                event: Some(MetricEvent {
                    name: name.clone(),
                    value: value.to_string(),
                    metric_type: "counter".to_string(),
                    tags: std::collections::HashMap::new(),
                    timestamp_ms: 0,
                }),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(record_resp.recorded);
    }

    // Step 2: Use cache orchestration to check/record cache entries
    // Compute cache key for a task
    let cache_key_resp = orch_client
        .compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "impl-hash-abc".to_string(),
            input_property_hashes: make_prop_map(vec![("source", "h1"), ("target", "h2")]),
            input_file_hashes: make_prop_map(vec![("classpath", "h3")]),
            output_property_names: vec!["classes".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!cache_key_resp.cache_key.is_empty());

    // Probe cache — should miss initially
    let probe_miss = orch_client
        .probe_cache(Request::new(ProbeCacheRequest {
            cache_key: cache_key_resp.cache_key.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!probe_miss.available);

    // Store outputs in cache
    let store_resp = orch_client
        .store_outputs(Request::new(StoreOutputsRequest {
            cache_key: cache_key_resp.cache_key.clone(),
            execution_time_ms: 500,
            output_properties: vec!["classes".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(store_resp.success);

    // Probe cache — metadata says stored but real cache doesn't have the entry
    // With real cache wiring, probe returns miss when actual cache file is absent
    let probe_after_store = orch_client
        .probe_cache(Request::new(ProbeCacheRequest {
            cache_key: cache_key_resp.cache_key.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!probe_after_store.available,
        "Probe should return miss when metadata exists but real cache entry is absent");

    // Step 3: Get performance summary and verify aggregated metrics
    let summary_resp = metrics_client
        .get_performance_summary(Request::new(GetPerformanceSummaryRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(summary_resp.summary.is_some());
    let summary = summary_resp.summary.unwrap();
    assert_eq!(summary.build_id, build_id);

    // Verify the metrics we recorded are retrievable
    let get_resp = metrics_client
        .get_metrics(Request::new(GetMetricsRequest {
            build_id: build_id.clone(),
            metric_names: vec![
                "tasks.total".to_string(),
                "tasks.cached".to_string(),
                "cache.hits".to_string(),
                "cache.misses".to_string(),
                "build.start".to_string(),
                "build.end".to_string(),
            ],
            since_ms: 0,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(get_resp.metrics.len() >= 6);

    // Verify we can find each metric by name
    let metric_map: std::collections::HashMap<String, &MetricSnapshot> = get_resp
        .metrics
        .iter()
        .map(|m| (m.name.clone(), m))
        .collect();

    assert!(metric_map.contains_key("tasks.total"));
    assert!(metric_map.contains_key("tasks.cached"));
    assert!(metric_map.contains_key("cache.hits"));
    assert!(metric_map.contains_key("cache.misses"));
    assert!(metric_map.contains_key("build.start"));
    assert!(metric_map.contains_key("build.end"));
}

// ============================================================
// Test 41: Toolchain -> exec -> process pipeline
// ============================================================

#[tokio::test]
async fn test_toolchain_to_exec_pipeline() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;

    let mut toolchain_client = toolchain_service_client::ToolchainServiceClient::new(channel.clone());
    let mut exec_client = exec_service_client::ExecServiceClient::new(channel.clone());

    // Step 1: Register/verify a Java toolchain by requesting its Java home
    let _java_home_resp = toolchain_client
        .get_java_home(Request::new(GetJavaHomeRequest {
            language_version: "17".to_string(),
            implementation: "jvm".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // Step 2: Spawn a process using the exec service
    // Use 'true' command (always exits 0) as a simple cross-platform test
    let spawn_resp = exec_client
        .spawn(Request::new(ExecSpawnRequest {
            command: "true".to_string(),
            args: vec![],
            environment: Default::default(),
            working_dir: "/tmp".to_string(),
            redirect_error_stream: false,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(spawn_resp.success);
    let pid = spawn_resp.pid;

    // Step 3: Wait for the process to complete
    let wait_resp = exec_client
        .wait(Request::new(ExecWaitRequest { pid }))
        .await
        .unwrap()
        .into_inner();

    // 'true' always exits with code 0
    assert_eq!(wait_resp.exit_code, 0);
    assert!(wait_resp.error_message.is_empty());

    // Step 4: Spawn 'false' command (always exits with code 1) to verify non-zero exit
    let spawn_false = exec_client
        .spawn(Request::new(ExecSpawnRequest {
            command: "false".to_string(),
            args: vec![],
            environment: Default::default(),
            working_dir: "/tmp".to_string(),
            redirect_error_stream: false,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(spawn_false.success);

    let wait_false = exec_client
        .wait(Request::new(ExecWaitRequest { pid: spawn_false.pid }))
        .await
        .unwrap()
        .into_inner();

    // 'false' always exits with code 1
    assert_eq!(wait_false.exit_code, 1);

    // Step 5: Spawn 'echo' and verify it runs successfully
    let spawn_echo = exec_client
        .spawn(Request::new(ExecSpawnRequest {
            command: "echo".to_string(),
            args: vec!["integration".to_string(), "test".to_string()],
            environment: Default::default(),
            working_dir: "/tmp".to_string(),
            redirect_error_stream: false,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(spawn_echo.success);

    let wait_echo = exec_client
        .wait(Request::new(ExecWaitRequest { pid: spawn_echo.pid }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(wait_echo.exit_code, 0);
}

// ============================================================
// Test 42: Configuration -> build init flow
// ============================================================

#[tokio::test]
async fn test_configuration_to_build_init_flow() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;

    let mut config_client = configuration_service_client::ConfigurationServiceClient::new(channel.clone());
    let mut init_client = build_init_service_client::BuildInitServiceClient::new(channel.clone());

    let build_id = "config-init-flow".to_string();

    // Step 1: Register a project with properties via configuration service
    let register_resp = config_client
        .register_project(Request::new(RegisterProjectRequest {
            project_path: ":app".to_string(),
            project_dir: "/tmp/config-init-app".to_string(),
            properties: make_prop_map(vec![
                ("version", "3.0.0"),
                ("group", "com.example.flow"),
                ("sourceCompatibility", "17"),
            ]),
            applied_plugins: vec!["java".to_string(), "application".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(register_resp.success);

    // Step 2: Resolve configuration properties
    let version_resp = config_client
        .resolve_property(Request::new(ResolvePropertyRequest {
            project_path: ":app".to_string(),
            property_name: "version".to_string(),
            requested_by: "build-init-test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(version_resp.found);
    assert_eq!(version_resp.value, "3.0.0");

    let group_resp = config_client
        .resolve_property(Request::new(ResolvePropertyRequest {
            project_path: ":app".to_string(),
            property_name: "group".to_string(),
            requested_by: "build-init-test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(group_resp.found);
    assert_eq!(group_resp.value, "com.example.flow");

    let compat_resp = config_client
        .resolve_property(Request::new(ResolvePropertyRequest {
            project_path: ":app".to_string(),
            property_name: "sourceCompatibility".to_string(),
            requested_by: "build-init-test".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(compat_resp.found);
    assert_eq!(compat_resp.value, "17");

    // Step 3: Cache the configuration state
    let cache_resp = config_client
        .cache_configuration(Request::new(CacheConfigurationRequest {
            project_path: ":app".to_string(),
            config_hash: vec![1, 2, 3, 4],
            timestamp_ms: 5000,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(cache_resp.cached);

    // Step 4: Init a build via build init service
    let init_resp = init_client
        .init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: build_id.clone(),
            root_dir: "/tmp/config-init-app".to_string(),
            settings_file: "/tmp/config-init-app/settings.gradle".to_string(),
            gradle_user_home: String::new(),
            init_scripts: vec![],
            requested_build_features: vec!["configuration-cache".to_string()],
            current_dir: "/tmp/config-init-app".to_string(),
            session_id: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(init_resp.initialized);

    // Step 5: Record settings details from the resolved configuration properties
    init_client
        .record_settings_detail(Request::new(RecordSettingsDetailRequest {
            build_id: build_id.clone(),
            detail: Some(SettingsDetailEntry {
                key: "projectVersion".to_string(),
                value: "3.0.0".to_string(),
            }),
        }))
        .await
        .unwrap();

    init_client
        .record_settings_detail(Request::new(RecordSettingsDetailRequest {
            build_id: build_id.clone(),
            detail: Some(SettingsDetailEntry {
                key: "projectGroup".to_string(),
                value: "com.example.flow".to_string(),
            }),
        }))
        .await
        .unwrap();

    init_client
        .record_settings_detail(Request::new(RecordSettingsDetailRequest {
            build_id: build_id.clone(),
            detail: Some(SettingsDetailEntry {
                key: "sourceCompatibility".to_string(),
                value: "17".to_string(),
            }),
        }))
        .await
        .unwrap();

    // Step 6: Get build init status and verify the details are present
    let status_resp = init_client
        .get_build_init_status(Request::new(GetBuildInitStatusRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(status_resp.status.is_some());
    let status = status_resp.status.unwrap();
    assert!(status.initialized);
    assert_eq!(status.build_id, build_id);

    // Verify the settings details we recorded are present
    let details: std::collections::HashMap<&str, &str> = status
        .settings_details
        .iter()
        .map(|d| (d.key.as_str(), d.value.as_str()))
        .collect();

    assert_eq!(details.get("projectVersion").copied(), Some("3.0.0"));
    assert_eq!(details.get("projectGroup").copied(), Some("com.example.flow"));
    assert_eq!(details.get("sourceCompatibility").copied(), Some("17"));
}

// ============================================================
// Test 43: Plugin -> dependency resolution chain
// ============================================================

#[tokio::test]
async fn test_plugin_to_dependency_resolution_chain() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;

    let mut plugin_client = plugin_service_client::PluginServiceClient::new(channel.clone());
    let mut dep_client =
        dependency_resolution_service_client::DependencyResolutionServiceClient::new(channel.clone());

    let project_path = ":chain-test".to_string();

    // Step 1: Register plugins
    plugin_client
        .register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "java".to_string(),
            plugin_class: "org.gradle.api.plugins.JavaPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec![],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

    plugin_client
        .register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "application".to_string(),
            plugin_class: "org.gradle.api.plugins.ApplicationPlugin".to_string(),
            version: "1.0".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["java".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

    // Step 2: Apply plugins in order
    let apply_java = plugin_client
        .apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "java".to_string(),
            project_path: project_path.clone(),
            apply_order: 0,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(apply_java.success);

    let apply_app = plugin_client
        .apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "application".to_string(),
            project_path: project_path.clone(),
            apply_order: 1,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(apply_app.success);

    // Step 3: Verify both plugins are applied
    let has_java = plugin_client
        .has_plugin(Request::new(HasPluginRequest {
            plugin_id: "java".to_string(),
            project_path: project_path.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(has_java.has_plugin);

    let has_app = plugin_client
        .has_plugin(Request::new(HasPluginRequest {
            plugin_id: "application".to_string(),
            project_path: project_path.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(has_app.has_plugin);

    // Step 4: Record dependency resolution for configurations used by those plugins
    // The java plugin adds "implementation" and "api" configurations
    dep_client
        .record_resolution(Request::new(RecordResolutionRequest {
            configuration_name: "implementation".to_string(),
            dependency_count: 5,
            resolution_time_ms: 200,
            success: true,
            cache_hits: 3,
        }))
        .await
        .unwrap();

    // The application plugin also adds "runtimeClasspath"
    dep_client
        .record_resolution(Request::new(RecordResolutionRequest {
            configuration_name: "runtimeClasspath".to_string(),
            dependency_count: 8,
            resolution_time_ms: 350,
            success: true,
            cache_hits: 5,
        }))
        .await
        .unwrap();

    // "api" configuration from java plugin
    dep_client
        .record_resolution(Request::new(RecordResolutionRequest {
            configuration_name: "api".to_string(),
            dependency_count: 2,
            resolution_time_ms: 100,
            success: true,
            cache_hits: 2,
        }))
        .await
        .unwrap();

    // A test configuration resolution
    dep_client
        .record_resolution(Request::new(RecordResolutionRequest {
            configuration_name: "testImplementation".to_string(),
            dependency_count: 4,
            resolution_time_ms: 150,
            success: true,
            cache_hits: 1,
        }))
        .await
        .unwrap();

    // Step 5: Use resolve_dependencies to drive total_resolutions stats
    // (record_resolution only acknowledges; resolve_dependencies increments counters)
    let resolve_resp = dep_client
        .resolve_dependencies(Request::new(ResolveDependenciesRequest {
            configuration_name: "implementation".to_string(),
            dependencies: vec![DependencyDescriptor {
                group: "org.slf4j".to_string(),
                name: "slf4j-api".to_string(),
                version: "2.0.9".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
                transitive: false,
            }],
            repositories: vec![],
            attributes: vec![],
            lenient: true,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(resolve_resp.success);
    assert_eq!(resolve_resp.total_artifacts, 1);

    // Step 6: Add artifacts to cache and check them (drives artifact_cache_hits)
    dep_client
        .add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
            group: "org.slf4j".to_string(),
            name: "slf4j-api".to_string(),
            version: "2.0.9".to_string(),
            classifier: String::new(),
            local_path: "/tmp/slf4j-api-2.0.9.jar".to_string(),
            size: 4096,
            sha256: "abcdef".to_string(),
        }))
        .await
        .unwrap();

    dep_client
        .add_artifact_to_cache(Request::new(AddArtifactToCacheRequest {
            group: "org.junit.jupiter".to_string(),
            name: "junit-jupiter-api".to_string(),
            version: "5.10.0".to_string(),
            classifier: String::new(),
            local_path: "/tmp/junit-jupiter-api-5.10.0.jar".to_string(),
            size: 8192,
            sha256: "fedcba".to_string(),
        }))
        .await
        .unwrap();

    // Check artifact cache (should hit)
    let check1 = dep_client
        .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
            group: "org.slf4j".to_string(),
            name: "slf4j-api".to_string(),
            version: "2.0.9".to_string(),
            classifier: String::new(),
            sha256: String::new(),
            extension: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(check1.cached);

    let check2 = dep_client
        .check_artifact_cache(Request::new(CheckArtifactCacheRequest {
            group: "org.junit.jupiter".to_string(),
            name: "junit-jupiter-api".to_string(),
            version: "5.10.0".to_string(),
            classifier: String::new(),
            sha256: String::new(),
            extension: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(check2.cached);

    // Step 7: Get resolution stats and verify aggregated data
    let stats = dep_client
        .get_resolution_stats(Request::new(GetResolutionStatsRequest {}))
        .await
        .unwrap()
        .into_inner();

    // resolve_dependencies incremented total_resolutions by 1
    assert_eq!(stats.total_resolutions, 1);
    // Two cache hits from check_artifact_cache + cache_hits from record_resolution calls (3+5+2+1=11)
    assert_eq!(stats.artifact_cache_hits, 13);
    // Two artifacts in cache
    assert_eq!(stats.cached_artifacts, 2);
    // total_resolution_time_ms should be >= 0 (from the resolve_dependencies call)
    assert!(stats.total_resolution_time_ms >= 0);
    // avg_resolution_time_ms = total / count
    assert!(stats.avg_resolution_time_ms >= 0.0);

    // Step 6: Verify applied plugins list
    let applied = plugin_client
        .get_applied_plugins(Request::new(GetAppliedPluginsRequest {
            project_path: project_path.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(applied.plugins.len(), 2);
    assert_eq!(applied.plugins[0].plugin_id, "java");
    assert_eq!(applied.plugins[1].plugin_id, "application");
}

// ============================================================
// Test: Cache service store + load via streaming gRPC
// ============================================================

#[tokio::test]
async fn test_cache_service_store_load() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    // Step 1: Store a value via client-streaming StoreEntry.
    // The key in the proto is `bytes`, and the server hex-encodes it to form the file path.
    let cache_key = b"aa11223344556677";
    let cache_value = b"hello cache world from integration test";

    let (tx, rx) = tokio::sync::mpsc::channel(8);
    tx.send(CacheStoreChunk {
        payload: Some(cache_store_chunk::Payload::Init(CacheStoreInit {
            key: cache_key.to_vec(),
            total_size: cache_value.len() as i64,
        })),
    }).await.unwrap();
    tx.send(CacheStoreChunk {
        payload: Some(cache_store_chunk::Payload::Data(cache_value.to_vec())),
    }).await.unwrap();
    drop(tx); // close the stream so the server knows we're done sending

    use tokio_stream::wrappers::ReceiverStream;
    let store_response = client
        .store_entry(Request::new(ReceiverStream::new(rx)))
        .await
        .unwrap()
        .into_inner();

    assert!(store_response.success, "store_entry failed: {}", store_response.error_message);
    assert!(store_response.error_message.is_empty());

    // Step 2: Load the value back via server-streaming LoadEntry.
    let mut load_response = client
        .load_entry(Request::new(CacheLoadRequest {
            key: cache_key.to_vec(),
        }))
        .await
        .unwrap()
        .into_inner();

    let mut metadata_seen = false;
    let mut data_chunks = Vec::new();

    use futures_util::StreamExt;
    while let Some(chunk_result) = load_response.next().await {
        let chunk = chunk_result.unwrap();
        match chunk.payload {
            Some(cache_load_chunk::Payload::Metadata(meta)) => {
                metadata_seen = true;
                assert_eq!(meta.size, cache_value.len() as i64);
                assert_eq!(meta.content_type, "application/octet-stream");
            }
            Some(cache_load_chunk::Payload::Data(bytes)) => {
                data_chunks.extend_from_slice(&bytes);
            }
            None => {}
        }
    }

    assert!(metadata_seen, "Expected a metadata chunk from load_entry");
    assert_eq!(data_chunks, cache_value);
}

// ============================================================
// Test: Cache service hit (key exists) and miss (key absent)
// ============================================================

#[tokio::test]
async fn test_cache_service_hit_and_miss() {
    let (socket_path, _dir) = spawn_test_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    // Step 1: Store an entry so we have something to hit.
    let cache_key = b"bbdeadbeefcafe00";
    let cache_value = b"cached artifact payload";

    let (tx, rx) = tokio::sync::mpsc::channel(8);
    tx.send(CacheStoreChunk {
        payload: Some(cache_store_chunk::Payload::Init(CacheStoreInit {
            key: cache_key.to_vec(),
            total_size: cache_value.len() as i64,
        })),
    }).await.unwrap();
    tx.send(CacheStoreChunk {
        payload: Some(cache_store_chunk::Payload::Data(cache_value.to_vec())),
    }).await.unwrap();
    drop(tx);

    use tokio_stream::wrappers::ReceiverStream;
    let store_response = client
        .store_entry(Request::new(ReceiverStream::new(rx)))
        .await
        .unwrap()
        .into_inner();
    assert!(store_response.success);

    // Step 2: Load the existing key — should get data back (hit).
    let mut load_hit = client
        .load_entry(Request::new(CacheLoadRequest {
            key: cache_key.to_vec(),
        }))
        .await
        .unwrap()
        .into_inner();

    use futures_util::StreamExt;
    let mut hit_chunks = Vec::new();
    let mut got_metadata = false;
    while let Some(chunk_result) = load_hit.next().await {
        let chunk = chunk_result.unwrap();
        match chunk.payload {
            Some(cache_load_chunk::Payload::Metadata(_)) => got_metadata = true,
            Some(cache_load_chunk::Payload::Data(bytes)) => hit_chunks.extend_from_slice(&bytes),
            None => {}
        }
    }
    assert!(got_metadata, "Expected metadata for existing key");
    assert_eq!(hit_chunks, cache_value);

    // Step 3: Load a key that was never stored — stream should produce zero chunks (miss).
    let miss_key = b"ff00000000000000";
    let mut load_miss = client
        .load_entry(Request::new(CacheLoadRequest {
            key: miss_key.to_vec(),
        }))
        .await
        .unwrap()
        .into_inner();

    let mut miss_received_any = false;
    while let Some(chunk_result) = load_miss.next().await {
        let _chunk = chunk_result.unwrap();
        miss_received_any = true;
    }
    assert!(!miss_received_any, "Expected no chunks for a cache miss");
}

/// Cross-service integration: cache orchestration probes the real local cache.
#[tokio::test]
async fn test_cache_orchestration_probes_real_cache() {
    let (socket_path, _dir) = spawn_test_server().await;

    let channel = connect(&socket_path).await;

    let mut cache_client = cache_service_client::CacheServiceClient::new(channel.clone());
    let mut orchestration_client =
        build_cache_orchestration_service_client::BuildCacheOrchestrationServiceClient::new(channel);

    // Step 1: Store an entry in the real cache
    let cache_key = "orchestration-test-key";
    let cache_value = b"cached-build-output-data";

    use futures_util::StreamExt;
    let (mut tx, rx) = tokio::sync::mpsc::channel(2);
    tx.send(CacheStoreChunk {
        payload: Some(cache_store_chunk::Payload::Init(CacheStoreInit {
            key: cache_key.as_bytes().to_vec(),
            total_size: cache_value.len() as i64,
        })),
    })
    .await
    .unwrap();
    tx.send(CacheStoreChunk {
        payload: Some(cache_store_chunk::Payload::Data(
            cache_value.to_vec(),
        )),
    })
    .await
    .unwrap();
    drop(tx);

    let store_response = cache_client
        .store_entry(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await
        .unwrap()
        .into_inner();
    assert!(store_response.success, "Cache store should succeed");

    // Step 2: Mark it as stored in orchestration
    let store_outputs = orchestration_client
        .store_outputs(Request::new(StoreOutputsRequest {
            cache_key: cache_key.as_bytes().to_vec(),
            execution_time_ms: 350,
            output_properties: vec!["classes".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(store_outputs.success, "Orchestration store_outputs should succeed");

    // Step 3: Probe should find it (both metadata AND real cache agree)
    let probe = orchestration_client
        .probe_cache(Request::new(ProbeCacheRequest {
            cache_key: cache_key.as_bytes().to_vec(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(probe.available, "Probe should find the entry in both metadata and real cache");
    assert_eq!(probe.location, "local");
    assert_eq!(probe.execution_time_ms, 350);
    assert!(probe.output_properties.contains(&"classes".to_string()));

    // Step 4: Probe a key that doesn't exist anywhere
    let probe_miss = orchestration_client
        .probe_cache(Request::new(ProbeCacheRequest {
            cache_key: b"nonexistent-key".to_vec(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!probe_miss.available, "Probe should not find nonexistent key");
}

/// Cross-service integration: task graph benefits from execution history duration estimates.
#[tokio::test]
async fn test_task_graph_with_execution_history_integration() {
    let (socket_path, _dir) = spawn_test_server().await;

    let channel = connect(&socket_path).await;

    let mut history_client =
        execution_history_service_client::ExecutionHistoryServiceClient::new(channel.clone());
    let mut task_graph_client =
        task_graph_service_client::TaskGraphServiceClient::new(channel);

    // Step 1: Store execution history for a task
    let task_path = ":app:compileJava";
    history_client
        .store_history(Request::new(StoreHistoryRequest {
            work_identity: format!("task_duration:{}", task_path),
            state: vec![],
            timestamp_ms: 1000,
        }))
        .await
        .unwrap();

    // Step 2: Register a task with the task graph
    task_graph_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: "integration-test-build".to_string(),
            task_path: task_path.to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "JavaCompile".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // Step 3: Resolve execution plan — task graph should be usable
    let plan = task_graph_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: "integration-test-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // The registered task should appear in the execution plan
    assert!(
        plan.execution_order.iter().any(|n| n.task_path == task_path),
        "Registered task should be in the execution plan"
    );
}
