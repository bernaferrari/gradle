/// Differential testing: verifies Rust service outputs are deterministic and
/// consistent across repeated invocations with the same inputs.
///
/// Unlike integration tests which check individual service correctness,
/// differential tests validate that the *same inputs always produce the same
/// outputs* — catching non-determinism, race conditions, and ordering bugs.
use std::sync::Arc;

use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::Request;

use gradle_substrate_daemon::proto::*;

use gradle_substrate_daemon::server::{
    artifact_publishing::ArtifactPublishingServiceImpl,
    bootstrap::BootstrapServiceImpl,
    build_comparison::BuildComparisonServiceImpl,
    build_event_stream::BuildEventStreamServiceImpl,
    build_init::BuildInitServiceImpl,
    build_layout::BuildLayoutServiceImpl,
    build_metrics::BuildMetricsServiceImpl,
    build_operations::BuildOperationsServiceImpl,
    build_result::BuildResultServiceImpl,
    cache::CacheServiceImpl,
    cache_orchestration::BuildCacheOrchestrationServiceImpl,
    config_cache::ConfigurationCacheServiceImpl,
    configuration::ConfigurationServiceImpl,
    console::ConsoleServiceImpl,
    control::ControlServiceImpl,
    dag_executor::DagExecutorServiceImpl,
    dependency_resolution::DependencyResolutionServiceImpl,
    event_dispatcher::EventDispatcher,
    exec::ExecServiceImpl,
    execution_history::ExecutionHistoryServiceImpl,
    execution_plan::ExecutionPlanServiceImpl,
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
    work::{WorkServiceImpl, WorkerScheduler},
    worker_process::WorkerProcessServiceImpl,
};

