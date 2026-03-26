/// Differential execution plan testing: validates Rust TaskGraphService and
/// DagExecutorService outputs against known-correct topological orderings
/// and execution behaviors.
///
/// Tests cover:
/// - Multiple execution plans with DAG structures
/// - Topological ordering verification of get_next_task calls
/// - Cycle detection
/// - Plan completion (all tasks executed)
/// - Dependency satisfaction correctness
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::sync::Arc;

use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::Request;

use gradle_substrate_daemon::proto::*;
use gradle_substrate_daemon::server::{
    artifact_publishing::ArtifactPublishingServiceImpl, bootstrap::BootstrapServiceImpl,
    build_comparison::BuildComparisonServiceImpl, build_event_stream::BuildEventStreamServiceImpl,
    build_init::BuildInitServiceImpl, build_layout::BuildLayoutServiceImpl,
    build_metrics::BuildMetricsServiceImpl, build_operations::BuildOperationsServiceImpl,
    build_result::BuildResultServiceImpl, cache::CacheServiceImpl,
    cache_orchestration::BuildCacheOrchestrationServiceImpl,
    config_cache::ConfigurationCacheServiceImpl, configuration::ConfigurationServiceImpl,
    console::ConsoleServiceImpl, control::ControlServiceImpl, dag_executor::DagExecutorServiceImpl,
    dependency_resolution::DependencyResolutionServiceImpl, event_dispatcher::EventDispatcher,
    exec::ExecServiceImpl, execution_history::ExecutionHistoryServiceImpl,
    file_fingerprint::FileFingerprintServiceImpl, file_watch::FileWatchServiceImpl,
    garbage_collection::GarbageCollectionServiceImpl, hash::HashServiceImpl,
    incremental_compilation::IncrementalCompilationServiceImpl, plugin::PluginServiceImpl,
    problem_reporting::ProblemReportingServiceImpl,
    resource_management::ResourceManagementServiceImpl, task_graph::TaskGraphServiceImpl,
    test_execution::TestExecutionServiceImpl, toolchain::ToolchainServiceImpl,
    value_snapshot::ValueSnapshotServiceImpl, work::WorkerScheduler,
    worker_process::WorkerProcessServiceImpl,
};

// ============================================================
// Test server setup
// ============================================================

