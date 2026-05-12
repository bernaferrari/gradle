use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use gradle_substrate_daemon::proto::bootstrap_service_client::BootstrapServiceClient;
use gradle_substrate_daemon::proto::dag_executor_service_client::DagExecutorServiceClient;
use gradle_substrate_daemon::proto::{CompleteBuildRequest, InitBuildRequest, RunBuildRequest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
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
    #[serde(default)]
    input_fingerprints: Vec<ShadowInputFingerprint>,
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
    path: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    input_specs: Vec<ShadowInputSpec>,
    #[serde(default)]
    outputs: Vec<String>,
    #[serde(default)]
    local_state: Vec<String>,
    #[serde(default)]
    destroyables: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ShadowInputSpec {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    value: String,
}

#[derive(Debug, Deserialize)]
struct ShadowInputFingerprint {
    #[serde(default)]
    task_path: String,
    #[serde(default)]
    input_name: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    exists: bool,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    modified_ms: i64,
    #[serde(default)]
    sha256: String,
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
    let project_dir_path = Path::new(&project_dir).canonicalize()?;
    let project_dir = project_dir_path.to_string_lossy().to_string();
    if !args.skip_invalidation {
        validate_build_definition_mtimes(&project_dir_path, artifact.stored_at_ms)?;
        validate_task_input_mtimes(&artifact, &project_dir_path, artifact.stored_at_ms)?;
        validate_input_fingerprints(&artifact, &project_dir_path)?;
    }
    validate_plan_dependencies(&artifact)?;
    let task_filter = resolve_task_filter(&artifact, &args.tasks)?;

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
            task_filter,
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

fn validate_task_input_mtimes(
    artifact: &ShadowArtifact,
    project_dir: &Path,
    stored_at_ms: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let produced_paths = captured_produced_paths(artifact);
    for task in &artifact.plan.tasks {
        for input in &task.input_specs {
            if input.kind != "path" {
                continue;
            }
            let path = Path::new(&input.value);
            if !path.is_absolute() || !path.starts_with(project_dir) || !path.exists() {
                continue;
            }
            if produced_paths
                .iter()
                .any(|produced| path == produced || path.starts_with(produced))
            {
                continue;
            }
            let modified_ms = newest_modified_ms(path)?;
            if modified_ms > stored_at_ms {
                return Err(format!(
                    "cached build-plan artifact is stale: task input '{}' was modified at {}ms after artifact stored_at_ms {}",
                    path.display(),
                    modified_ms,
                    stored_at_ms
                )
                .into());
            }
        }
    }
    Ok(())
}

fn captured_produced_paths(artifact: &ShadowArtifact) -> Vec<PathBuf> {
    artifact
        .plan
        .tasks
        .iter()
        .flat_map(|task| {
            task.outputs
                .iter()
                .chain(task.local_state.iter())
                .chain(task.destroyables.iter())
        })
        .filter_map(|value| {
            let path = Path::new(value);
            path.is_absolute().then(|| path.to_path_buf())
        })
        .collect()
}

fn validate_input_fingerprints(
    artifact: &ShadowArtifact,
    project_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if artifact.input_fingerprints.is_empty() && has_project_path_inputs(artifact, project_dir) {
        return Err(
            "cached build-plan artifact is unsafe for direct RunBuild: missing input_fingerprints"
                .into(),
        );
    }

    for expected in &artifact.input_fingerprints {
        let path = Path::new(&expected.path);
        if !path.is_absolute() || !path.starts_with(project_dir) {
            return Err(format!(
                "cached build-plan artifact has unsupported input fingerprint outside project: task '{}' input '{}' path '{}'",
                expected.task_path, expected.input_name, expected.path
            )
            .into());
        }
        let actual = fingerprint_input_path(path)?;
        if actual.kind != expected.kind
            || actual.exists != expected.exists
            || actual.size != expected.size
            || actual.sha256 != expected.sha256
        {
            return Err(format!(
                "cached build-plan artifact is stale: task '{}' input '{}' path '{}' fingerprint changed (expected kind={} exists={} size={} sha256={} modified_ms={}, actual kind={} exists={} size={} sha256={} modified_ms={})",
                expected.task_path,
                expected.input_name,
                expected.path,
                expected.kind,
                expected.exists,
                expected.size,
                expected.sha256,
                expected.modified_ms,
                actual.kind,
                actual.exists,
                actual.size,
                actual.sha256,
                actual.modified_ms
            )
            .into());
        }
    }
    Ok(())
}

fn validate_plan_dependencies(artifact: &ShadowArtifact) -> Result<(), Box<dyn std::error::Error>> {
    let task_paths = task_path_set(artifact);
    for task in &artifact.plan.tasks {
        if task.path.trim().is_empty() {
            return Err("cached build-plan artifact contains a task without a path".into());
        }
        for dependency in &task.depends_on {
            if !task_paths.contains(dependency) {
                return Err(format!(
                    "cached build-plan artifact is incomplete: task '{}' depends on missing task '{}'",
                    task.path, dependency
                )
                .into());
            }
        }
    }
    Ok(())
}

fn resolve_task_filter(
    artifact: &ShadowArtifact,
    requested_tasks: &[String],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if requested_tasks.is_empty() {
        return Ok(Vec::new());
    }
    let task_paths = task_path_set(artifact);
    let mut selected = HashSet::new();
    for task in requested_tasks {
        if !task.starts_with(':') {
            return Err(format!(
                "direct RunBuild only supports fully-qualified task paths, got '{}'",
                task
            )
            .into());
        }
        if !task_paths.contains(task) {
            return Err(format!(
                "cached build-plan artifact does not contain requested task '{}'",
                task
            )
            .into());
        }
        include_task_and_dependencies(artifact, task, &task_paths, &mut selected)?;
    }
    let mut result = selected.into_iter().collect::<Vec<_>>();
    result.sort();
    Ok(result)
}

fn include_task_and_dependencies(
    artifact: &ShadowArtifact,
    task_path: &str,
    task_paths: &HashSet<String>,
    selected: &mut HashSet<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !selected.insert(task_path.to_string()) {
        return Ok(());
    }
    let task = artifact
        .plan
        .tasks
        .iter()
        .find(|candidate| candidate.path == task_path)
        .ok_or_else(|| format!("cached build-plan artifact does not contain task '{task_path}'"))?;
    for dependency in &task.depends_on {
        if !task_paths.contains(dependency) {
            return Err(format!(
                "cached build-plan artifact is incomplete: task '{}' depends on missing task '{}'",
                task.path, dependency
            )
            .into());
        }
        include_task_and_dependencies(artifact, dependency, task_paths, selected)?;
    }
    Ok(())
}

fn task_path_set(artifact: &ShadowArtifact) -> HashSet<String> {
    artifact
        .plan
        .tasks
        .iter()
        .map(|task| task.path.clone())
        .collect()
}

fn has_project_path_inputs(artifact: &ShadowArtifact, project_dir: &Path) -> bool {
    let produced_paths = captured_produced_paths(artifact);
    artifact.plan.tasks.iter().any(|task| {
        task.input_specs.iter().any(|input| {
            if input.kind != "path" {
                return false;
            }
            let path = Path::new(&input.value);
            path.is_absolute()
                && path.starts_with(project_dir)
                && !produced_paths
                    .iter()
                    .any(|produced| path == produced || path.starts_with(produced))
        })
    })
}

struct CurrentInputFingerprint {
    kind: String,
    exists: bool,
    size: u64,
    modified_ms: i64,
    sha256: String,
}

fn fingerprint_input_path(
    path: &Path,
) -> Result<CurrentInputFingerprint, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(CurrentInputFingerprint {
            kind: "missing".to_string(),
            exists: false,
            size: 0,
            modified_ms: 0,
            sha256: String::new(),
        });
    }

    let metadata = path.metadata()?;
    let modified_ms = metadata_modified_ms(&metadata)?;
    if metadata.is_file() {
        return Ok(CurrentInputFingerprint {
            kind: "file".to_string(),
            exists: true,
            size: metadata.len(),
            modified_ms,
            sha256: sha256_file(path)?,
        });
    }
    if metadata.is_dir() {
        let (size, newest_modified_ms, sha256) = fingerprint_directory(path)?;
        return Ok(CurrentInputFingerprint {
            kind: "directory".to_string(),
            exists: true,
            size,
            modified_ms: newest_modified_ms.max(modified_ms),
            sha256,
        });
    }

    Ok(CurrentInputFingerprint {
        kind: "other".to_string(),
        exists: true,
        size: 0,
        modified_ms,
        sha256: String::new(),
    })
}