/// Spawns a full gRPC server for differential testing.
async fn spawn_server() -> (String, tempfile::TempDir) {
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
    let artifact_store_dir = dir.path().join("artifacts");
    std::fs::create_dir_all(&artifact_store_dir).unwrap();

    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    let control = ControlServiceImpl::new(shutdown_tx);
    let hash = HashServiceImpl;
    let cache = CacheServiceImpl::new(cache_dir.clone());
    let cache_local_store = cache.local_store();
    let exec = ExecServiceImpl::new();
    let work_scheduler = Arc::new(WorkerScheduler::new(4));
    let work = WorkServiceImpl::new(work_scheduler.clone());
    let shared_history = Arc::new(ExecutionHistoryServiceImpl::new(history_dir.clone()));
    let execution_plan = ExecutionPlanServiceImpl::with_persistent_history(
        work_scheduler.clone(),
        Arc::clone(&shared_history),
    );
    let execution_plan_arc = Arc::new(execution_plan);
    let execution_plan_server = ExecutionPlanServiceImpl::with_persistent_history(
        work_scheduler.clone(),
        Arc::clone(&shared_history),
    );
    let cache_orchestration =
        BuildCacheOrchestrationServiceImpl::with_local_cache(cache_local_store);
    let file_fingerprint = FileFingerprintServiceImpl::new();
    let value_snapshot = ValueSnapshotServiceImpl::new();
    let task_graph = Arc::new(TaskGraphServiceImpl::with_history(Arc::clone(
        &shared_history,
    )));
    let execution_history = ExecutionHistoryServiceImpl::new(history_dir.clone());
    let configuration = ConfigurationServiceImpl::new();
    let plugin = PluginServiceImpl::new();
    let build_operations = BuildOperationsServiceImpl::new();
    let bootstrap = BootstrapServiceImpl::new();
    let dependency_resolution = DependencyResolutionServiceImpl::new(artifact_store_dir);
    let file_watch = FileWatchServiceImpl::with_task_graph(Arc::clone(&task_graph));
    let config_cache = ConfigurationCacheServiceImpl::new(config_cache_dir.clone());
    let toolchain = ToolchainServiceImpl::new(toolchain_dir);
    let console = Arc::new(ConsoleServiceImpl::new());
    let build_metrics = Arc::new(BuildMetricsServiceImpl::new());
    let event_dispatchers: Vec<Arc<dyn EventDispatcher>> = vec![
        Arc::clone(&console) as Arc<dyn EventDispatcher>,
        Arc::clone(&build_metrics) as Arc<dyn EventDispatcher>,
    ];
    let build_event_stream =
        BuildEventStreamServiceImpl::with_dispatchers(event_dispatchers.clone());
    let dag_executor = DagExecutorServiceImpl::new(
        work_scheduler.clone(),
        Arc::clone(&task_graph),
        execution_plan_arc,
        event_dispatchers,
    );
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

    tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(control_service_server::ControlServiceServer::new(control))
            .add_service(dag_executor_service_server::DagExecutorServiceServer::new(
                dag_executor,
            ))
            .add_service(hash_service_server::HashServiceServer::new(hash))
            .add_service(cache_service_server::CacheServiceServer::new(cache))
            .add_service(exec_service_server::ExecServiceServer::new(exec))
            .add_service(work_service_server::WorkServiceServer::new(work))
            .add_service(
                execution_plan_service_server::ExecutionPlanServiceServer::new(
                    execution_plan_server,
                ),
            )
            .add_service(
                execution_history_service_server::ExecutionHistoryServiceServer::new(
                    execution_history,
                ),
            )
            .add_service(
                build_cache_orchestration_service_server::BuildCacheOrchestrationServiceServer::new(
                    cache_orchestration,
                ),
            )
            .add_service(
                file_fingerprint_service_server::FileFingerprintServiceServer::new(
                    file_fingerprint,
                ),
            )
            .add_service(
                value_snapshot_service_server::ValueSnapshotServiceServer::new(value_snapshot),
            )
            .add_service(task_graph_service_server::TaskGraphServiceServer::new(
                (*task_graph).clone(),
            ))
            .add_service(
                configuration_service_server::ConfigurationServiceServer::new(configuration),
            )
            .add_service(plugin_service_server::PluginServiceServer::new(plugin))
            .add_service(
                build_operations_service_server::BuildOperationsServiceServer::new(
                    build_operations,
                ),
            )
            .add_service(bootstrap_service_server::BootstrapServiceServer::new(
                bootstrap,
            ))
            .add_service(
                dependency_resolution_service_server::DependencyResolutionServiceServer::new(
                    dependency_resolution,
                ),
            )
            .add_service(file_watch_service_server::FileWatchServiceServer::new(
                file_watch,
            ))
            .add_service(
                configuration_cache_service_server::ConfigurationCacheServiceServer::new(
                    config_cache,
                ),
            )
            .add_service(toolchain_service_server::ToolchainServiceServer::new(
                toolchain,
            ))
            .add_service(
                build_event_stream_service_server::BuildEventStreamServiceServer::new(
                    build_event_stream,
                ),
            )
            .add_service(
                worker_process_service_server::WorkerProcessServiceServer::new(worker_process),
            )
            .add_service(build_layout_service_server::BuildLayoutServiceServer::new(
                build_layout,
            ))
            .add_service(build_result_service_server::BuildResultServiceServer::new(
                build_result,
            ))
            .add_service(
                problem_reporting_service_server::ProblemReportingServiceServer::new(
                    problem_reporting,
                ),
            )
            .add_service(
                resource_management_service_server::ResourceManagementServiceServer::new(
                    resource_management,
                ),
            )
            .add_service(
                build_comparison_service_server::BuildComparisonServiceServer::new(
                    build_comparison,
                ),
            )
            .add_service(console_service_server::ConsoleServiceServer::new(
                (*console).clone(),
            ))
            .add_service(
                test_execution_service_server::TestExecutionServiceServer::new(test_execution),
            )
            .add_service(
                artifact_publishing_service_server::ArtifactPublishingServiceServer::new(
                    artifact_publishing,
                ),
            )
            .add_service(build_init_service_server::BuildInitServiceServer::new(
                build_init,
            ))
            .add_service(
                incremental_compilation_service_server::IncrementalCompilationServiceServer::new(
                    incremental_compilation,
                ),
            )
            .add_service(
                build_metrics_service_server::BuildMetricsServiceServer::new(
                    (*build_metrics).clone(),
                ),
            )
            .add_service(
                garbage_collection_service_server::GarbageCollectionServiceServer::new(
                    garbage_collection,
                ),
            )
            .serve_with_incoming(UnixListenerStream::new(listener))
            .await;
    });

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
    pairs
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ============================================================
// Differential Test 1: Hash determinism — same file, same hash every time
// ============================================================