async fn spawn_server() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("test.sock");
    let socket_path_str = socket_path.to_string_lossy().to_string();

    let cache_dir = dir.path().join("cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let history_dir = dir.path().join("history");
    fs::create_dir_all(&history_dir).unwrap();
    let config_cache_dir = dir.path().join("config-cache");
    fs::create_dir_all(&config_cache_dir).unwrap();
    let toolchain_dir = dir.path().join("toolchains");
    fs::create_dir_all(&toolchain_dir).unwrap();
    let artifact_store_dir = dir.path().join("artifacts");
    fs::create_dir_all(&artifact_store_dir).unwrap();

    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    let control = ControlServiceImpl::new(shutdown_tx);
    let hash = HashServiceImpl;
    let cache = CacheServiceImpl::new(cache_dir.clone());
    let cache_local_store = cache.local_store();
    let exec = ExecServiceImpl::new();
    let work_scheduler = Arc::new(WorkerScheduler::new(4));
    let work = gradle_substrate_daemon::server::work::WorkServiceImpl::new(work_scheduler.clone());
    let shared_history = Arc::new(ExecutionHistoryServiceImpl::new(history_dir.clone()));
    let execution_plan =
        gradle_substrate_daemon::server::execution_plan::ExecutionPlanServiceImpl::with_persistent_history(
            work_scheduler.clone(),
            Arc::clone(&shared_history),
        );
    let execution_plan_arc = Arc::new(execution_plan);
    let execution_plan_server =
        gradle_substrate_daemon::server::execution_plan::ExecutionPlanServiceImpl::with_persistent_history(
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

// ============================================================
// Reference topological sort (independent of the service)
// ============================================================

/// Reference Kahn's algorithm for topological sort.
/// Returns (execution_order, has_cycle).
fn reference_topological_sort(
    tasks: &[(String, Vec<String>)], // (task_path, depends_on)
) -> (Vec<String>, bool) {
    let task_set: HashSet<&str> = tasks.iter().map(|(t, _)| t.as_str()).collect();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for (task, deps) in tasks {
        in_degree.entry(task.as_str()).or_insert(0);
        for dep in deps {
            if task_set.contains(dep.as_str()) {
                *in_degree.entry(task.as_str()).or_insert(0) += 1;
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(task.as_str());
            }
        }
    }

    let mut queue: VecDeque<&str> = VecDeque::new();
    for (&task, &degree) in &in_degree {
        if degree == 0 {
            queue.push_back(task);
        }
    }

    let mut order = Vec::new();
    let mut visited = 0usize;

    while let Some(task) = queue.pop_front() {
        visited += 1;
        order.push(task.to_string());

        if let Some(deps) = dependents.get(task) {
            for &dep in deps {
                let degree = in_degree.get_mut(dep).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(dep);
                }
            }
        }
    }

    let has_cycle = visited != tasks.len();
    (order, has_cycle)
}

/// Verify that the service's execution order is a valid topological ordering
/// of the given tasks.
fn verify_topological_order(
    tasks: &[(String, Vec<String>)],
    execution_order: &[ExecutionNode],
) -> Vec<String> {
    let mut errors = Vec::new();

    // Build a map from task_path to its position in the execution order
    let order_map: HashMap<String, usize> = execution_order
        .iter()
        .enumerate()
        .map(|(i, node)| (node.task_path.clone(), i))
        .collect();

    // All executing tasks should be in the order
    let ordered_tasks: HashSet<&str> = execution_order
        .iter()
        .map(|n| n.task_path.as_str())
        .collect();
    for (task, _) in tasks {
        if !ordered_tasks.contains(task.as_str()) {
            errors.push(format!("Task '{}' not found in execution order", task));
        }
    }

    // Verify dependencies come before dependents
    for node in execution_order {
        for dep in &node.dependencies {
            let dep_pos = order_map.get(dep);
            let task_pos = order_map.get(&node.task_path);
            match (dep_pos, task_pos) {
                (Some(&d), Some(&t)) => {
                    if d >= t {
                        errors.push(format!(
                            "Dependency violation: '{}' (pos {}) should come before '{}' (pos {})",
                            dep, d, node.task_path, t
                        ));
                    }
                }
                (None, _) => {
                    // Dependency is not in the execution order (it might be should_execute=false)
                }
                _ => {}
            }
        }
    }

    errors
}

// ============================================================
// Test 1: Linear chain A -> B -> C -> D
// ============================================================

#[tokio::test]
async fn test_linear_chain_topological_order() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());

    let build_id = "linear-chain-test";

    // Register tasks: A -> B -> C -> D
    let tasks = vec![
        (":A", "TaskA", vec![]),
        (":B", "TaskB", vec![":A".to_string()]),
        (":C", "TaskC", vec![":B".to_string()]),
        (":D", "TaskD", vec![":C".to_string()]),
    ];

    for (path, task_type, deps) in &tasks {
        tg_client
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

    let plan = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(plan.total_tasks, 4);
    assert!(!plan.has_cycles, "Linear chain should not have cycles");
    assert_eq!(plan.execution_order.len(), 4);

    let task_defs: Vec<(String, Vec<String>)> = tasks
        .iter()
        .map(|(p, _, d)| (p.to_string(), d.clone()))
        .collect();

    let errors = verify_topological_order(&task_defs, &plan.execution_order);
    assert!(
        errors.is_empty(),
        "Topological order errors:\n{}",
        errors.join("\n")
    );

    // Verify the strict order A, B, C, D
    let order: Vec<&str> = plan
        .execution_order
        .iter()
        .map(|n| n.task_path.as_str())
        .collect();
    assert_eq!(
        order,
        vec![":A", ":B", ":C", ":D"],
        "Linear chain must execute in exact order"
    );
}

// ============================================================
// Test 2: Diamond dependency A -> B,C -> D
// ============================================================