fn fingerprint_directory(path: &Path) -> Result<(u64, i64, String), Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    collect_directory_files(path, path, &mut files)?;
    let mut hasher = Sha256::new();
    let mut total_size = 0;
    let mut newest_modified_ms = metadata_modified_ms(&path.metadata()?)?;
    for (relative_path, file_path) in files {
        let metadata = file_path.metadata()?;
        total_size += metadata.len();
        newest_modified_ms = newest_modified_ms.max(metadata_modified_ms(&metadata)?);
        hasher.update(relative_path.as_bytes());
        hasher.update([0]);
        hasher.update(sha256_file(&file_path)?.as_bytes());
        hasher.update([0]);
    }
    Ok((
        total_size,
        newest_modified_ms,
        format!("{:x}", hasher.finalize()),
    ))
}

fn collect_directory_files(
    root: &Path,
    path: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(path)? {
        let child = entry?.path();
        let metadata = child.metadata()?;
        if metadata.is_dir() {
            collect_directory_files(root, &child, files)?;
        } else if metadata.is_file() {
            let relative_path = child
                .strip_prefix(root)?
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            files.push((relative_path, child));
        }
    }
    files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn metadata_modified_ms(metadata: &std::fs::Metadata) -> Result<i64, Box<dyn std::error::Error>> {
    Ok(metadata.modified()?.duration_since(UNIX_EPOCH)?.as_millis() as i64)
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

fn newest_modified_ms(path: &Path) -> Result<i64, Box<dyn std::error::Error>> {
    let metadata = path.metadata()?;
    if metadata.is_file() {
        return file_modified_ms(path);
    }
    let mut newest = file_modified_ms(path)?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let child = entry?.path();
            newest = newest.max(newest_modified_ms(&child)?);
        }
    }
    Ok(newest)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact_with_path_input(
        path: &Path,
        fingerprints: Vec<ShadowInputFingerprint>,
    ) -> ShadowArtifact {
        ShadowArtifact {
            plan: ShadowPlan {
                build_id: "build:test".to_string(),
                projects: Vec::new(),
                tasks: vec![ShadowTask {
                    path: ":compileJava".to_string(),
                    depends_on: Vec::new(),
                    input_specs: vec![ShadowInputSpec {
                        kind: "path".to_string(),
                        value: path.to_string_lossy().into_owned(),
                    }],
                    outputs: Vec::new(),
                    local_state: Vec::new(),
                    destroyables: Vec::new(),
                }],
            },
            stored_at_ms: 0,
            input_fingerprints: fingerprints,
        }
    }

    #[test]
    fn validates_matching_file_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src/Main.java");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "class Main {}\n").unwrap();
        let current = fingerprint_input_path(&file).unwrap();
        let artifact = artifact_with_path_input(
            &file,
            vec![ShadowInputFingerprint {
                task_path: ":compileJava".to_string(),
                input_name: "source".to_string(),
                path: file.to_string_lossy().into_owned(),
                kind: current.kind,
                exists: current.exists,
                size: current.size,
                modified_ms: current.modified_ms,
                sha256: current.sha256,
            }],
        );

        validate_input_fingerprints(&artifact, temp.path()).unwrap();
    }

    #[test]
    fn rejects_changed_file_content_even_when_path_still_exists() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src/Main.java");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "class Main {}\n").unwrap();
        let current = fingerprint_input_path(&file).unwrap();
        let artifact = artifact_with_path_input(
            &file,
            vec![ShadowInputFingerprint {
                task_path: ":compileJava".to_string(),
                input_name: "source".to_string(),
                path: file.to_string_lossy().into_owned(),
                kind: current.kind,
                exists: current.exists,
                size: current.size,
                modified_ms: current.modified_ms,
                sha256: current.sha256,
            }],
        );

        std::fs::write(&file, "class Main { String changed; }\n").unwrap();
        let error = validate_input_fingerprints(&artifact, temp.path()).unwrap_err();

        assert!(error.to_string().contains("fingerprint changed"));
    }

    #[test]
    fn rejects_missing_fingerprints_for_project_path_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src/Main.java");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "class Main {}\n").unwrap();
        let artifact = artifact_with_path_input(&file, Vec::new());

        let error = validate_input_fingerprints(&artifact, temp.path()).unwrap_err();

        assert!(error.to_string().contains("missing input_fingerprints"));
    }

    #[test]
    fn expands_requested_task_filter_to_dependency_closure() {
        let artifact = ShadowArtifact {
            plan: ShadowPlan {
                build_id: "build:test".to_string(),
                projects: Vec::new(),
                tasks: vec![
                    ShadowTask {
                        path: ":compileJava".to_string(),
                        depends_on: Vec::new(),
                        input_specs: Vec::new(),
                        outputs: Vec::new(),
                        local_state: Vec::new(),
                        destroyables: Vec::new(),
                    },
                    ShadowTask {
                        path: ":classes".to_string(),
                        depends_on: vec![":compileJava".to_string()],
                        input_specs: Vec::new(),
                        outputs: Vec::new(),
                        local_state: Vec::new(),
                        destroyables: Vec::new(),
                    },
                    ShadowTask {
                        path: ":build".to_string(),
                        depends_on: vec![":classes".to_string()],
                        input_specs: Vec::new(),
                        outputs: Vec::new(),
                        local_state: Vec::new(),
                        destroyables: Vec::new(),
                    },
                ],
            },
            stored_at_ms: 0,
            input_fingerprints: Vec::new(),
        };

        let selected = resolve_task_filter(&artifact, &[":build".to_string()]).unwrap();

        assert_eq!(selected, vec![":build", ":classes", ":compileJava"]);
    }

    #[test]
    fn rejects_unqualified_or_unknown_requested_tasks() {
        let artifact = ShadowArtifact {
            plan: ShadowPlan {
                build_id: "build:test".to_string(),
                projects: Vec::new(),
                tasks: vec![ShadowTask {
                    path: ":build".to_string(),
                    depends_on: Vec::new(),
                    input_specs: Vec::new(),
                    outputs: Vec::new(),
                    local_state: Vec::new(),
                    destroyables: Vec::new(),
                }],
            },
            stored_at_ms: 0,
            input_fingerprints: Vec::new(),
        };

        let unqualified = resolve_task_filter(&artifact, &["build".to_string()]).unwrap_err();
        assert!(unqualified.to_string().contains("fully-qualified"));
        let unknown = resolve_task_filter(&artifact, &[":test".to_string()]).unwrap_err();
        assert!(unknown
            .to_string()
            .contains("does not contain requested task"));
    }

    #[test]
    fn rejects_incomplete_dependency_graphs() {
        let artifact = ShadowArtifact {
            plan: ShadowPlan {
                build_id: "build:test".to_string(),
                projects: Vec::new(),
                tasks: vec![ShadowTask {
                    path: ":build".to_string(),
                    depends_on: vec![":classes".to_string()],
                    input_specs: Vec::new(),
                    outputs: Vec::new(),
                    local_state: Vec::new(),
                    destroyables: Vec::new(),
                }],
            },
            stored_at_ms: 0,
            input_fingerprints: Vec::new(),
        };

        let error = validate_plan_dependencies(&artifact).unwrap_err();

        assert!(error.to_string().contains("depends on missing task"));
    }
}