#[tokio::test]
async fn test_hash_determinism_across_repeated_calls() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let dir = tempfile::tempdir().unwrap();
    let file_path = make_test_file(
        dir.path(),
        "deterministic.txt",
        b"deterministic content for hash verification",
    );

    // Compute hash 5 times and verify all results are identical
    let mut hashes = Vec::new();
    for _ in 0..5 {
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
        assert!(!response.results[0].hash_bytes.is_empty());
        hashes.push(response.results.into_iter().next().unwrap().hash_bytes);
    }

    // All 5 hashes must be identical
    for i in 1..hashes.len() {
        assert_eq!(
            hashes[0], hashes[i],
            "Hash at iteration {} differs from first",
            i
        );
    }
}

// ============================================================
// Differential Test 2: Hash consistency — different files produce different hashes
// ============================================================

#[tokio::test]
async fn test_hash_different_files_produce_different_hashes() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let dir = tempfile::tempdir().unwrap();
    let file_a = make_test_file(dir.path(), "file_a.txt", b"content alpha");
    let file_b = make_test_file(dir.path(), "file_b.txt", b"content beta");
    let file_c = make_test_file(dir.path(), "file_c.txt", b"content alpha"); // same as A

    let response = client
        .hash_batch(Request::new(HashBatchRequest {
            files: vec![
                FileToHash {
                    absolute_path: file_a,
                    length: 0,
                    last_modified: 0,
                },
                FileToHash {
                    absolute_path: file_b,
                    length: 0,
                    last_modified: 0,
                },
                FileToHash {
                    absolute_path: file_c,
                    length: 0,
                    last_modified: 0,
                },
            ],
            algorithm: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.results.len(), 3);
    // A and C must have identical hashes
    assert_eq!(
        response.results[0].hash_bytes,
        response.results[2].hash_bytes
    );
    // A and B must have different hashes
    assert_ne!(
        response.results[0].hash_bytes,
        response.results[1].hash_bytes
    );
}

// ============================================================
// Differential Test 3: Cache key determinism — same inputs produce same key
// ============================================================

#[tokio::test]
async fn test_cache_key_determinism() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client =
        build_cache_orchestration_service_client::BuildCacheOrchestrationServiceClient::new(
            channel,
        );

    // Compute cache key twice with identical inputs
    let key1 = client
        .compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "abc123".to_string(),
            input_property_hashes: make_prop_map(vec![("source", "v1"), ("target", "v2")]),
            input_file_hashes: make_prop_map(vec![("classpath", "hash3")]),
            output_property_names: vec!["classes".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    let key2 = client
        .compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "abc123".to_string(),
            input_property_hashes: make_prop_map(vec![("source", "v1"), ("target", "v2")]),
            input_file_hashes: make_prop_map(vec![("classpath", "hash3")]),
            output_property_names: vec!["classes".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        key1.cache_key, key2.cache_key,
        "Identical inputs must produce identical cache keys"
    );
    assert_eq!(key1.cache_key_string, key2.cache_key_string);

    // Different inputs should produce different keys
    let key3 = client
        .compute_cache_key(Request::new(ComputeCacheKeyRequest {
            work_identity: ":compileJava".to_string(),
            implementation_hash: "xyz999".to_string(), // different
            input_property_hashes: make_prop_map(vec![("source", "v1"), ("target", "v2")]),
            input_file_hashes: make_prop_map(vec![("classpath", "hash3")]),
            output_property_names: vec!["classes".to_string()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_ne!(
        key1.cache_key, key3.cache_key,
        "Different implementation hash must produce different cache key"
    );
}

// ============================================================
// Differential Test 4: Value snapshot determinism
// ============================================================

#[tokio::test]
async fn test_value_snapshot_determinism() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = value_snapshot_service_client::ValueSnapshotServiceClient::new(channel);

    let request = SnapshotValuesRequest {
        values: vec![
            PropertyValue {
                name: "source".to_string(),
                value: Some(property_value::Value::StringValue(
                    "src/main/java".to_string(),
                )),
                type_name: "java.lang.String".to_string(),
            },
            PropertyValue {
                name: "target".to_string(),
                value: Some(property_value::Value::StringValue("1.8".to_string())),
                type_name: "java.lang.String".to_string(),
            },
        ],
        implementation_fingerprint: "impl-abc".to_string(),
    };

    let resp1 = client
        .snapshot_values(Request::new(request.clone()))
        .await
        .unwrap()
        .into_inner();

    let resp2 = client
        .snapshot_values(Request::new(request))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp1.composite_hash, resp2.composite_hash,
        "Composite hash must be deterministic"
    );
    assert_eq!(resp1.results.len(), resp2.results.len());
    for (r1, r2) in resp1.results.iter().zip(resp2.results.iter()) {
        assert_eq!(
            r1.fingerprint, r2.fingerprint,
            "Fingerprint for {} must be deterministic",
            r1.name
        );
    }
}

// ============================================================
// Differential Test 5: File fingerprint determinism
// ============================================================

#[tokio::test]
async fn test_file_fingerprint_determinism() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = file_fingerprint_service_client::FileFingerprintServiceClient::new(channel);

    let dir = tempfile::tempdir().unwrap();
    let file_path = make_test_file(dir.path(), "fingerprint.txt", b"fingerprint test content");

    let request = FingerprintFilesRequest {
        files: vec![FileToFingerprint {
            absolute_path: file_path.clone(),
            r#type: FingerprintType::FingerprintFile as i32,
        }],
        normalization_strategy: "ABSOLUTE_PATH".to_string(),
        ignore_patterns: vec![],
    };

    let resp1 = client
        .fingerprint_files(Request::new(request.clone()))
        .await
        .unwrap()
        .into_inner();

    let resp2 = client
        .fingerprint_files(Request::new(request))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp1.collection_hash, resp2.collection_hash,
        "Collection hash must be deterministic"
    );
    assert_eq!(resp1.entries.len(), resp2.entries.len());
    for (e1, e2) in resp1.entries.iter().zip(resp2.entries.iter()) {
        assert_eq!(e1.hash, e2.hash);
        assert_eq!(e1.size, e2.size);
    }
}

// ============================================================
// Differential Test 6: Execution plan consistency — same inputs yield same outcome
// ============================================================

#[tokio::test]
async fn test_execution_plan_consistency() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = execution_plan_service_client::ExecutionPlanServiceClient::new(channel);

    let work = WorkMetadata {
        work_identity: ":compileJava".to_string(),
        display_name: "compileJava".to_string(),
        implementation_class: "JavaCompile".to_string(),
        input_properties: make_prop_map(vec![("source", "src/main/java")]),
        input_file_fingerprints: make_prop_map(vec![("classpath", "abc123")]),
        caching_enabled: true,
        can_load_from_cache: true,
        has_previous_execution_state: false,
        rebuild_reasons: vec![],
    };

    let resp1 = client
        .predict_outcome(Request::new(PredictOutcomeRequest {
            work: Some(work.clone()),
        }))
        .await
        .unwrap()
        .into_inner();

    let resp2 = client
        .predict_outcome(Request::new(PredictOutcomeRequest { work: Some(work) }))
        .await
        .unwrap()
        .into_inner();

    // Same inputs should produce the same prediction
    assert_eq!(
        resp1.predicted_outcome, resp2.predicted_outcome,
        "Same inputs must yield same predicted outcome"
    );
}