#[tokio::test]
async fn test_diamond_dependency_topological_order() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());

    let build_id = "diamond-test";

    //     A
    //    / \
    //   B   C
    //    \ /
    //     D
    let tasks = vec![
        (":A", "TaskA", vec![]),
        (":B", "TaskB", vec![":A".to_string()]),
        (":C", "TaskC", vec![":A".to_string()]),
        (":D", "TaskD", vec![":B".to_string(), ":C".to_string()]),
    ];

    for (path, task_type, deps) in &tasks {
        tg_client
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

    let plan = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(plan.total_tasks, 4);
    assert!(!plan.has_cycles);
    assert_eq!(plan.execution_order.len(), 4);

    let task_defs: Vec<(String, Vec<String>)> = tasks
        .iter()
        .map(|(p, _, d)| (p.to_string(), d.clone()))
        .collect();

    let errors = verify_topological_order(&task_defs, &plan.execution_order);
    assert!(
        errors.is_empty(),
        "Topological order errors:\n{}",
        errors.join("\n")
    );

    // A must be first, D must be last
    let order: Vec<&str> = plan
        .execution_order
        .iter()
        .map(|n| n.task_path.as_str())
        .collect();
    assert_eq!(order[0], ":A", "A must execute first");
    assert_eq!(order[3], ":D", "D must execute last");
    // B and C can be in either order, but both before D
    assert_eq!(order[1..3].len(), 2);
    assert!(order[1..3].contains(&":B") && order[1..3].contains(&":C"));
}

// ============================================================
// Test 3: Large DAG with 15 tasks
// ============================================================

#[tokio::test]
async fn test_large_dag_topological_order() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());

    let build_id = "large-dag-test";

    // Build a realistic Gradle-like task graph with 15 tasks
    // :processResources -> :compileJava -> :classes -> :jar
    // :processResources -> :compileTestJava -> :testClasses -> :test -> :check -> :build
    // :classes -> :jar -> :build
    let tasks = vec![
        (":processResources", "ProcessResources", vec![]),
        (
            ":compileJava",
            "JavaCompile",
            vec![":processResources".to_string()],
        ),
        (":classes", "Lifecycle", vec![":compileJava".to_string()]),
        (":jar", "Jar", vec![":classes".to_string()]),
        (
            ":compileTestJava",
            "JavaCompile",
            vec![":processResources".to_string()],
        ),
        (
            ":testClasses",
            "Lifecycle",
            vec![":compileTestJava".to_string()],
        ),
        (":test", "Test", vec![":testClasses".to_string()]),
        (":check", "Lifecycle", vec![":test".to_string()]),
        (
            ":build",
            "Lifecycle",
            vec![":jar".to_string(), ":check".to_string()],
        ),
        (":clean", "Delete", vec![]),
        (":assemble", "Lifecycle", vec![":jar".to_string()]),
        (":spotlessCheck", "SpotlessCheck", vec![]),
        (":spotlessApply", "SpotlessApply", vec![]),
        (":lint", "Lint", vec![":compileJava".to_string()]),
        (
            ":verify",
            "Lifecycle",
            vec![":check".to_string(), ":lint".to_string()],
        ),
    ];

    for (path, task_type, deps) in &tasks {
        tg_client
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

    let plan = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(plan.total_tasks, 15, "Should have 15 tasks total");
    assert!(!plan.has_cycles, "Should not have cycles");
    assert_eq!(
        plan.execution_order.len(),
        15,
        "All 15 tasks should be in execution order"
    );

    let task_defs: Vec<(String, Vec<String>)> = tasks
        .iter()
        .map(|(p, _, d)| (p.to_string(), d.clone()))
        .collect();

    let errors = verify_topological_order(&task_defs, &plan.execution_order);
    assert!(
        errors.is_empty(),
        "Topological order errors:\n{}",
        errors.join("\n")
    );

    // Verify specific ordering constraints
    let pos: HashMap<&str, usize> = plan
        .execution_order
        .iter()
        .enumerate()
        .map(|(i, n)| (n.task_path.as_str(), i))
        .collect();

    // processResources must come before compileJava and compileTestJava
    assert!(pos[":processResources"] < pos[":compileJava"]);
    assert!(pos[":processResources"] < pos[":compileTestJava"]);
    // compileJava must come before classes and lint
    assert!(pos[":compileJava"] < pos[":classes"]);
    assert!(pos[":compileJava"] < pos[":lint"]);
    // classes must come before jar
    assert!(pos[":classes"] < pos[":jar"]);
    // jar must come before build and assemble
    assert!(pos[":jar"] < pos[":build"]);
    assert!(pos[":jar"] < pos[":assemble"]);
    // test must come before check
    assert!(pos[":test"] < pos[":check"]);
    // check must come before build and verify
    assert!(pos[":check"] < pos[":build"]);
    assert!(pos[":check"] < pos[":verify"]);
    // lint must come before verify
    assert!(pos[":lint"] < pos[":verify"]);
    // verify must come before build (since build depends on check which depends on verify path)
    // Actually, build depends on jar + check, and verify depends on check + lint
    // So verify is NOT necessarily before build -- that's fine
}

