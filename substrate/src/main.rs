use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::net::UnixListener;
use tokio::signal;
use tonic::transport::Server;

use gradle_substrate_daemon::{
    proto::{
        bootstrap_service_server::BootstrapServiceServer,
        build_cache_orchestration_service_server::BuildCacheOrchestrationServiceServer,
        build_event_stream_service_server::BuildEventStreamServiceServer,
        build_layout_service_server::BuildLayoutServiceServer,
        build_operations_service_server::BuildOperationsServiceServer,
        cache_service_server::CacheServiceServer,
        configuration_cache_service_server::ConfigurationCacheServiceServer,
        configuration_service_server::ConfigurationServiceServer,
        control_service_server::ControlServiceServer,
        dependency_resolution_service_server::DependencyResolutionServiceServer,
        execution_history_service_server::ExecutionHistoryServiceServer,
        execution_plan_service_server::ExecutionPlanServiceServer,
        exec_service_server::ExecServiceServer,
        file_fingerprint_service_server::FileFingerprintServiceServer,
        file_watch_service_server::FileWatchServiceServer,
        hash_service_server::HashServiceServer,
        plugin_service_server::PluginServiceServer,
        task_graph_service_server::TaskGraphServiceServer,
        toolchain_service_server::ToolchainServiceServer,
        value_snapshot_service_server::ValueSnapshotServiceServer,
        worker_process_service_server::WorkerProcessServiceServer,
        work_service_server::WorkServiceServer,
    },
    server::{
        bootstrap::BootstrapServiceImpl, build_event_stream::BuildEventStreamServiceImpl,
        build_layout::BuildLayoutServiceImpl, build_operations::BuildOperationsServiceImpl,
        cache::CacheServiceImpl, cache_orchestration::BuildCacheOrchestrationServiceImpl,
        config_cache::ConfigurationCacheServiceImpl, configuration::ConfigurationServiceImpl,
        control::ControlServiceImpl, dependency_resolution::DependencyResolutionServiceImpl,
        execution_history::ExecutionHistoryServiceImpl, execution_plan::ExecutionPlanServiceImpl,
        exec::ExecServiceImpl, file_fingerprint::FileFingerprintServiceImpl,
        file_watch::FileWatchServiceImpl, hash::HashServiceImpl, plugin::PluginServiceImpl,
        task_graph::TaskGraphServiceImpl, toolchain::ToolchainServiceImpl,
        value_snapshot::ValueSnapshotServiceImpl, worker_process::WorkerProcessServiceImpl,
        work::WorkServiceImpl,
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

    // Phase 0: Control
    let control = ControlServiceImpl::new(shutdown_tx.clone());

    // Phase 1: Hashing
    let hash = HashServiceImpl;

    // Phase 2: Build cache
    let cache_dir = PathBuf::from(&args.cache_dir);
    std::fs::create_dir_all(&cache_dir)?;
    let cache = CacheServiceImpl::new(cache_dir);

    // Phase 3: Process execution
    let exec = ExecServiceImpl::new();

    // Phase 4: Work scheduling
    let work_scheduler = Arc::new(gradle_substrate_daemon::server::work::WorkerScheduler::new(num_cpus::get()));
    let work = WorkServiceImpl::new(work_scheduler.clone());

    // Phase 5-6: Execution planning
    let execution_plan = ExecutionPlanServiceImpl::new(work_scheduler);

    // Phase 7: Execution history
    let history_dir = PathBuf::from(&args.history_dir);
    let execution_history = ExecutionHistoryServiceImpl::new(history_dir);
    let history_count = execution_history.load_from_disk().await?;
    tracing::info!("Loaded {} execution history entries", history_count);

    // Phase 8: Build cache orchestration
    let cache_orchestration = BuildCacheOrchestrationServiceImpl::new();

    // Phase 9: File fingerprinting
    let file_fingerprint = FileFingerprintServiceImpl::new();

    // Phase 10: Value snapshotting
    let value_snapshot = ValueSnapshotServiceImpl::new();

    // Phase 11: Task graph
    let task_graph = TaskGraphServiceImpl::new();

    // Phase 12: Configuration
    let configuration = ConfigurationServiceImpl::new();

    // Phase 13: Plugin management
    let plugin = PluginServiceImpl::new();

    // Phase 14: Build operations
    let build_operations = BuildOperationsServiceImpl::new();

    // Phase 15: Bootstrap
    let bootstrap = BootstrapServiceImpl::new();

    // Phase 18: Dependency resolution
    let dependency_resolution = DependencyResolutionServiceImpl::new();

    // Phase 19: File watching
    let file_watch = FileWatchServiceImpl::new();

    // Phase 20: Configuration cache
    let config_cache_dir = PathBuf::from(&args.config_cache_dir);
    let config_cache = ConfigurationCacheServiceImpl::new(config_cache_dir);

    // Phase 23: Toolchain management
    let toolchain_dir = PathBuf::from(&args.toolchain_dir);
    let toolchain = ToolchainServiceImpl::new(toolchain_dir);

    // Phase 24: Build event streaming
    let build_event_stream = BuildEventStreamServiceImpl::new();

    // Phase 25: Worker process management
    let worker_process = WorkerProcessServiceImpl::new();

    // Phase 26: Build layout / project model
    let build_layout = BuildLayoutServiceImpl::new();

    let listener = UnixListener::bind(&socket_path)?;

    println!("Gradle Substrate Daemon v{}", env!("CARGO_PKG_VERSION"));
    println!("Protocol version: {}", PROTOCOL_VERSION);
    println!("Listening on: {}", args.socket_path);
    println!("Services: control, hash, cache, exec, work, execution-plan, execution-history, cache-orchestration, file-fingerprint, value-snapshot, task-graph, configuration, plugin, build-operations, bootstrap, dependency-resolution, file-watch, config-cache, toolchain, build-event-stream, worker-process, build-layout");

    Server::builder()
        .add_service(ControlServiceServer::new(control))
        .add_service(HashServiceServer::new(hash))
        .add_service(CacheServiceServer::new(cache))
        .add_service(ExecServiceServer::new(exec))
        .add_service(WorkServiceServer::new(work))
        .add_service(ExecutionPlanServiceServer::new(execution_plan))
        .add_service(ExecutionHistoryServiceServer::new(execution_history))
        .add_service(BuildCacheOrchestrationServiceServer::new(cache_orchestration))
        .add_service(FileFingerprintServiceServer::new(file_fingerprint))
        .add_service(ValueSnapshotServiceServer::new(value_snapshot))
        .add_service(TaskGraphServiceServer::new(task_graph))
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
        .serve_with_incoming_shutdown(tokio_stream::wrappers::UnixListenerStream::new(listener), shutdown_signal())
        .await?;

    tracing::info!("Daemon shut down cleanly");

    // Clean up socket file
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    Ok(())
}