// ============================================================
// Differential Test 7: Task graph resolution determinism
// ============================================================

#[tokio::test]
async fn test_task_graph_resolution_determinism() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;

    let register_and_resolve = |ch: Channel| async {
        let mut tc = task_graph_service_client::TaskGraphServiceClient::new(ch);

        tc.register_task(Request::new(RegisterTaskRequest {
            build_id: "det-build".to_string(),
            task_path: ":compileJava".to_string(),
            depends_on: vec![":processResources".to_string()],
            task_type: "JavaCompile".to_string(),
            input_files: vec![],
            should_execute: true,
        }))
        .await
        .unwrap();

        tc.register_task(Request::new(RegisterTaskRequest {
            build_id: "det-build".to_string(),
            task_path: ":processResources".to_string(),
            depends_on: vec![],
            task_type: "ProcessResources".to_string(),
            input_files: vec![],
            should_execute: true,
        }))
        .await
        .unwrap();

        tc.resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: "det-build".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
    };

    let plan1 = register_and_resolve(channel.clone()).await;
    let plan2 = register_and_resolve(channel.clone()).await;

    assert_eq!(plan1.total_tasks, plan2.total_tasks);
    assert_eq!(plan1.has_cycles, plan2.has_cycles);
}

// ============================================================
// Differential Test 8: Build comparison consistency
// ============================================================