// ============================================================
// Test 4: Cycle detection A -> B -> C -> A
// ============================================================

#[tokio::test]
async fn test_cycle_detection() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());

    let build_id = "cycle-test";

    // Create a cycle: A -> B -> C -> A
    let tasks = vec![
        (":A", "TaskA", vec![":C".to_string()]),
        (":B", "TaskB", vec![":A".to_string()]),
        (":C", "TaskC", vec![":B".to_string()]),
    ];

    for (path, task_type, deps) in &tasks {
        tg_client
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

    let plan = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(plan.has_cycles, "Cycle should be detected");
    // With a cycle, the execution order should be incomplete
    assert!(
        plan.execution_order.len() < 3,
        "Cyclic graph should have incomplete execution order (got {} nodes)",
        plan.execution_order.len()
    );

    // Also verify with our reference implementation
    let task_defs: Vec<(String, Vec<String>)> = tasks
        .iter()
        .map(|(p, _, d)| (p.to_string(), d.clone()))
        .collect();
    let (_, ref_has_cycle) = reference_topological_sort(&task_defs);
    assert!(
        ref_has_cycle,
        "Reference implementation should also detect the cycle"
    );
}

// ============================================================
// Test 5: Self-cycle detection A -> A
// ============================================================

#[tokio::test]
async fn test_self_cycle_detection() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());

    let build_id = "self-cycle-test";

    tg_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: build_id.to_string(),
            task_path: ":A".to_string(),
            depends_on: vec![":A".to_string()],
            task_type: "TaskA".to_string(),
            input_files: vec![],
            should_execute: true,
        }))
        .await
        .unwrap();

    let plan = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(plan.has_cycles, "Self-cycle should be detected");
    assert!(
        plan.execution_order.is_empty(),
        "Self-cyclic task should have no execution order"
    );
}

// ============================================================
// Test 6: DAG execution via DagExecutorService
// ============================================================

