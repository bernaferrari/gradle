use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::net::UnixListener;
use tokio::signal;
use tonic::transport::Server;

use gradle_substrate_daemon::{
    proto::{
        artifact_publishing_service_server::ArtifactPublishingServiceServer,
        bootstrap_service_server::BootstrapServiceServer,
        build_cache_orchestration_service_server::BuildCacheOrchestrationServiceServer,
        build_comparison_service_server::BuildComparisonServiceServer,
        build_event_stream_service_server::BuildEventStreamServiceServer,
        build_init_service_server::BuildInitServiceServer,
        build_layout_service_server::BuildLayoutServiceServer,
        build_metrics_service_server::BuildMetricsServiceServer,
        build_operations_service_server::BuildOperationsServiceServer,
        build_result_service_server::BuildResultServiceServer,
        cache_service_server::CacheServiceServer,
        configuration_cache_service_server::ConfigurationCacheServiceServer,
        configuration_service_server::ConfigurationServiceServer,
        console_service_server::ConsoleServiceServer,
        control_service_server::ControlServiceServer,
        dag_executor_service_server::DagExecutorServiceServer,
        dependency_resolution_service_server::DependencyResolutionServiceServer,
        execution_history_service_server::ExecutionHistoryServiceServer,
        execution_plan_service_server::ExecutionPlanServiceServer,
        exec_service_server::ExecServiceServer,
        file_fingerprint_service_server::FileFingerprintServiceServer,
        file_watch_service_server::FileWatchServiceServer,
        garbage_collection_service_server::GarbageCollectionServiceServer,
        hash_service_server::HashServiceServer,
        incremental_compilation_service_server::IncrementalCompilationServiceServer,
        plugin_service_server::PluginServiceServer,
        problem_reporting_service_server::ProblemReportingServiceServer,
        resource_management_service_server::ResourceManagementServiceServer,
        task_graph_service_server::TaskGraphServiceServer,
        test_execution_service_server::TestExecutionServiceServer,
        toolchain_service_server::ToolchainServiceServer,
        value_snapshot_service_server::ValueSnapshotServiceServer,
        worker_process_service_server::WorkerProcessServiceServer,
        work_service_server::WorkServiceServer,
    },
    server::{
        authoritative::AuthoritativeConfig,
        dag_executor::DagExecutorServiceImpl,
        artifact_publishing::ArtifactPublishingServiceImpl,
        bootstrap::BootstrapServiceImpl, build_comparison::BuildComparisonServiceImpl,
        build_event_stream::BuildEventStreamServiceImpl, build_init::BuildInitServiceImpl,
        build_layout::BuildLayoutServiceImpl, build_metrics::BuildMetricsServiceImpl,
        build_operations::BuildOperationsServiceImpl, build_result::BuildResultServiceImpl,
        cache::CacheServiceImpl, cache_orchestration::BuildCacheOrchestrationServiceImpl,
        config_cache::ConfigurationCacheServiceImpl, configuration::ConfigurationServiceImpl,
        console::ConsoleServiceImpl, control::ControlServiceImpl,
        dependency_resolution::DependencyResolutionServiceImpl,
        execution_history::ExecutionHistoryServiceImpl, execution_plan::ExecutionPlanServiceImpl,
        exec::ExecServiceImpl, file_fingerprint::FileFingerprintServiceImpl,
        file_watch::FileWatchServiceImpl, garbage_collection::GarbageCollectionServiceImpl,
        hash::HashServiceImpl, incremental_compilation::IncrementalCompilationServiceImpl,
        plugin::PluginServiceImpl, problem_reporting::ProblemReportingServiceImpl,
        resource_management::ResourceManagementServiceImpl, scopes::ScopeRegistry,
        task_graph::TaskGraphServiceImpl, test_execution::TestExecutionServiceImpl,
        toolchain::ToolchainServiceImpl, value_snapshot::ValueSnapshotServiceImpl,
        worker_process::WorkerProcessServiceImpl, work::WorkServiceImpl,
    },
    PROTOCOL_VERSION,
};