#[tokio::test]
async fn test_build_comparison_determinism() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client =
        build_comparison_service_client::BuildComparisonServiceClient::new(channel.clone());

    // Record identical build data twice and compare
    let mut durations: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    durations.insert(":compileJava".to_string(), 1000);
    durations.insert(":test".to_string(), 2000);
    let mut outcomes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    outcomes.insert(":compileJava".to_string(), "SUCCESS".to_string());
    outcomes.insert(":test".to_string(), "SUCCESS".to_string());

    let mut c_rec =
        build_comparison_service_client::BuildComparisonServiceClient::new(channel.clone());
    for build_id in ["baseline-a", "baseline-b"] {
        c_rec
            .record_build_data(Request::new(RecordBuildDataRequest {
                snapshot: Some(BuildDataSnapshot {
                    build_id: build_id.to_string(),
                    start_time_ms: 0,
                    end_time_ms: 3000,
                    task_durations: durations.clone(),
                    task_outcomes: outcomes.clone(),
                    task_order: vec![":compileJava".to_string(), ":test".to_string()],
                    root_dir: "/tmp/project".to_string(),
                    input_properties: vec![],
                }),
            }))
            .await
            .unwrap();
    }

    let comp1 = client
        .start_comparison(Request::new(StartComparisonRequest {
            baseline_build_id: "baseline-a".to_string(),
            candidate_build_id: "baseline-b".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    let result1 = client
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: comp1.comparison_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    // Identical builds should show no regressions and no improvements
    assert_eq!(
        result1.summary.unwrap().tasks_with_regression,
        0,
        "Identical builds should have zero regressions"
    );
    assert!(result1.task_comparisons.len() >= 2);

    for tc in &result1.task_comparisons {
        assert_eq!(
            tc.duration_ratio, 1.0,
            "Task {} should have ratio 1.0 for identical builds",
            tc.task_path
        );
    }
}

// ============================================================
// Differential Test 9: Plugin system determinism
// ============================================================

#[tokio::test]
async fn test_plugin_apply_order_determinism() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;

    let setup_and_get_plugins = |ch: Channel| async {
        let mut pc = plugin_service_client::PluginServiceClient::new(ch);
        let project = format!(
            "det-plugin-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        pc.register_plugin(Request::new(RegisterPluginRequest {
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

        pc.register_plugin(Request::new(RegisterPluginRequest {
            plugin_id: "kotlin".to_string(),
            plugin_class: "KotlinPlugin".to_string(),
            version: "1.9".to_string(),
            is_imperative: false,
            applies_to: vec![],
            requires: vec!["java".to_string()],
            conflicts_with: vec![],
        }))
        .await
        .unwrap();

        pc.apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "java".to_string(),
            project_path: project.clone(),
            apply_order: 0,
        }))
        .await
        .unwrap();

        pc.apply_plugin(Request::new(ApplyPluginRequest {
            plugin_id: "kotlin".to_string(),
            project_path: project.clone(),
            apply_order: 1,
        }))
        .await
        .unwrap();

        pc.get_applied_plugins(Request::new(GetAppliedPluginsRequest {
            project_path: project.clone(),
        }))
        .await
        .unwrap()
        .into_inner()
    };

    let plugins1 = setup_and_get_plugins(channel.clone()).await;
    let plugins2 = setup_and_get_plugins(channel.clone()).await;

    assert_eq!(plugins1.plugins.len(), plugins2.plugins.len());
    for (p1, p2) in plugins1.plugins.iter().zip(plugins2.plugins.iter()) {
        assert_eq!(p1.plugin_id, p2.plugin_id);
        assert_eq!(p1.apply_order, p2.apply_order);
    }
}

// ============================================================
// Differential Test 10: Configuration cache determinism
// ============================================================