#[tokio::test]
async fn test_dag_executor_full_build_completion() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());
    let mut dag_client =
        dag_executor_service_client::DagExecutorServiceClient::new(channel.clone());

    let build_id = "dag-exec-complete-test";

    // Register a simple diamond: A -> B,C -> D
    let tasks = vec![
        (":A", "TaskA", vec![]),
        (":B", "TaskB", vec![":A".to_string()]),
        (":C", "TaskC", vec![":A".to_string()]),
        (":D", "TaskD", vec![":B".to_string(), ":C".to_string()]),
    ];

    for (path, task_type, deps) in &tasks {
        tg_client
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

    // Start the build
    let start_resp = dag_client
        .start_build(Request::new(StartBuildRequest {
            build_id: build_id.to_string(),
            max_parallelism: 2,
            task_filter: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(start_resp.accepted, "Build should be accepted");
    assert_eq!(start_resp.total_tasks, 4, "Should have 4 total tasks");

    // Execute all tasks via get_next_task -> notify_started -> notify_finished loop
    let mut executed_tasks: Vec<String> = Vec::new();
    let mut completed_set: HashSet<String> = HashSet::new();

    loop {
        let next = dag_client
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: build_id.to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        // Check for build complete sentinel
        if next.task_path == "__BUILD_COMPLETE__" {
            break;
        }
        if next.task_path.is_empty() {
            break;
        }

        let task_path = next.task_path.clone();
        executed_tasks.push(task_path.clone());

        // Verify dependency: all deps of this task should be in completed_set
        let task_def = tasks.iter().find(|(p, _, _)| *p == task_path.as_str());
        if let Some((_, _, deps)) = task_def {
            for dep in deps {
                assert!(
                    completed_set.contains(dep),
                    "Task '{}' was scheduled before its dependency '{}' was completed",
                    task_path,
                    dep
                );
            }
        }

        // Notify started
        dag_client
            .notify_task_started(Request::new(NotifyTaskStartedRequest {
                build_id: build_id.to_string(),
                task_path: task_path.clone(),
                start_time_ms: 0,
            }))
            .await
            .unwrap();

        // Notify finished
        let finish_resp = dag_client
            .notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                build_id: build_id.to_string(),
                task_path: task_path.clone(),
                success: true,
                outcome: "SUCCEEDED".to_string(),
                duration_ms: 100,
                failure_message: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(finish_resp.acknowledged);
        completed_set.insert(task_path);
    }

    assert_eq!(executed_tasks.len(), 4, "All 4 tasks should be executed");

    // Verify the order is topologically valid
    let task_defs: Vec<(String, Vec<String>)> = tasks
        .iter()
        .map(|(p, _, d)| (p.to_string(), d.clone()))
        .collect();

    let errors = verify_topological_order(
        &task_defs,
        &executed_tasks
            .iter()
            .enumerate()
            .map(|(i, t)| ExecutionNode {
                task_path: t.clone(),
                dependencies: task_defs
                    .iter()
                    .find(|(p, _)| p == t)
                    .map(|(_, d)| d.clone())
                    .unwrap_or_default(),
                execution_order: (i + 1) as i64,
                estimated_duration_ms: 0,
                task_type: String::new(),
            })
            .collect::<Vec<_>>(),
    );
    assert!(
        errors.is_empty(),
        "Execution order errors:\n{}",
        errors.join("\n")
    );

    // Verify build status is complete
    let status = dag_client
        .get_build_status(Request::new(GetBuildStatusRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(status.completed_tasks, 4);
    assert_eq!(status.failed_tasks, 0);
    assert_eq!(status.pending_tasks, 0);
    assert_eq!(status.executing_tasks, 0);
}

// ============================================================
// Test 7: DAG executor with task failure
// ============================================================

#[tokio::test]
async fn test_dag_executor_task_failure() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());
    let mut dag_client =
        dag_executor_service_client::DagExecutorServiceClient::new(channel.clone());

    let build_id = "dag-exec-failure-test";

    // A -> B -> C
    let tasks = vec![
        (":A", "TaskA", vec![]),
        (":B", "TaskB", vec![":A".to_string()]),
        (":C", "TaskC", vec![":B".to_string()]),
    ];

    for (path, task_type, deps) in &tasks {
        tg_client
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

    dag_client
        .start_build(Request::new(StartBuildRequest {
            build_id: build_id.to_string(),
            max_parallelism: 1,
            task_filter: vec![],
        }))
        .await
        .unwrap();

    // Execute A (success)
    let next = dag_client
        .get_next_task(Request::new(GetNextTaskRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(next.task_path, ":A");

    dag_client
        .notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: build_id.to_string(),
            task_path: ":A".to_string(),
            start_time_ms: 0,
        }))
        .await
        .unwrap();

    dag_client
        .notify_task_finished(Request::new(NotifyTaskFinishedRequest {
            build_id: build_id.to_string(),
            task_path: ":A".to_string(),
            success: true,
            outcome: "SUCCEEDED".to_string(),
            duration_ms: 100,
            failure_message: String::new(),
        }))
        .await
        .unwrap();

    // Execute B (failure)
    let next = dag_client
        .get_next_task(Request::new(GetNextTaskRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(next.task_path, ":B");

    dag_client
        .notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: build_id.to_string(),
            task_path: ":B".to_string(),
            start_time_ms: 0,
        }))
        .await
        .unwrap();

    dag_client
        .notify_task_finished(Request::new(NotifyTaskFinishedRequest {
            build_id: build_id.to_string(),
            task_path: ":B".to_string(),
            success: false,
            outcome: "FAILED".to_string(),
            duration_ms: 200,
            failure_message: "Simulated failure".to_string(),
        }))
        .await
        .unwrap();

    // Build should complete (possibly without C executing)
    let next = dag_client
        .get_next_task(Request::new(GetNextTaskRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // The build should either give us C or the sentinel
    if next.task_path == ":C" {
        // If C is given, finish it
        dag_client
            .notify_task_started(Request::new(NotifyTaskStartedRequest {
                build_id: build_id.to_string(),
                task_path: ":C".to_string(),
                start_time_ms: 0,
            }))
            .await
            .unwrap();
        dag_client
            .notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                build_id: build_id.to_string(),
                task_path: ":C".to_string(),
                success: true,
                outcome: "SUCCEEDED".to_string(),
                duration_ms: 50,
                failure_message: String::new(),
            }))
            .await
            .unwrap();
    }

    // Get final status
    let status = dag_client
        .get_build_status(Request::new(GetBuildStatusRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(
        status.failed_tasks >= 1,
        "Should have at least 1 failed task, got {}",
        status.failed_tasks
    );
}

// ============================================================
// Test 8: Build isolation - different build IDs don't interfere
// ============================================================

#[tokio::test]
async fn test_build_isolation_separate_build_ids() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel);

    // Build 1: A -> B
    let build_id_1 = "isolation-build-1";
    tg_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: build_id_1.to_string(),
            task_path: ":A1".to_string(),
            depends_on: vec![],
            task_type: "TaskA".to_string(),
            input_files: vec![],
            should_execute: true,
        }))
        .await
        .unwrap();
    tg_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: build_id_1.to_string(),
            task_path: ":B1".to_string(),
            depends_on: vec![":A1".to_string()],
            task_type: "TaskB".to_string(),
            input_files: vec![],
            should_execute: true,
        }))
        .await
        .unwrap();

    // Build 2: X -> Y -> Z
    let build_id_2 = "isolation-build-2";
    tg_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: build_id_2.to_string(),
            task_path: ":X2".to_string(),
            depends_on: vec![],
            task_type: "TaskX".to_string(),
            input_files: vec![],
            should_execute: true,
        }))
        .await
        .unwrap();
    tg_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: build_id_2.to_string(),
            task_path: ":Y2".to_string(),
            depends_on: vec![":X2".to_string()],
            task_type: "TaskY".to_string(),
            input_files: vec![],
            should_execute: true,
        }))
        .await
        .unwrap();
    tg_client
        .register_task(Request::new(RegisterTaskRequest {
            build_id: build_id_2.to_string(),
            task_path: ":Z2".to_string(),
            depends_on: vec![":Y2".to_string()],
            task_type: "TaskZ".to_string(),
            input_files: vec![],
            should_execute: true,
        }))
        .await
        .unwrap();

    // Resolve both plans independently
    let plan1 = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id_1.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    let plan2 = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id_2.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    // Build 1 should have 2 tasks
    assert_eq!(plan1.total_tasks, 2);
    assert!(!plan1.has_cycles);
    assert_eq!(plan1.execution_order.len(), 2);
    let paths1: Vec<&str> = plan1
        .execution_order
        .iter()
        .map(|n| n.task_path.as_str())
        .collect();
    assert_eq!(paths1, vec![":A1", ":B1"]);

    // Build 2 should have 3 tasks
    assert_eq!(plan2.total_tasks, 3);
    assert!(!plan2.has_cycles);
    assert_eq!(plan2.execution_order.len(), 3);
    let paths2: Vec<&str> = plan2
        .execution_order
        .iter()
        .map(|n| n.task_path.as_str())
        .collect();
    assert_eq!(paths2, vec![":X2", ":Y2", ":Z2"]);
}

