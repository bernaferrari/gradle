use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use gradle_substrate_daemon::proto::bootstrap_service_client::BootstrapServiceClient;
use gradle_substrate_daemon::proto::dag_executor_service_client::DagExecutorServiceClient;
use gradle_substrate_daemon::proto::{CompleteBuildRequest, InitBuildRequest, RunBuildRequest};
use serde::Deserialize;
use tonic::transport::Endpoint;

#[derive(Parser, Debug)]
#[command(name = "gradle-substrate-runbuild")]
#[command(
    about = "Run a cached Rust build-plan shadow directly through an existing substrate daemon"
)]
struct Args {
    /// Existing daemon endpoint, for example tcp://127.0.0.1:51234.
    #[arg(long)]
    endpoint: String,

    /// Build-plan shadow artifact JSON written under state/config-cache/build-plan-shadow.
    #[arg(long)]
    artifact: PathBuf,

    /// Project directory to register with the daemon when the cached artifact does not contain one.
    #[arg(long)]
    project_dir: Option<PathBuf>,

    /// Maximum Rust task parallelism.
    #[arg(long, default_value_t = 12)]
    max_parallelism: i32,

    /// Optional task paths to execute from the cached plan. Defaults to all tasks.
    #[arg(long = "task")]
    tasks: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ShadowArtifact {
    plan: ShadowPlan,
}

#[derive(Debug, Deserialize)]
struct ShadowPlan {
    build_id: String,
    #[serde(default)]
    projects: Vec<ShadowProject>,
}

#[derive(Debug, Deserialize)]
struct ShadowProject {
    #[serde(default)]
    project_dir: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let artifact = read_artifact(&args.artifact)?;
    let build_id = artifact.plan.build_id;
    let project_dir = args
        .project_dir
        .as_ref()
        .map(|path| path.to_string_lossy().to_string())
        .or_else(|| {
            artifact.plan.projects.iter().find_map(|project| {
                let value = project.project_dir.trim();
                if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            })
        })
        .unwrap_or_default();

    let channel = connect_tcp(&args.endpoint).await?;
    let mut bootstrap = BootstrapServiceClient::new(channel.clone());
    let mut dag = DagExecutorServiceClient::new(channel);
    let start_ms = now_ms();
    let session_id = format!("direct-runbuild-{start_ms}");

    bootstrap
        .init_build(InitBuildRequest {
            build_id: build_id.clone(),
            project_dir,
            start_time_ms: start_ms,
            requested_parallelism: args.max_parallelism,
            system_properties: HashMap::new(),
            requested_features: vec!["direct-runbuild".to_string()],
            session_id,
        })
        .await?;

    let response = dag
        .run_build(RunBuildRequest {
            build_id: build_id.clone(),
            max_parallelism: args.max_parallelism,
            task_filter: args.tasks,
            task_contexts: HashMap::new(),
            allow_jvm_forwarding: false,
        })
        .await?
        .into_inner();

    let outcome = response.final_status.clone();
    let _ = bootstrap
        .complete_build(CompleteBuildRequest {
            build_id,
            outcome: outcome.clone(),
            duration_ms: response.total_duration_ms,
        })
        .await;

    println!(
        "direct-runbuild status={} tasks={} succeeded={} failed={} skipped={} up_to_date={} from_cache={} jvm_forwarded={} duration_ms={} plan_source={}",
        response.final_status,
        response.total_tasks,
        response.tasks_succeeded,
        response.tasks_failed,
        response.tasks_skipped,
        response.tasks_up_to_date,
        response.tasks_from_cache,
        response.tasks_forwarded_to_jvm,
        response.total_duration_ms,
        response.plan_source
    );
    if !response.failure_message.is_empty() {
        eprintln!("{}", response.failure_message);
    }
    for task in response.task_details {
        println!(
            "task path={} type={} outcome={} mode={} duration_ms={}",
            task.task_path, task.task_type, task.outcome, task.execution_mode, task.duration_ms
        );
    }

    if outcome == "COMPLETED" {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn read_artifact(path: &PathBuf) -> Result<ShadowArtifact, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn connect_tcp(
    endpoint: &str,
) -> Result<tonic::transport::Channel, Box<dyn std::error::Error>> {
    let Some(address) = endpoint.strip_prefix("tcp://") else {
        return Err(format!(
            "only tcp:// endpoints are supported by this preview tool: {endpoint}"
        )
        .into());
    };
    Ok(Endpoint::from_shared(format!("http://{address}"))?
        .connect()
        .await?)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