/// Gradle Rust Substrate Daemon
///
/// A sidecar process that provides high-performance execution
/// substrate services (hashing, caching, process execution) to
/// the Gradle JVM via gRPC over Unix domain sockets.
#[derive(Parser, Debug)]
#[command(name = "gradle-substrate-daemon")]
#[command(version, about)]
struct Args {
    /// Path to the Unix domain socket to listen on
    #[arg(long, default_value = "/tmp/gradle-substrate.sock")]
    socket_path: String,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", env = "SUBSTRATE_LOG_LEVEL")]
    log_level: String,

    /// Path to the build cache directory
    #[arg(long, default_value = "/tmp/gradle-substrate-cache")]
    cache_dir: String,

    /// Path to the execution history directory
    #[arg(long, default_value = "/tmp/gradle-substrate-history")]
    history_dir: String,

    /// Path to the configuration cache directory
    #[arg(long, default_value = "/tmp/gradle-substrate-config-cache")]
    config_cache_dir: String,

    /// Path to the toolchain installation directory
    #[arg(long, default_value = "/tmp/gradle-substrate-toolchains")]
    toolchain_dir: String,

    /// Remote build cache URL (e.g., https://cache.example.com/build-cache)
    #[arg(long, env = "SUBSTRATE_REMOTE_CACHE_URL")]
    remote_cache_url: Option<String>,

    /// Remote cache username for HTTP Basic auth
    #[arg(long, env = "SUBSTRATE_REMOTE_CACHE_USERNAME")]
    remote_cache_username: Option<String>,

    /// Remote cache password for HTTP Basic auth
    #[arg(long, env = "SUBSTRATE_REMOTE_CACHE_PASSWORD")]
    remote_cache_password: Option<String>,
}