// ============================================================
// Test 9: Multiple independent roots (parallel start)
// ============================================================

#[tokio::test]
async fn test_multiple_independent_roots() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel);

    let build_id = "multi-root-test";

    // Three independent chains that can execute in parallel:
    // A -> B, C -> D, E
    let tasks = vec![
        (":A", "TaskA", vec![]),
        (":B", "TaskB", vec![":A".to_string()]),
        (":C", "TaskC", vec![]),
        (":D", "TaskD", vec![":C".to_string()]),
        (":E", "TaskE", vec![]),
    ];

    for (path, task_type, deps) in &tasks {
        tg_client
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

    let plan = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(plan.total_tasks, 5);
    assert!(!plan.has_cycles);
    assert_eq!(plan.execution_order.len(), 5);

    // A, C, E should all be in the first ready-to-execute batch
    assert_eq!(
        plan.ready_to_execute, 3,
        "Three independent roots should be ready"
    );

    let task_defs: Vec<(String, Vec<String>)> = tasks
        .iter()
        .map(|(p, _, d)| (p.to_string(), d.clone()))
        .collect();

    let errors = verify_topological_order(&task_defs, &plan.execution_order);
    assert!(
        errors.is_empty(),
        "Topological order errors:\n{}",
        errors.join("\n")
    );

    // Verify constraints
    let pos: HashMap<&str, usize> = plan
        .execution_order
        .iter()
        .enumerate()
        .map(|(i, n)| (n.task_path.as_str(), i))
        .collect();

    assert!(pos[":A"] < pos[":B"]);
    assert!(pos[":C"] < pos[":D"]);
}

// ============================================================
// Test 10: Empty graph (no tasks registered)
// ============================================================

