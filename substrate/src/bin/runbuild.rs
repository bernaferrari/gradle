use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    artifact: Option<PathBuf>,

    /// Substrate state directory containing state/config-cache/build-plan-shadow.
    #[arg(long)]
    state_dir: Option<PathBuf>,

    /// Build id to select when multiple cached plans match a project.
    #[arg(long)]
    build_id: Option<String>,

    /// Project directory to register with the daemon when the cached artifact does not contain one.
    #[arg(long)]
    project_dir: Option<PathBuf>,

    /// Maximum Rust task parallelism.
    #[arg(long, default_value_t = 12)]
    max_parallelism: i32,

    /// Optional task paths to execute from the cached plan. Defaults to all tasks.
    #[arg(long = "task")]
    tasks: Vec<String>,

    /// Skip conservative build-definition mtime invalidation checks.
    #[arg(long)]
    skip_invalidation: bool,
}

#[derive(Debug, Deserialize)]
struct ShadowArtifact {
    plan: ShadowPlan,
    stored_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct ShadowPlan {
    build_id: String,
    #[serde(default)]
    projects: Vec<ShadowProject>,
    #[serde(default)]
    tasks: Vec<ShadowTask>,
}

#[derive(Debug, Deserialize)]
struct ShadowProject {
    #[serde(default)]
    project_dir: String,
}

#[derive(Debug, Deserialize)]
struct ShadowTask {
    #[serde(default)]
    outputs: Vec<String>,
    #[serde(default)]
    local_state: Vec<String>,
    #[serde(default)]
    destroyables: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let artifact_path = resolve_artifact_path(&args)?;
    let artifact = read_artifact(&artifact_path)?;
    let build_id = artifact.plan.build_id.clone();
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
    if project_dir.trim().is_empty() {
        return Err(
            "project_dir is required because the cached artifact does not contain one".into(),
        );
    }
    if !args.skip_invalidation {
        validate_build_definition_mtimes(Path::new(&project_dir), artifact.stored_at_ms)?;
    }

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

fn resolve_artifact_path(args: &Args) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = &args.artifact {
        return Ok(path.clone());
    }
    let state_dir = args
        .state_dir
        .as_ref()
        .ok_or("--state-dir is required when --artifact is not supplied")?;
    let project_dir = args
        .project_dir
        .as_ref()
        .ok_or("--project-dir is required when locating a cached artifact")?
        .canonicalize()?;
    let root = build_plan_shadow_root(state_dir);
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let artifact = match read_artifact(&path) {
            Ok(artifact) => artifact,
            Err(_) => continue,
        };
        if let Some(build_id) = &args.build_id {
            if artifact.plan.build_id != *build_id {
                continue;
            }
        }
        if artifact_matches_project(&artifact, &project_dir) {
            candidates.push((path, artifact.plan.build_id));
        }
    }
    match candidates.len() {
        0 => Err(format!(
            "no cached build-plan artifact under '{}' matches project '{}'",
            root.display(),
            project_dir.display()
        )
        .into()),
        1 => Ok(candidates.remove(0).0),
        _ => Err(format!(
            "multiple cached build-plan artifacts match project '{}': {}; pass --build-id or --artifact",
            project_dir.display(),
            candidates
                .iter()
                .map(|(_, build_id)| build_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into()),
    }
}

fn build_plan_shadow_root(state_dir: &Path) -> PathBuf {
    let direct = state_dir.join("config-cache").join("build-plan-shadow");
    if direct.exists() {
        return direct;
    }
    state_dir
        .join("state")
        .join("config-cache")
        .join("build-plan-shadow")
}

fn artifact_matches_project(artifact: &ShadowArtifact, project_dir: &Path) -> bool {
    artifact.plan.projects.iter().any(|project| {
        let value = project.project_dir.trim();
        !value.is_empty()
            && Path::new(value)
                .canonicalize()
                .map(|path| path == project_dir)
                .unwrap_or(false)
    }) || artifact.plan.tasks.iter().any(|task| {
        task.outputs
            .iter()
            .chain(task.local_state.iter())
            .chain(task.destroyables.iter())
            .any(|path| path_belongs_to_project(path, project_dir))
    })
}

fn path_belongs_to_project(path: &str, project_dir: &Path) -> bool {
    let candidate = Path::new(path);
    candidate.is_absolute() && candidate.starts_with(project_dir)
}

fn validate_build_definition_mtimes(
    project_dir: &Path,
    stored_at_ms: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    for path in tracked_build_definition_files(project_dir) {
        let modified_ms = file_modified_ms(&path)?;
        if modified_ms > stored_at_ms {
            return Err(format!(
                "cached build-plan artifact is stale: '{}' was modified at {}ms after artifact stored_at_ms {}",
                path.display(),
                modified_ms,
                stored_at_ms
            )
            .into());
        }
    }
    Ok(())
}

fn tracked_build_definition_files(project_dir: &Path) -> Vec<PathBuf> {
    [
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
        "gradle.properties",
        "gradle/libs.versions.toml",
    ]
    .into_iter()
    .map(|relative| project_dir.join(relative))
    .filter(|path| path.exists())
    .collect()
}

fn file_modified_ms(path: &Path) -> Result<i64, Box<dyn std::error::Error>> {
    Ok(path
        .metadata()?
        .modified()?
        .duration_since(UNIX_EPOCH)?
        .as_millis() as i64)
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