#[tokio::test]
async fn test_config_cache_roundtrip_determinism() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client =
        configuration_cache_service_client::ConfigurationCacheServiceClient::new(channel);

    // Store, load, re-store with same key, load again
    let store_req = StoreConfigCacheRequest {
        cache_key: "det-config-123".to_string(),
        serialized_config: vec![10, 20, 30, 40, 50],
        entry_count: 5,
        input_hashes: vec!["build.gradle".to_string(), "settings.gradle".to_string()],
        timestamp_ms: 1000,
        ..Default::default()
    };

    client
        .store_config_cache(Request::new(store_req.clone()))
        .await
        .unwrap();

    let load1 = client
        .load_config_cache(Request::new(LoadConfigCacheRequest {
            cache_key: "det-config-123".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // Store again (overwrite)
    client
        .store_config_cache(Request::new(store_req))
        .await
        .unwrap();

    let load2 = client
        .load_config_cache(Request::new(LoadConfigCacheRequest {
            cache_key: "det-config-123".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(load1.serialized_config, load2.serialized_config);
    assert_eq!(load1.found, load2.found);
}

// ============================================================
// Differential Test 11: Build metrics aggregation determinism
// ============================================================

#[tokio::test]
async fn test_build_metrics_aggregation_determinism() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;

    let record_and_get = |ch: Channel| async {
        let mut mc = build_metrics_service_client::BuildMetricsServiceClient::new(ch);
        let build_id = format!(
            "det-metrics-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        mc.record_metric(Request::new(RecordMetricRequest {
            build_id: build_id.clone(),
            event: Some(MetricEvent {
                name: "tasks.total".to_string(),
                value: "5".to_string(),
                metric_type: "counter".to_string(),
                tags: std::collections::HashMap::new(),
                timestamp_ms: 0,
            }),
        }))
        .await
        .unwrap();

        mc.record_metric(Request::new(RecordMetricRequest {
            build_id: build_id.clone(),
            event: Some(MetricEvent {
                name: "tasks.executed".to_string(),
                value: "3".to_string(),
                metric_type: "counter".to_string(),
                tags: std::collections::HashMap::new(),
                timestamp_ms: 0,
            }),
        }))
        .await
        .unwrap();

        mc.record_metric(Request::new(RecordMetricRequest {
            build_id: build_id.clone(),
            event: Some(MetricEvent {
                name: "tasks.cached".to_string(),
                value: "2".to_string(),
                metric_type: "counter".to_string(),
                tags: std::collections::HashMap::new(),
                timestamp_ms: 0,
            }),
        }))
        .await
        .unwrap();

        mc.get_performance_summary(Request::new(GetPerformanceSummaryRequest {
            build_id: build_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner()
    };

    let summary1 = record_and_get(channel.clone()).await;
    let summary2 = record_and_get(channel.clone()).await;

    let s1 = summary1.summary.as_ref().unwrap();
    let s2 = summary2.summary.as_ref().unwrap();
    assert_eq!(s1.total_tasks_executed, s2.total_tasks_executed);
    assert_eq!(s1.tasks_executed, s2.tasks_executed);
    assert_eq!(s1.tasks_from_cache, s2.tasks_from_cache);
}

// ============================================================
// Differential Test 12: Full build lifecycle determinism
// ============================================================

#[tokio::test]
async fn test_full_build_lifecycle_determinism() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;

    let run_lifecycle = |ch: Channel| async move {
        let build_id = format!(
            "det-lifecycle-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let mut bc = bootstrap_service_client::BootstrapServiceClient::new(ch.clone());
        bc.init_build(Request::new(InitBuildRequest {
            build_id: build_id.clone(),
            project_dir: "/tmp/det-test".to_string(),
            start_time_ms: 0,
            requested_parallelism: 4,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: String::new(),
        }))
        .await
        .unwrap();

        let mut rc = build_result_service_client::BuildResultServiceClient::new(ch.clone());
        rc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":compileJava".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 500,
                did_work: true,
                cache_key: String::new(),
                start_time_ms: 0,
                end_time_ms: 500,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

        rc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(TaskResult {
                task_path: ":test".to_string(),
                outcome: "FROM_CACHE".to_string(),
                duration_ms: 10,
                did_work: false,
                cache_key: "ck-test".to_string(),
                start_time_ms: 500,
                end_time_ms: 510,
                failure_message: String::new(),
                execution_reason: 0,
            }),
        }))
        .await
        .unwrap();

        let result = rc
            .get_build_result(Request::new(GetBuildResultRequest {
                build_id: build_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        result.outcome.unwrap()
    };

    let outcome1 = run_lifecycle(channel.clone()).await;
    let outcome2 = run_lifecycle(channel.clone()).await;

    assert_eq!(outcome1.overall_result, outcome2.overall_result);
    assert_eq!(outcome1.tasks_total, outcome2.tasks_total);
    assert_eq!(outcome1.tasks_executed, outcome2.tasks_executed);
    assert_eq!(outcome1.tasks_from_cache, outcome2.tasks_from_cache);
    assert_eq!(outcome1.total_duration_ms, outcome2.total_duration_ms);
}