#[tokio::test]
async fn test_empty_graph_no_tasks() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel);

    let build_id = "empty-graph-test";

    let plan = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(plan.total_tasks, 0);
    assert!(!plan.has_cycles);
    assert_eq!(plan.execution_order.len(), 0);
    assert_eq!(plan.ready_to_execute, 0);
}

// ============================================================
// Test 11: Verify against independent reference topological sort
// ============================================================

#[tokio::test]
async fn test_topological_sort_matches_reference_implementation() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel);

    let build_id = "ref-sort-test";

    // Build a complex graph with 10 tasks and various dependencies
    let tasks = vec![
        (":root1", "Root", vec![]),
        (":root2", "Root", vec![]),
        (":mid1", "Mid", vec![":root1".to_string()]),
        (
            ":mid2",
            "Mid",
            vec![":root1".to_string(), ":root2".to_string()],
        ),
        (":mid3", "Mid", vec![":root2".to_string()]),
        (
            ":leaf1",
            "Leaf",
            vec![":mid1".to_string(), ":mid2".to_string()],
        ),
        (
            ":leaf2",
            "Leaf",
            vec![":mid2".to_string(), ":mid3".to_string()],
        ),
        (":leaf3", "Leaf", vec![":mid3".to_string()]),
        (
            ":final1",
            "Final",
            vec![":leaf1".to_string(), ":leaf2".to_string()],
        ),
        (
            ":final2",
            "Final",
            vec![":leaf2".to_string(), ":leaf3".to_string()],
        ),
    ];

    for (path, task_type, deps) in &tasks {
        tg_client
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

    let plan = tg_client
        .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!plan.has_cycles);

    // Run our independent reference topological sort
    let task_defs: Vec<(String, Vec<String>)> = tasks
        .iter()
        .map(|(p, _, d)| (p.to_string(), d.clone()))
        .collect();
    let (ref_order, ref_cycle) = reference_topological_sort(&task_defs);

    assert!(!ref_cycle, "Reference should not find cycles");
    assert_eq!(ref_order.len(), 10, "Reference should have all 10 tasks");

    // Both should produce valid topological orderings
    // (they may not be identical since topological sort can have multiple valid orderings)
    let ref_set: HashSet<String> = ref_order.into_iter().collect();
    let service_set: HashSet<String> = plan
        .execution_order
        .iter()
        .map(|n| n.task_path.clone())
        .collect();

    assert_eq!(
        ref_set, service_set,
        "Service and reference should produce the same set of tasks"
    );

    // Verify service ordering is valid
    let errors = verify_topological_order(&task_defs, &plan.execution_order);
    assert!(
        errors.is_empty(),
        "Topological order errors:\n{}",
        errors.join("\n")
    );
}

// ============================================================
// Test 12: Progress tracking through build
// ============================================================

#[tokio::test]
async fn test_progress_tracking() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut tg_client = task_graph_service_client::TaskGraphServiceClient::new(channel.clone());

    let build_id = "progress-test";

    let tasks = vec![
        (":A", "TaskA", vec![]),
        (":B", "TaskB", vec![":A".to_string()]),
        (":C", "TaskC", vec![":B".to_string()]),
    ];

    for (path, task_type, deps) in &tasks {
        tg_client
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

    // Check initial progress
    let progress = tg_client
        .get_progress(Request::new(GetProgressRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(progress.total, 3);
    assert_eq!(progress.completed, 0);
    assert_eq!(progress.executing, 0);

    // Mark A as started
    tg_client
        .task_started(Request::new(TaskStartedRequest {
            build_id: build_id.to_string(),
            task_path: ":A".to_string(),
            start_time_ms: 0,
        }))
        .await
        .unwrap();

    let progress = tg_client
        .get_progress(Request::new(GetProgressRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(progress.total, 3);
    assert_eq!(progress.completed, 0);
    assert_eq!(progress.executing, 1);

    // Mark A as finished
    tg_client
        .task_finished(Request::new(TaskFinishedRequest {
            build_id: build_id.to_string(),
            task_path: ":A".to_string(),
            duration_ms: 100,
            success: true,
            outcome: "SUCCEEDED".to_string(),
        }))
        .await
        .unwrap();

    let progress = tg_client
        .get_progress(Request::new(GetProgressRequest {
            build_id: build_id.to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(progress.total, 3);
    assert_eq!(progress.completed, 1);
    assert_eq!(progress.executing, 0);
}
