use std::sync::Arc;

use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::Request;

use gradle_substrate_daemon::proto::*;
use gradle_substrate_daemon::server::{
    dag_executor::DagExecutorServiceImpl,
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
    event_dispatcher::EventDispatcher,
};

/// Spawn a full gRPC server with dispatchers wired (same as main.rs).
async fn spawn_server_with_dispatchers() -> (String, tempfile::TempDir) {
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
    let cache_orchestration = BuildCacheOrchestrationServiceImpl::with_local_cache(cache_local_store);
    let file_fingerprint = FileFingerprintServiceImpl::new();
    let value_snapshot = ValueSnapshotServiceImpl::new();
    let task_graph = Arc::new(TaskGraphServiceImpl::with_history(Arc::clone(&shared_history)));
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
    let build_event_stream = BuildEventStreamServiceImpl::with_dispatchers(event_dispatchers.clone());
    let dag_executor = DagExecutorServiceImpl::new(work_scheduler.clone(), Arc::clone(&task_graph), event_dispatchers);
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
            .add_service(dag_executor_service_server::DagExecutorServiceServer::new(dag_executor))
            .add_service(hash_service_server::HashServiceServer::new(hash))
            .add_service(cache_service_server::CacheServiceServer::new(cache))
            .add_service(exec_service_server::ExecServiceServer::new(exec))
            .add_service(work_service_server::WorkServiceServer::new(work))
            .add_service(execution_plan_service_server::ExecutionPlanServiceServer::new(execution_plan))
            .add_service(execution_history_service_server::ExecutionHistoryServiceServer::new(execution_history))
            .add_service(build_cache_orchestration_service_server::BuildCacheOrchestrationServiceServer::new(cache_orchestration))
            .add_service(file_fingerprint_service_server::FileFingerprintServiceServer::new(file_fingerprint))
            .add_service(value_snapshot_service_server::ValueSnapshotServiceServer::new(value_snapshot))
            .add_service(task_graph_service_server::TaskGraphServiceServer::new((*task_graph).clone()))
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

/// Full build lifecycle E2E test validating cross-service event fan-out.
///
/// Flow: InitBuild → Send build_start event → Register tasks → Send task_start/task_finish
/// events → Send build_finish event → Verify metrics auto-recorded → Verify console auto-buffered
#[tokio::test]
async fn test_full_lifecycle_with_event_fanout() {
    let (socket_path, _dir) = spawn_server_with_dispatchers().await;
    let channel = connect(&socket_path).await;

    let build_id = "lifecycle-e2e";

    // 1. Bootstrap: init build
    let mut bootstrap_client = bootstrap_service_client::BootstrapServiceClient::new(channel.clone());
    let init = bootstrap_client
        .init_build(Request::new(InitBuildRequest {
            build_id: build_id.to_string(),
            project_dir: "/tmp/lifecycle-e2e".to_string(),
            start_time_ms: 0,
            requested_parallelism: 4,
            system_properties: Default::default(),
            requested_features: vec![],
            session_id: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!init.build_id.is_empty());

    // 2. Send build_start event (should auto-dispatch to metrics + console)
    let mut event_client =
        build_event_stream_service_client::BuildEventStreamServiceClient::new(channel.clone());
    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "build_start".to_string(),
            event_id: "evt-build-start".to_string(),
            properties: Default::default(),
            display_name: "Build".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    // 3. Register tasks in task graph
    let mut task_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());
    let tasks = vec![
        (":processResources", "ProcessResources", vec![]),
        (":compileJava", "JavaCompile", vec![":processResources".to_string()]),
        (":classes", "Lifecycle", vec![":compileJava".to_string(), ":processResources".to_string()]),
        (":test", "Test", vec![":classes".to_string()]),
    ];
    for (path, task_type, deps) in &tasks {
        task_client
            .register_task(Request::new(RegisterTaskRequest {
                build_id: build_id.to_string(),
                task_path: path.to_string(),
                depends_on: deps.clone(),
                task_type: task_type.to_string(),
                input_files: vec![],
                should_execute: true,
            }))
            .await
            .unwrap();
    }

    // 4. Simulate task execution via events (each task_start + task_finish)
    // task_start events should auto-increment tasks.total in metrics
    // task_finish events should auto-increment tasks.executed/cached/failed
    let task_outcomes = vec![
        (":processResources", "UP_TO_DATE", 50),
        (":compileJava", "SUCCESS", 800),
        (":classes", "SUCCESS", 5),
        (":test", "SUCCESS", 1200),
    ];

    for (task_path, outcome, duration_ms) in &task_outcomes {
        event_client
            .send_build_event(Request::new(SendBuildEventRequest {
                build_id: build_id.to_string(),
                event_type: "task_start".to_string(),
                event_id: format!("evt-{}-start", task_path),
                properties: Default::default(),
                display_name: task_path.to_string(),
                parent_id: String::new(),
            }))
            .await
            .unwrap();

        event_client
            .send_build_event(Request::new(SendBuildEventRequest {
                build_id: build_id.to_string(),
                event_type: "task_finish".to_string(),
                event_id: format!("evt-{}-finish", task_path),
                properties: {
                    let mut p = std::collections::HashMap::new();
                    p.insert("outcome".to_string(), outcome.to_string());
                    p.insert("duration_ms".to_string(), duration_ms.to_string());
                    p
                },
                display_name: task_path.to_string(),
                parent_id: String::new(),
            }))
            .await
            .unwrap();
    }

    // 5. Send build_finish event
    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "build_finish".to_string(),
            event_id: "evt-build-finish".to_string(),
            properties: {
                let mut p = std::collections::HashMap::new();
                p.insert("outcome".to_string(), "SUCCESS".to_string());
                p
            },
            display_name: "Build".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    // 6. Verify metrics were auto-recorded by event fan-out
    let mut metrics_client =
        build_metrics_service_client::BuildMetricsServiceClient::new(channel.clone());

    let metrics_resp = metrics_client
        .get_metrics(Request::new(GetMetricsRequest {
            build_id: build_id.to_string(),
            metric_names: vec![
                "tasks.total".to_string(),
                "tasks.executed".to_string(),
                "tasks.up_to_date".to_string(),
                "tasks.failed".to_string(),
                "build.start".to_string(),
                "build.end".to_string(),
            ],
            since_ms: 0,
        }))
        .await
        .unwrap()
        .into_inner();

    let metric_map: std::collections::HashMap<String, &MetricSnapshot> = metrics_resp
        .metrics
        .iter()
        .map(|m| (m.name.clone(), m))
        .collect();

    // tasks.total should be 4 (4 task_start events)
    assert!(
        metric_map.contains_key("tasks.total"),
        "tasks.total should be auto-recorded from task_start events"
    );
    assert_eq!(
        metric_map["tasks.total"].count, 4,
        "Expected 4 task_start events"
    );

    // tasks.executed should be 3 (SUCCESS outcomes for compileJava, classes, test)
    assert!(
        metric_map.contains_key("tasks.executed"),
        "tasks.executed should be auto-recorded from task_finish SUCCESS events"
    );
    assert_eq!(metric_map["tasks.executed"].count, 3);

    // tasks.up_to_date should be 1 (processResources UP_TO_DATE)
    assert!(
        metric_map.contains_key("tasks.up_to_date"),
        "tasks.up_to_date should be auto-recorded from task_finish UP_TO_DATE events"
    );
    assert_eq!(metric_map["tasks.up_to_date"].count, 1);

    // tasks.failed should be 0
    let failed_count = metric_map
        .get("tasks.failed")
        .map(|m| m.count)
        .unwrap_or(0);
    assert_eq!(failed_count, 0, "No tasks should have failed");

    // build.start and build.end should be recorded
    assert!(metric_map.contains_key("build.start"));
    assert!(metric_map.contains_key("build.end"));

    // 7. Verify performance summary reflects the auto-recorded data
    let summary_resp = metrics_client
        .get_performance_summary(Request::new(GetPerformanceSummaryRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(summary_resp.summary.is_some());
    let summary = summary_resp.summary.unwrap();
    assert_eq!(summary.build_id, build_id);
    assert_eq!(summary.total_tasks_executed, 4);
    assert_eq!(summary.tasks_executed, 3);
    assert_eq!(summary.tasks_up_to_date, 1);
    assert_eq!(summary.tasks_failed, 0);
    assert_eq!(summary.build_outcome, "SUCCESS");

    // 8. Verify event log has all events (console buffering is internal, verified via unit tests)
    let event_log = event_client
        .get_event_log(Request::new(GetEventLogRequest {
            build_id: build_id.to_string(),
            since_timestamp_ms: 0,
            max_events: 100,
            event_types: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    // 1 build_start + 4 task_start + 4 task_finish + 1 build_finish = 10
    assert_eq!(event_log.total_events, 10);
    assert_eq!(event_log.events.len(), 10);
    assert_eq!(event_log.events[0].event_type, "build_start");
    assert_eq!(event_log.events[9].event_type, "build_finish");

    // 10. Verify task graph has all tasks registered
    let plan = task_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(plan.total_tasks, 4);
    assert!(!plan.has_cycles);
}

/// Test that event fan-out correctly handles FAILED task outcomes.
#[tokio::test]
async fn test_event_fanout_handles_failed_tasks() {
    let (socket_path, _dir) = spawn_server_with_dispatchers().await;
    let channel = connect(&socket_path).await;

    let build_id = "failed-tasks-e2e";
    let mut event_client =
        build_event_stream_service_client::BuildEventStreamServiceClient::new(channel.clone());

    // Send build_start
    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "build_start".to_string(),
            event_id: "evt-start".to_string(),
            properties: Default::default(),
            display_name: "Build".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    // 2 task starts, 1 success, 1 failure
    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "task_start".to_string(),
            event_id: "evt-t1-start".to_string(),
            properties: Default::default(),
            display_name: ":compileJava".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "task_start".to_string(),
            event_id: "evt-t2-start".to_string(),
            properties: Default::default(),
            display_name: ":test".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "task_finish".to_string(),
            event_id: "evt-t1-finish".to_string(),
            properties: {
                let mut p = std::collections::HashMap::new();
                p.insert("outcome".to_string(), "SUCCESS".to_string());
                p
            },
            display_name: ":compileJava".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "task_finish".to_string(),
            event_id: "evt-t2-finish".to_string(),
            properties: {
                let mut p = std::collections::HashMap::new();
                p.insert("outcome".to_string(), "FAILED".to_string());
                p
            },
            display_name: ":test".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    // Send build_finish with FAILED outcome
    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "build_finish".to_string(),
            event_id: "evt-end".to_string(),
            properties: {
                let mut p = std::collections::HashMap::new();
                p.insert("outcome".to_string(), "FAILED".to_string());
                p
            },
            display_name: "Build".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    // Verify metrics
    let mut metrics_client =
        build_metrics_service_client::BuildMetricsServiceClient::new(channel.clone());

    let metrics_resp = metrics_client
        .get_metrics(Request::new(GetMetricsRequest {
            build_id: build_id.to_string(),
            metric_names: vec![
                "tasks.total".to_string(),
                "tasks.executed".to_string(),
                "tasks.failed".to_string(),
            ],
            since_ms: 0,
        }))
        .await
        .unwrap()
        .into_inner();

    let metric_map: std::collections::HashMap<String, &MetricSnapshot> = metrics_resp
        .metrics
        .iter()
        .map(|m| (m.name.clone(), m))
        .collect();

    assert_eq!(metric_map["tasks.total"].count, 2);
    assert_eq!(metric_map["tasks.executed"].count, 1); // SUCCESS compileJava
    assert_eq!(metric_map["tasks.failed"].count, 1); // FAILED test

    // Verify performance summary shows FAILED build
    let summary = metrics_client
        .get_performance_summary(Request::new(GetPerformanceSummaryRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .summary
        .unwrap();

    assert_eq!(summary.build_outcome, "FAILED");
    assert_eq!(summary.tasks_failed, 1);
}

/// Test that FROM_CACHE task outcomes are correctly tracked.
#[tokio::test]
async fn test_event_fanout_handles_cached_tasks() {
    let (socket_path, _dir) = spawn_server_with_dispatchers().await;
    let channel = connect(&socket_path).await;

    let build_id = "cached-tasks-e2e";
    let mut event_client =
        build_event_stream_service_client::BuildEventStreamServiceClient::new(channel.clone());

    // Send build_start
    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "build_start".to_string(),
            event_id: "evt-start".to_string(),
            properties: Default::default(),
            display_name: "Build".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    // 3 task starts, all FROM_CACHE
    for i in 0..3 {
        event_client
            .send_build_event(Request::new(SendBuildEventRequest {
                build_id: build_id.to_string(),
                event_type: "task_start".to_string(),
                event_id: format!("evt-t{}-start", i),
                properties: Default::default(),
                display_name: format!(":task_{}", i),
                parent_id: String::new(),
            }))
            .await
            .unwrap();

        event_client
            .send_build_event(Request::new(SendBuildEventRequest {
                build_id: build_id.to_string(),
                event_type: "task_finish".to_string(),
                event_id: format!("evt-t{}-finish", i),
                properties: {
                    let mut p = std::collections::HashMap::new();
                    p.insert("outcome".to_string(), "FROM_CACHE".to_string());
                    p
                },
                display_name: format!(":task_{}", i),
                parent_id: String::new(),
            }))
            .await
            .unwrap();
    }

    event_client
        .send_build_event(Request::new(SendBuildEventRequest {
            build_id: build_id.to_string(),
            event_type: "build_finish".to_string(),
            event_id: "evt-end".to_string(),
            properties: {
                let mut p = std::collections::HashMap::new();
                p.insert("outcome".to_string(), "SUCCESS".to_string());
                p
            },
            display_name: "Build".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

    let mut metrics_client =
        build_metrics_service_client::BuildMetricsServiceClient::new(channel);

    let summary = metrics_client
        .get_performance_summary(Request::new(GetPerformanceSummaryRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .summary
        .unwrap();

    assert_eq!(summary.total_tasks_executed, 3);
    assert_eq!(summary.tasks_from_cache, 3);
    assert_eq!(summary.tasks_executed, 0);
    assert_eq!(summary.tasks_failed, 0);
    assert_eq!(summary.build_outcome, "SUCCESS");
}
