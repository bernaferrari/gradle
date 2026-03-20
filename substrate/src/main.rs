use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::net::UnixListener;
use tokio::signal;
use tonic::transport::Server;

use gradle_substrate_daemon::{
    proto::{
        cache_service_server::CacheServiceServer,
        control_service_server::ControlServiceServer,
        execution_plan_service_server::ExecutionPlanServiceServer,
        exec_service_server::ExecServiceServer,
        hash_service_server::HashServiceServer,
        work_service_server::WorkServiceServer,
    },
    server::{cache::CacheServiceImpl, control::ControlServiceImpl, exec::ExecServiceImpl, execution_plan::ExecutionPlanServiceImpl, hash::HashServiceImpl, work::WorkServiceImpl},
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

    let control = ControlServiceImpl::new(shutdown_tx.clone());
    let hash = HashServiceImpl;
    let cache_dir = PathBuf::from(&args.cache_dir);
    std::fs::create_dir_all(&cache_dir)?;
    let cache = CacheServiceImpl::new(cache_dir);
    let exec = ExecServiceImpl::new();
    let work_scheduler = Arc::new(gradle_substrate_daemon::server::work::WorkerScheduler::new(
        num_cpus::get(),
    ));
    let work = WorkServiceImpl::new(work_scheduler.clone());
    let execution_plan = ExecutionPlanServiceImpl::new(work_scheduler);

    let listener = UnixListener::bind(&socket_path)?;

    println!("Gradle Substrate Daemon v{}", env!("CARGO_PKG_VERSION"));
    println!("Protocol version: {}", PROTOCOL_VERSION);
    println!("Listening on: {}", args.socket_path);

    Server::builder()
        .add_service(ControlServiceServer::new(control))
        .add_service(HashServiceServer::new(hash))
        .add_service(CacheServiceServer::new(cache))
        .add_service(ExecServiceServer::new(exec))
        .add_service(WorkServiceServer::new(work))
        .add_service(ExecutionPlanServiceServer::new(execution_plan))
        .serve_with_incoming_shutdown(tokio_stream::wrappers::UnixListenerStream::new(listener), shutdown_signal())
        .await?;

    tracing::info!("Daemon shut down cleanly");

    // Clean up socket file
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    Ok(())
}