fn init_logging(level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Received shutdown signal");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    init_logging(&args.log_level);

    let socket_path = PathBuf::from(&args.socket_path);

    // Clean up stale socket if present
    if socket_path.exists() {
        tracing::warn!(path = %args.socket_path, "Removing stale socket file");
        std::fs::remove_file(&socket_path)?;
    }

    // Create parent directory if needed
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    // Authoritative mode configuration (shared across services)
    let authoritative_config = Arc::new(AuthoritativeConfig::new());

    // Phase 0: Control
    let control = ControlServiceImpl::with_config(shutdown_tx.clone(), authoritative_config);

    // Phase 1: Hashing
    let hash = HashServiceImpl;

    // Phase 2: Build cache
    let cache_dir = PathBuf::from(&args.cache_dir);
    std::fs::create_dir_all(&cache_dir)?;
    let gc_cache_dir = cache_dir.clone();

    let cache = if let Some(remote_url) = &args.remote_cache_url {
        let remote = gradle_substrate_daemon::server::remote_cache::RemoteCacheStore::new(
            remote_url.clone(),
            args.remote_cache_username.clone(),
            args.remote_cache_password.clone(),
        );
        tracing::info!(remote_url = %remote_url, "Remote cache configured");
        CacheServiceImpl::with_remote(cache_dir, remote)
    } else {
        CacheServiceImpl::new(cache_dir)
    };

    // Phase 3: Process execution
    let exec = ExecServiceImpl::new();

    // Phase 4: Work scheduling
    let work_scheduler = Arc::new(gradle_substrate_daemon::server::work::WorkerScheduler::new(num_cpus::get()));
    let work = WorkServiceImpl::new(work_scheduler.clone());

    // Phase 7: Execution history (created early so execution plan + task graph can reference it)
    let history_dir = PathBuf::from(&args.history_dir);
    let gc_history_dir = history_dir.clone();
    let execution_history = Arc::new(ExecutionHistoryServiceImpl::new(history_dir.clone()));
    let history_count = execution_history.load_from_disk().await?;
    tracing::info!("Loaded {} execution history entries", history_count);

    // Phase 5-6: Execution planning (wired to persistent history for rebuild loop detection)
    let execution_plan = ExecutionPlanServiceImpl::with_persistent_history(work_scheduler.clone(), execution_history.clone());
    execution_plan.load_persistent_history();

    // Phase 8: Build cache orchestration (wired to local cache for real probe operations)
    let cache_orchestration = BuildCacheOrchestrationServiceImpl::with_local_cache(cache.local_store());

    // Phase 9: File fingerprinting
    let file_fingerprint = FileFingerprintServiceImpl::new();

    // Phase 10: Value snapshotting
    let value_snapshot = ValueSnapshotServiceImpl::new();

    // Phase 11: Task graph (wired to execution history for duration estimates)
    let task_graph = Arc::new(TaskGraphServiceImpl::with_history(execution_history.clone()));

    // Phase 12: Configuration
    let configuration = ConfigurationServiceImpl::new();

    // Phase 13: Plugin management
    let plugin = PluginServiceImpl::new();

    // Phase 14: Build operations
    let build_operations = BuildOperationsServiceImpl::new();

    // Scope registry — tracks session→build membership for proper scope isolation
    let scope_registry = Arc::new(ScopeRegistry::new());

    // Phase 15: Bootstrap (wired to scope registry for session tracking)
    let bootstrap = BootstrapServiceImpl::with_scope_registry(scope_registry.clone());

    // Phase 18: Dependency resolution
    let dependency_resolution = DependencyResolutionServiceImpl::new();

    // Phase 19: File watching (wired to task graph for file-change -> task invalidation)
    let file_watch = FileWatchServiceImpl::with_task_graph(Arc::clone(&task_graph));

    // Phase 20: Configuration cache
    let config_cache_dir = PathBuf::from(&args.config_cache_dir);
    let gc_config_cache_dir = config_cache_dir.clone();
    let config_cache = ConfigurationCacheServiceImpl::new(config_cache_dir);

    // Phase 23: Toolchain management
    let toolchain_dir = PathBuf::from(&args.toolchain_dir);
    let toolchain = ToolchainServiceImpl::new(toolchain_dir);

    // Phase 24: Build event streaming (wired to console + metrics for auto fan-out)
    let console = Arc::new(ConsoleServiceImpl::new());
    let build_metrics = Arc::new(BuildMetricsServiceImpl::new());
    let event_dispatchers: Vec<Arc<dyn gradle_substrate_daemon::server::event_dispatcher::EventDispatcher>> = vec![
        Arc::clone(&console) as Arc<dyn gradle_substrate_daemon::server::event_dispatcher::EventDispatcher>,
        Arc::clone(&build_metrics) as Arc<dyn gradle_substrate_daemon::server::event_dispatcher::EventDispatcher>,
    ];
    let build_event_stream = BuildEventStreamServiceImpl::with_dispatchers(event_dispatchers.clone());

    // DAG Executor (orchestrates build execution using task graph + worker scheduler)
    let dag_executor = DagExecutorServiceImpl::new(work_scheduler.clone(), Arc::clone(&task_graph), event_dispatchers);

    // Phase 25: Worker process management
    let worker_process = WorkerProcessServiceImpl::new();

    // Phase 26: Build layout / project model
    let build_layout = BuildLayoutServiceImpl::new();

    // Phase 28: Build result reporting
    let build_result = BuildResultServiceImpl::new();

    // Phase 29: Problem / diagnostic reporting
    let problem_reporting = ProblemReportingServiceImpl::new();

    // Phase 30: Resource management
    let resource_management = ResourceManagementServiceImpl::new();

    // Phase 31: Build comparison
    let build_comparison = BuildComparisonServiceImpl::new();

    // Phase 33: Test execution
    let test_execution = TestExecutionServiceImpl::new();

    // Phase 34: Artifact publishing
    let artifact_publishing = ArtifactPublishingServiceImpl::new();

    // Phase 35: Build initialization (wired to scope registry)
    let build_init = BuildInitServiceImpl::with_scope_registry(scope_registry.clone());

    // Phase 36: Incremental compilation
    let incremental_compilation = IncrementalCompilationServiceImpl::new();

    // Phase 38: Garbage collection
    let garbage_collection = GarbageCollectionServiceImpl::new(
        gc_cache_dir,
        gc_history_dir,
        gc_config_cache_dir,
    );

    let listener = UnixListener::bind(&socket_path)?;

    println!("Gradle Substrate Daemon v{}", env!("CARGO_PKG_VERSION"));
    println!("Protocol version: {}", PROTOCOL_VERSION);
    println!("Listening on: {}", args.socket_path);
    println!("Services: control, dag-executor, hash, cache, exec, work, execution-plan, execution-history, cache-orchestration, file-fingerprint, value-snapshot, task-graph, configuration, plugin, build-operations, bootstrap, dependency-resolution, file-watch, config-cache, toolchain, build-event-stream, worker-process, build-layout, build-result, problem-reporting, resource-management, build-comparison, console, test-execution, artifact-publishing, build-init, incremental-compilation, build-metrics, garbage-collection");

    Server::builder()
        .add_service(ControlServiceServer::new(control))
        .add_service(DagExecutorServiceServer::new(dag_executor))
        .add_service(HashServiceServer::new(hash))
        .add_service(CacheServiceServer::new(cache))
        .add_service(ExecServiceServer::new(exec))
        .add_service(WorkServiceServer::new(work))
        .add_service(ExecutionPlanServiceServer::new(execution_plan))
        .add_service(ExecutionHistoryServiceServer::new((*execution_history).clone()))
        .add_service(BuildCacheOrchestrationServiceServer::new(cache_orchestration))
        .add_service(FileFingerprintServiceServer::new(file_fingerprint))
        .add_service(ValueSnapshotServiceServer::new(value_snapshot))
        .add_service(TaskGraphServiceServer::new((*task_graph).clone()))
        .add_service(ConfigurationServiceServer::new(configuration))
        .add_service(PluginServiceServer::new(plugin))
        .add_service(BuildOperationsServiceServer::new(build_operations))
        .add_service(BootstrapServiceServer::new(bootstrap))
        .add_service(DependencyResolutionServiceServer::new(dependency_resolution))
        .add_service(FileWatchServiceServer::new(file_watch))
        .add_service(ConfigurationCacheServiceServer::new(config_cache))
        .add_service(ToolchainServiceServer::new(toolchain))
        .add_service(BuildEventStreamServiceServer::new(build_event_stream))
        .add_service(WorkerProcessServiceServer::new(worker_process))
        .add_service(BuildLayoutServiceServer::new(build_layout))
        .add_service(BuildResultServiceServer::new(build_result))
        .add_service(ProblemReportingServiceServer::new(problem_reporting))
        .add_service(ResourceManagementServiceServer::new(resource_management))
        .add_service(BuildComparisonServiceServer::new(build_comparison))
        .add_service(ConsoleServiceServer::new((*console).clone()))
        .add_service(TestExecutionServiceServer::new(test_execution))
        .add_service(ArtifactPublishingServiceServer::new(artifact_publishing))
        .add_service(BuildInitServiceServer::new(build_init))
        .add_service(IncrementalCompilationServiceServer::new(incremental_compilation))
        .add_service(BuildMetricsServiceServer::new((*build_metrics).clone()))
        .add_service(GarbageCollectionServiceServer::new(garbage_collection))
        .serve_with_incoming_shutdown(tokio_stream::wrappers::UnixListenerStream::new(listener), shutdown_signal())
        .await?;

    tracing::info!("Daemon shut down cleanly");

    // Clean up socket file
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    Ok(())
}
