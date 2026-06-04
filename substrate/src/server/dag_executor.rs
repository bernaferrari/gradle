use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use super::dependency_solver::graph_builder;
use super::event_dispatcher::EventDispatcher;
use super::execution_kernel::{
    admit_build_plan, KernelAdmission, KernelBuildPlan, KernelDependencyConfiguration,
    KernelDependencyGraph, KernelDependencyRequest, KernelRepository, KernelTaskPlan,
};
use super::file_watch::VfsDeltaStore;
use super::scopes::{BuildId, ScopeRegistry};
use super::work::WorkerScheduler;

use crate::client::jvm_host_bridge::SharedJvmHostBridge;
use crate::proto::{
    dag_executor_service_server::DagExecutorService,
    execution_plan_service_server::ExecutionPlanService,
    task_graph_service_server::TaskGraphService, AwaitBuildCompletionRequest,
    AwaitBuildCompletionResponse, BuildCachePackFile, BuildEventMessage, CancelBuildRequest,
    CancelBuildResponse, GetBuildStatusRequest, GetBuildStatusResponse, GetNextTaskRequest,
    GetNextTaskResponse, NotifyTaskFinishedRequest, NotifyTaskFinishedResponse,
    NotifyTaskStartedRequest, NotifyTaskStartedResponse, PackCacheEntryRequest, PredictedOutcome,
    RecordOutcomeRequest, ResolveExecutionPlanRequest, ResolvePlanRequest, RunBuildRequest,
    RunBuildResponse, StartBuildRequest, StartBuildResponse, TaskExecutionDetail,
    TaskFinishedRequest, TaskStartedRequest, TaskStatusEntry, UnpackCacheEntryRequest,
    WorkMetadata,
};
use crate::server::cache::LocalCacheStore;
use crate::server::cache_packaging::BuildCachePackagingServiceImpl;
use crate::server::task_executor::{TaskExecutorRegistry, TaskInput};

/// Sentinel value returned by GetNextTask when the build is complete.
const BUILD_COMPLETE_SENTINEL: &str = "__BUILD_COMPLETE__";

/// Status of a single task within a build.
#[derive(Clone, Debug)]
struct TaskSlot {
    task_path: String,
    task_type: String,
    status: String,
    start_time_ms: i64,
    duration_ms: i64,
    dependencies: Vec<String>,
    work_metadata: Option<WorkMetadata>,
    predicted_outcome: i32,
    input_fingerprint: String,
    execution_context_json: String,
    estimated_duration_ms: i64,
    critical_path_remaining_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReadyTask {
    task_path: String,
    critical_path_remaining_ms: i64,
    sequence: u64,
}

impl Ord for ReadyTask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.critical_path_remaining_ms
            .cmp(&other.critical_path_remaining_ms)
            .then_with(|| other.sequence.cmp(&self.sequence))
            .then_with(|| other.task_path.cmp(&self.task_path))
    }
}

impl PartialOrd for ReadyTask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Runtime state for an active build execution.
#[allow(dead_code)]
struct BuildExecution {
    build_id: BuildId,
    status: String,
    start_time_ms: i64,
    /// Tasks whose dependencies are all satisfied.
    ready_queue: std::sync::Mutex<BinaryHeap<ReadyTask>>,
    ready_sequence: u64,
    /// Tasks currently being executed by the JVM.
    executing: std::sync::Mutex<HashSet<String>>,
    /// Reverse adjacency: task -> list of tasks that depend on it.
    dependents: HashMap<String, Vec<String>>,
    /// All task slots keyed by task_path.
    tasks: HashMap<String, TaskSlot>,
    /// Set of task paths to include (None = all).
    task_filter: Option<HashSet<String>>,
    total_tasks: i32,
    max_parallelism: usize,
    /// Notify when a task finishes (wakes AwaitBuildCompletion).
    completion_notify: Arc<tokio::sync::Notify>,
    /// Watch channel for cancellation.
    cancel_rx: tokio::sync::watch::Receiver<bool>,
    cancel_tx: tokio::sync::watch::Sender<bool>,
    failure_message: String,
}

impl BuildExecution {
    fn completed_count(&self) -> i32 {
        self.tasks
            .values()
            .filter(|t| matches!(t.status.as_str(), "SUCCEEDED" | "FAILED" | "SKIPPED"))
            .count() as i32
    }

    fn executing_count(&self) -> i32 {
        self.executing
            .lock()
            .expect("executing lock should not be poisoned")
            .len() as i32
    }

    fn pending_count(&self) -> i32 {
        self.tasks
            .values()
            .filter(|t| t.status == "PENDING")
            .count() as i32
    }

    fn failed_count(&self) -> i32 {
        self.tasks.values().filter(|t| t.status == "FAILED").count() as i32
    }

    fn skipped_count(&self) -> i32 {
        self.tasks
            .values()
            .filter(|t| t.status == "SKIPPED")
            .count() as i32
    }

    fn is_cancelled(&self) -> bool {
        *self.cancel_rx.borrow()
    }

    fn is_terminal(&self) -> bool {
        matches!(self.status.as_str(), "COMPLETED" | "FAILED" | "CANCELLED")
    }
}

/// Current time in milliseconds since UNIX epoch.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Build a `TaskInput` from optional JSON context.
fn build_task_input(task_type: &str, context_json: Option<&String>) -> TaskInput {
    let mut input = TaskInput::new(task_type);
    if let Some(json) = context_json {
        if let Ok(map) = serde_json::from_str::<HashMap<String, serde_json::Value>>(json) {
            if let Some(v) = map.get("source_files") {
                if let Some(arr) = v.as_array() {
                    input.source_files = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(std::path::PathBuf::from))
                        .collect();
                }
            }
            if let Some(v) = map.get("target_dir") {
                if let Some(s) = v.as_str() {
                    input.target_dir = std::path::PathBuf::from(s);
                }
            }
            if let Some(v) = map.get("options") {
                if let Some(obj) = v.as_object() {
                    input.options = obj
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|sv| (k.clone(), sv.to_string())))
                        .collect();
                }
            }
            if let Some(v) = map.get("output_files") {
                if let Some(arr) = v.as_array() {
                    let values = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>();
                    if !values.is_empty() {
                        input.options.insert(
                            "output_files_json".to_string(),
                            serde_json::to_string(&values).unwrap_or_default(),
                        );
                    }
                }
            }
        }
    }
    input
}

fn declared_outputs_present(task_type: &str, context_json: Option<&String>) -> bool {
    let Some(json) = context_json else {
        return true;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return true;
    };
    let Some(outputs) = value.get("output_files").and_then(|v| v.as_array()) else {
        return true;
    };
    if outputs.is_empty() {
        return true;
    }
    let output_paths: Vec<&str> = outputs
        .iter()
        .filter_map(|v| v.as_str())
        .filter(|path| !path.is_empty())
        .collect();
    if output_paths.is_empty() {
        return true;
    }

    let mut required_count = 0usize;
    for path in output_paths {
        if java_compile_optional_output(task_type, path) {
            continue;
        }
        required_count += 1;
        if !std::path::Path::new(path).exists() {
            return false;
        }
    }

    required_count > 0
}

fn java_compile_optional_output(task_type: &str, path: &str) -> bool {
    if task_type != "JavaCompile" {
        return false;
    }
    let normalized = path.replace('\\', "/");
    normalized.contains("/build/generated/sources/annotationProcessor/java/")
        || normalized.contains("/build/generated/sources/headers/java/")
        || normalized.ends_with("/previous-compilation-data.bin")
}

fn context_allows_up_to_date(context_json: Option<&String>) -> bool {
    let Some(json) = context_json else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("up_to_date_enabled")
                .and_then(|flag| flag.as_bool())
        })
        .unwrap_or(false)
}

fn context_is_no_source(context_json: Option<&String>) -> bool {
    let Some(json) = context_json else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| value.get("no_source").and_then(|flag| flag.as_bool()))
        .unwrap_or(false)
}

fn merged_task_context(
    override_json: Option<&String>,
    base_json: Option<String>,
) -> Option<String> {
    match (override_json, base_json) {
        (Some(override_json), Some(base_json)) => {
            let Ok(mut base) = serde_json::from_str::<serde_json::Value>(&base_json) else {
                return Some(override_json.clone());
            };
            let Ok(override_value) = serde_json::from_str::<serde_json::Value>(override_json)
            else {
                return Some(base_json);
            };
            let (Some(base_obj), Some(override_obj)) =
                (base.as_object_mut(), override_value.as_object())
            else {
                return Some(override_json.clone());
            };
            for (key, value) in override_obj {
                base_obj.insert(key.clone(), value.clone());
            }
            Some(base.to_string())
        }
        (Some(override_json), None) => Some(override_json.clone()),
        (None, Some(base_json)) => Some(base_json),
        (None, None) => None,
    }
}

fn context_vfs_changed_paths(context_json: Option<&String>) -> Vec<String> {
    let Some(json) = context_json else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    if !value
        .get("trusted_vfs_delta")
        .and_then(|flag| flag.as_bool())
        .unwrap_or(false)
    {
        return Vec::new();
    }
    value
        .get("trusted_changed_paths")
        .and_then(|paths| paths.as_array())
        .map(|paths| {
            paths
                .iter()
                .filter_map(|path| path.as_str())
                .filter(|path| !path.is_empty())
                .map(normalize_context_path)
                .collect()
        })
        .unwrap_or_default()
}

fn context_has_trusted_vfs_delta(context_json: Option<&String>) -> bool {
    let Some(json) = context_json else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("trusted_vfs_delta")
                .and_then(|flag| flag.as_bool())
        })
        .unwrap_or(false)
}

fn context_vfs_delta_since_ms(context_json: Option<&String>) -> i64 {
    let Some(json) = context_json else {
        return 0;
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("vfs_delta_since_ms")
                .and_then(|timestamp| timestamp.as_i64())
        })
        .unwrap_or(0)
}

fn apply_vfs_delta_to_work_metadata(meta: &mut WorkMetadata, context_json: Option<&String>) {
    let changed_paths = context_vfs_changed_paths(context_json);
    if changed_paths.is_empty() {
        return;
    }
    let intersects_input = meta.input_file_fingerprints.keys().any(|input| {
        let input = normalize_context_path(input);
        changed_paths
            .iter()
            .any(|changed| paths_intersect_normalized(changed, &input))
    });
    if intersects_input {
        meta.rebuild_reasons
            .push("trusted VFS delta intersects task inputs".to_string());
    }
}

fn normalize_context_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn paths_intersect_normalized(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|rest| rest.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn context_output_paths(context_json: Option<&String>) -> Vec<PathBuf> {
    let Some(json) = context_json else {
        return Vec::new();
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("output_files")
                .and_then(|outputs| outputs.as_array())
                .map(|outputs| {
                    outputs
                        .iter()
                        .filter_map(|output| output.as_str())
                        .filter(|output| !output.is_empty())
                        .map(PathBuf::from)
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_default()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}

#[cfg(unix)]
async fn apply_cached_executable_bit(path: &Path, executable: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("metadata {}: {error}", path.display()))?;
    let mut permissions = metadata.permissions();
    let current = permissions.mode();
    let next = if executable {
        current | 0o111
    } else {
        current & !0o111
    };
    permissions.set_mode(next);
    tokio::fs::set_permissions(path, permissions)
        .await
        .map_err(|error| format!("chmod {}: {error}", path.display()))
}

#[cfg(not(unix))]
async fn apply_cached_executable_bit(_path: &Path, _executable: bool) -> Result<(), String> {
    Ok(())
}

fn collect_output_cache_files(
    root: &Path,
    relative_prefix: &str,
    files: &mut Vec<BuildCachePackFile>,
) -> Result<String, String> {
    let metadata = std::fs::metadata(root)
        .map_err(|error| format!("metadata output {}: {error}", root.display()))?;
    if metadata.is_file() {
        files.push(BuildCachePackFile {
            path: relative_prefix.to_string(),
            content: std::fs::read(root)
                .map_err(|error| format!("read output {}: {error}", root.display()))?,
            executable: is_executable(root),
        });
        return Ok("file".to_string());
    }
    if metadata.is_dir() {
        collect_output_directory_files(root, root, relative_prefix, files)?;
        return Ok("dir".to_string());
    }
    Err(format!(
        "output {} is neither a regular file nor a directory",
        root.display()
    ))
}

fn collect_output_directory_files(
    base: &Path,
    dir: &Path,
    relative_prefix: &str,
    files: &mut Vec<BuildCachePackFile>,
) -> Result<(), String> {
    let mut entries = std::fs::read_dir(dir)
        .map_err(|error| format!("read output dir {}: {error}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read output dir entry {}: {error}", dir.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|error| format!("metadata output {}: {error}", path.display()))?;
        if metadata.is_dir() {
            collect_output_directory_files(base, &path, relative_prefix, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(base)
                .map_err(|error| format!("relativize output {}: {error}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            files.push(BuildCachePackFile {
                path: format!("{relative_prefix}/{relative}"),
                content: std::fs::read(&path)
                    .map_err(|error| format!("read output {}: {error}", path.display()))?,
                executable: is_executable(&path),
            });
        }
    }
    Ok(())
}

fn output_kinds_from_metadata(
    metadata: &HashMap<String, String>,
    output_count: usize,
) -> Vec<Option<String>> {
    metadata
        .get("output_kinds_json")
        .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
        .map(|kinds| {
            (0..output_count)
                .map(|index| kinds.get(index).cloned())
                .collect()
        })
        .unwrap_or_else(|| vec![None; output_count])
}

fn refreshed_work_metadata(meta: &WorkMetadata, context_json: &str) -> WorkMetadata {
    let mut refreshed = meta.clone();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(context_json) else {
        return refreshed;
    };
    let Some(source_files) = value.get("source_files").and_then(|files| files.as_array()) else {
        return refreshed;
    };
    let paths = source_files
        .iter()
        .filter_map(|file| file.as_str())
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return refreshed;
    }
    refreshed.input_file_fingerprints = super::task_graph::work_input_file_fingerprints(&paths)
        .into_iter()
        .collect();
    refreshed
}

/// Result from a spawned task execution, sent back via channel.
struct TaskExecResult {
    task_path: String,
    task_type: String,
    success: bool,
    outcome: String,
    duration_ms: i64,
    execution_mode: String,
    error_message: String,
}

fn is_jvm_execution_mode(mode: &str) -> bool {
    mode == "jvm_host" || mode.starts_with("mock-jvm") || mode.starts_with("jvm_")
}

fn rust_execution_mode(task_type: &str) -> &'static str {
    match task_type {
        "JavaCompile" | "Exec" | "JavaExec" | "Javadoc" | "TestExec" => "rust_process",
        _ => "rust_in_process",
    }
}

/// DAG Executor service.
/// Orchestrates build execution by managing task scheduling, parallelism,
/// cancellation, and event dispatch.
pub struct DagExecutorServiceImpl {
    /// Active build executions.
    builds: Arc<DashMap<BuildId, BuildExecution>>,
    /// Shared worker scheduler for bounded parallelism.
    scheduler: Arc<WorkerScheduler>,
    /// Task graph service for plan resolution and progress tracking.
    task_graph: Arc<super::task_graph::TaskGraphServiceImpl>,
    /// Execution plan service for UP-TO-DATE detection.
    execution_plan: Arc<super::execution_plan::ExecutionPlanServiceImpl>,
    /// Event dispatchers for automatic fan-out (console + metrics).
    dispatchers: Vec<Arc<dyn EventDispatcher>>,
    /// Native task executor registry for RunBuild authoritative execution.
    executor_registry: Arc<TaskExecutorRegistry>,
    /// JVM compatibility host bridge for explicit legacy task execution.
    jvm_host_bridge: Option<SharedJvmHostBridge>,
    /// Scope registry for build-session membership validation.
    scope_registry: Option<Arc<ScopeRegistry>>,
    /// Local Rust build cache store used for authoritative output restore/store.
    local_cache: Option<Arc<LocalCacheStore>>,
    /// Retained daemon file-watch/VFS deltas for fail-closed up-to-date admission.
    vfs_delta_store: Option<Arc<VfsDeltaStore>>,
    request_counter: AtomicI64,
    builds_started: AtomicI64,
}

impl Clone for DagExecutorServiceImpl {
    fn clone(&self) -> Self {
        Self {
            builds: Arc::clone(&self.builds),
            scheduler: Arc::clone(&self.scheduler),
            task_graph: Arc::clone(&self.task_graph),
            execution_plan: Arc::clone(&self.execution_plan),
            dispatchers: self.dispatchers.clone(),
            executor_registry: Arc::clone(&self.executor_registry),
            jvm_host_bridge: self.jvm_host_bridge.clone(),
            scope_registry: self.scope_registry.clone(),
            local_cache: self.local_cache.clone(),
            vfs_delta_store: self.vfs_delta_store.clone(),
            request_counter: AtomicI64::new(self.request_counter.load(Ordering::Relaxed)),
            builds_started: AtomicI64::new(self.builds_started.load(Ordering::Relaxed)),
        }
    }
}

impl Default for DagExecutorServiceImpl {
    fn default() -> Self {
        Self::new(
            Arc::new(WorkerScheduler::new(16)),
            Arc::new(super::task_graph::TaskGraphServiceImpl::new()),
            Arc::new(super::execution_plan::ExecutionPlanServiceImpl::default()),
            Vec::new(),
        )
    }
}

impl DagExecutorServiceImpl {
    pub fn new(
        scheduler: Arc<WorkerScheduler>,
        task_graph: Arc<super::task_graph::TaskGraphServiceImpl>,
        execution_plan: Arc<super::execution_plan::ExecutionPlanServiceImpl>,
        dispatchers: Vec<Arc<dyn EventDispatcher>>,
    ) -> Self {
        Self {
            builds: Arc::new(DashMap::new()),
            scheduler,
            task_graph,
            execution_plan,
            dispatchers,
            executor_registry: Arc::new(TaskExecutorRegistry::new()),
            jvm_host_bridge: None,
            scope_registry: None,
            local_cache: None,
            vfs_delta_store: None,
            request_counter: AtomicI64::new(0),
            builds_started: AtomicI64::new(0),
        }
    }

    pub fn with_jvm_host_bridge(mut self, bridge: SharedJvmHostBridge) -> Self {
        self.jvm_host_bridge = Some(bridge);
        self
    }

    pub fn with_scope_registry(mut self, registry: Arc<ScopeRegistry>) -> Self {
        self.scope_registry = Some(registry);
        self
    }

    pub fn with_local_cache(mut self, local_cache: Arc<LocalCacheStore>) -> Self {
        self.local_cache = Some(local_cache);
        self
    }

    pub fn with_vfs_delta_store(mut self, vfs_delta_store: Arc<VfsDeltaStore>) -> Self {
        self.vfs_delta_store = Some(vfs_delta_store);
        self
    }

    fn merge_daemon_vfs_delta_context(&self, context_json: Option<String>) -> Option<String> {
        if context_has_trusted_vfs_delta(context_json.as_ref()) {
            return context_json;
        }
        let Some(store) = &self.vfs_delta_store else {
            return context_json;
        };

        let since_ms = context_vfs_delta_since_ms(context_json.as_ref());
        let changed_paths = store.changed_paths_since(since_ms);
        if changed_paths.is_empty() {
            return context_json;
        }

        let delta_context = serde_json::json!({
            "trusted_vfs_delta": true,
            "trusted_changed_paths": changed_paths,
        })
        .to_string();
        merged_task_context(Some(&delta_context), context_json)
    }

    /// Dispatch an event to all registered dispatchers.
    fn dispatch_event(&self, event: &BuildEventMessage) {
        for dispatcher in &self.dispatchers {
            dispatcher.dispatch_event(event);
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// Check if a task passes the filter.
    fn passes_filter(task_path: &str, filter: &Option<HashSet<String>>) -> bool {
        match filter {
            None => true,
            Some(f) => f.contains(task_path),
        }
    }

    fn compute_critical_path_remaining(
        nodes: &[crate::proto::ExecutionNode],
        task_filter: &Option<HashSet<String>>,
    ) -> HashMap<String, i64> {
        let mut dependents: HashMap<String, Vec<String>> = HashMap::with_capacity(nodes.len());
        let mut estimates: HashMap<String, i64> = HashMap::with_capacity(nodes.len());

        for node in nodes {
            if !Self::passes_filter(&node.task_path, task_filter) {
                continue;
            }
            estimates.insert(node.task_path.clone(), node.estimated_duration_ms.max(0));
            for dep in node
                .dependencies
                .iter()
                .filter(|dep| Self::passes_filter(dep, task_filter))
            {
                dependents
                    .entry(dep.clone())
                    .or_default()
                    .push(node.task_path.clone());
            }
        }

        let mut remaining: HashMap<String, i64> = HashMap::with_capacity(estimates.len());
        for node in nodes.iter().rev() {
            if !Self::passes_filter(&node.task_path, task_filter) {
                continue;
            }
            let longest_dependent = dependents
                .get(&node.task_path)
                .into_iter()
                .flat_map(|deps| deps.iter())
                .filter_map(|dep| remaining.get(dep).copied())
                .max()
                .unwrap_or(0);
            let estimate = estimates.get(&node.task_path).copied().unwrap_or(0);
            remaining.insert(node.task_path.clone(), estimate + longest_dependent);
        }
        remaining
    }

    fn enqueue_ready_task(execution: &mut BuildExecution, task_path: String) {
        let critical_path_remaining_ms = execution
            .tasks
            .get(&task_path)
            .map(|slot| slot.critical_path_remaining_ms)
            .unwrap_or(0);
        let ready_task = ReadyTask {
            task_path,
            critical_path_remaining_ms,
            sequence: execution.ready_sequence,
        };
        execution.ready_sequence = execution.ready_sequence.saturating_add(1);
        execution
            .ready_queue
            .lock()
            .expect("ready_queue lock should not be poisoned")
            .push(ready_task);
    }

    /// Try to mark dependents as ready after a task finishes.
    /// Returns list of newly ready task paths.
    fn try_unblock_dependents(execution: &mut BuildExecution, finished_task: &str) -> Vec<String> {
        let dep_count = execution
            .dependents
            .get(finished_task)
            .map_or(0, |d| d.len());
        let mut newly_ready = Vec::with_capacity(dep_count);
        if let Some(deps) = execution.dependents.get(finished_task) {
            for dependent in deps {
                if let Some(slot) = execution.tasks.get(dependent) {
                    if slot.status != "PENDING" {
                        continue;
                    }
                    // Check if ALL dependencies are satisfied
                    let all_deps_met = slot.dependencies.iter().all(|dep| {
                        execution
                            .tasks
                            .get(dep)
                            .map(|d| {
                                matches!(d.status.as_str(), "SUCCEEDED" | "FAILED" | "SKIPPED")
                            })
                            .unwrap_or(true)
                    });
                    if all_deps_met {
                        newly_ready.push(dependent.clone());
                    }
                }
            }
        }
        // Update ready queue
        for task in &newly_ready {
            Self::enqueue_ready_task(execution, task.clone());
        }
        newly_ready
    }

    /// Transitively skip all dependents of a failed task (BFS).
    fn skip_transitive_dependents(execution: &mut BuildExecution, failed_task: &str) {
        let mut to_visit = VecDeque::new();
        if let Some(deps) = execution.dependents.get(failed_task) {
            for d in deps {
                to_visit.push_back(d.clone());
            }
        }
        let mut visited = HashSet::with_capacity(to_visit.len() * 2);
        while let Some(task_path) = to_visit.pop_front() {
            if visited.contains(&task_path) {
                continue;
            }
            visited.insert(task_path.clone());
            if let Some(slot) = execution.tasks.get_mut(&task_path) {
                if slot.status == "PENDING" {
                    slot.status = "SKIPPED".to_string();
                    // Remove from ready queue if present
                    if let Ok(mut queue) = execution.ready_queue.lock() {
                        let retained: BinaryHeap<_> = queue
                            .drain()
                            .filter(|ready| ready.task_path != task_path)
                            .collect();
                        *queue = retained;
                    }
                    // Continue BFS to dependents of this task
                    if let Some(next_deps) = execution.dependents.get(&task_path) {
                        for d in next_deps {
                            to_visit.push_back(d.clone());
                        }
                    }
                }
            }
        }
    }

    /// Check if a build is complete and update its status accordingly.
    fn check_build_completion(execution: &mut BuildExecution) -> bool {
        let completed = execution.completed_count();
        if completed >= execution.total_tasks && execution.total_tasks > 0 {
            let failed = execution.failed_count();
            execution.status = if failed > 0 {
                "FAILED".to_string()
            } else {
                "COMPLETED".to_string()
            };
            execution.completion_notify.notify_waiters();
            true
        } else {
            false
        }
    }

    /// Get the current task statuses for a build.
    fn get_task_statuses(execution: &BuildExecution) -> Vec<TaskStatusEntry> {
        let mut result = Vec::with_capacity(execution.tasks.len());
        for t in execution.tasks.values() {
            result.push(TaskStatusEntry {
                task_path: t.task_path.clone(),
                status: t.status.clone(),
                duration_ms: t.duration_ms,
            });
        }
        result
    }

    fn task_execution_context(&self, build_id: &str, task_path: &str) -> Option<String> {
        self.builds
            .get(&BuildId::from(build_id.to_string()))
            .and_then(|execution| {
                execution
                    .tasks
                    .get(task_path)
                    .map(|slot| slot.execution_context_json.clone())
            })
            .filter(|context| !context.is_empty())
    }

    async fn restore_outputs_from_cache(
        &self,
        cache_key: &str,
        context_json: Option<&String>,
    ) -> Result<bool, String> {
        let Some(cache) = &self.local_cache else {
            return Ok(false);
        };
        if cache_key.is_empty() {
            return Ok(false);
        }
        let output_paths = context_output_paths(context_json);
        if output_paths.is_empty() {
            return Ok(false);
        }

        let Some(packaged_bytes) = cache
            .load(cache_key)
            .await
            .map_err(|error| format!("load cache entry {cache_key}: {error}"))?
        else {
            return Ok(false);
        };

        let (files, metadata, _entry_count) =
            BuildCachePackagingServiceImpl::unpack(UnpackCacheEntryRequest {
                build_id: String::new(),
                packaged_bytes,
                gzip: true,
            })?;
        let output_kinds = output_kinds_from_metadata(&metadata, output_paths.len());

        for output in &output_paths {
            match tokio::fs::metadata(output).await {
                Ok(metadata) if metadata.is_dir() => {
                    tokio::fs::remove_dir_all(output).await.map_err(|error| {
                        format!("remove cached output dir {}: {error}", output.display())
                    })?
                }
                Ok(_) => tokio::fs::remove_file(output).await.map_err(|error| {
                    format!("remove cached output file {}: {error}", output.display())
                })?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "inspect cached output {}: {error}",
                        output.display()
                    ));
                }
            }
        }

        for (index, output) in output_paths.iter().enumerate() {
            if output_kinds.get(index).and_then(|kind| kind.as_deref()) == Some("dir") {
                tokio::fs::create_dir_all(output).await.map_err(|error| {
                    format!("create cached output dir {}: {error}", output.display())
                })?;
            } else if let Some(parent) = output.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|error| {
                    format!("create cached output parent {}: {error}", parent.display())
                })?;
            }
        }

        for file in files {
            let normalized = file.path.replace('\\', "/");
            let Some((prefix, relative)) = normalized.split_once('/') else {
                let Some(index) = normalized
                    .strip_prefix("out")
                    .and_then(|value| value.parse::<usize>().ok())
                else {
                    continue;
                };
                let Some(target) = output_paths.get(index) else {
                    continue;
                };
                if let Some(parent) = target.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|error| {
                        format!("create cached file parent {}: {error}", parent.display())
                    })?;
                }
                tokio::fs::write(target, &file.content)
                    .await
                    .map_err(|error| {
                        format!("write cached output {}: {error}", target.display())
                    })?;
                apply_cached_executable_bit(target, file.executable).await?;
                continue;
            };
            let Some(index) = prefix
                .strip_prefix("out")
                .and_then(|value| value.parse::<usize>().ok())
            else {
                continue;
            };
            let Some(root) = output_paths.get(index) else {
                continue;
            };
            let target = root.join(relative);
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|error| {
                    format!("create cached file parent {}: {error}", parent.display())
                })?;
            }
            tokio::fs::write(&target, &file.content)
                .await
                .map_err(|error| format!("write cached output {}: {error}", target.display()))?;
            apply_cached_executable_bit(&target, file.executable).await?;
        }

        Ok(true)
    }

    async fn store_outputs_in_cache(
        &self,
        cache_key: &str,
        context_json: Option<&String>,
    ) -> Result<bool, String> {
        let Some(cache) = &self.local_cache else {
            return Ok(false);
        };
        if cache_key.is_empty() {
            return Ok(false);
        }
        let output_paths = context_output_paths(context_json);
        if output_paths.is_empty() {
            return Ok(false);
        }

        let mut files = Vec::new();
        let mut output_kinds = Vec::with_capacity(output_paths.len());
        for (index, output) in output_paths.iter().enumerate() {
            let kind = collect_output_cache_files(output, &format!("out{index}"), &mut files)?;
            output_kinds.push(kind);
        }

        let mut origin_metadata = HashMap::new();
        origin_metadata.insert(
            "output_kinds_json".to_string(),
            serde_json::to_string(&output_kinds).unwrap_or_default(),
        );
        let (packaged_bytes, _entry_count) =
            BuildCachePackagingServiceImpl::pack(PackCacheEntryRequest {
                build_id: String::new(),
                files,
                origin_metadata,
                gzip: true,
            })?;
        cache
            .store(cache_key, &packaged_bytes)
            .await
            .map_err(|error| format!("store cache entry {cache_key}: {error}"))?;
        Ok(true)
    }

    fn build_kernel_plan(
        &self,
        build_id: &str,
        task_contexts: &HashMap<String, String>,
        plan_dependencies: &[crate::proto::BuildPlanDependency],
    ) -> Result<KernelBuildPlan, String> {
        let build_id_key = BuildId::from(build_id.to_string());
        let execution = self
            .builds
            .get(&build_id_key)
            .ok_or_else(|| format!("Build '{}' has no materialized Rust task graph", build_id))?;

        let mut tasks = execution
            .tasks
            .values()
            .map(|slot| {
                let execution_context_json =
                    task_contexts.get(&slot.task_path).cloned().or_else(|| {
                        if slot.execution_context_json.is_empty() {
                            None
                        } else {
                            Some(slot.execution_context_json.clone())
                        }
                    });
                KernelTaskPlan {
                    task_path: slot.task_path.clone(),
                    task_type: slot.task_type.clone(),
                    dependencies: slot.dependencies.clone(),
                    execution_context_json,
                }
            })
            .collect::<Vec<_>>();
        tasks.sort_by(|a, b| a.task_path.cmp(&b.task_path));

        Ok(KernelBuildPlan {
            build_id: build_id.to_string(),
            tasks,
            dependency_graph: kernel_dependency_graph_from_plan_dependencies(plan_dependencies),
        })
    }
}

fn kernel_plan_from_execution_nodes(
    build_id: &str,
    execution_order: &[crate::proto::ExecutionNode],
    plan_dependencies: &[crate::proto::BuildPlanDependency],
) -> KernelBuildPlan {
    let mut tasks = execution_order
        .iter()
        .map(|node| KernelTaskPlan {
            task_path: node.task_path.clone(),
            task_type: node.task_type.clone(),
            dependencies: node.dependencies.clone(),
            execution_context_json: if node.execution_context_json.is_empty() {
                None
            } else {
                Some(node.execution_context_json.clone())
            },
        })
        .collect::<Vec<_>>();
    tasks.sort_by(|a, b| a.task_path.cmp(&b.task_path));
    KernelBuildPlan {
        build_id: build_id.to_string(),
        tasks,
        dependency_graph: kernel_dependency_graph_from_plan_dependencies(plan_dependencies),
    }
}

fn kernel_dependency_graph_from_plan_dependencies(
    plan_dependencies: &[crate::proto::BuildPlanDependency],
) -> Option<KernelDependencyGraph> {
    if plan_dependencies.is_empty() {
        return None;
    }
    let mut configurations = HashMap::<String, KernelDependencyConfiguration>::new();
    for dependency in plan_dependencies {
        let configuration_name =
            format!("{}:{}", dependency.project_path, dependency.configuration);
        let configuration = configurations
            .entry(configuration_name.clone())
            .or_insert_with(|| KernelDependencyConfiguration {
                name: configuration_name,
                repositories: Vec::new(),
                dependencies: Vec::new(),
                project_dependencies: Vec::new(),
                constraints: Vec::new(),
                unsupported_features: Vec::new(),
            });
        let dependency_kind = if dependency.kind.trim().is_empty() {
            "dependency"
        } else {
            dependency.kind.trim()
        };
        for repository in &dependency.repositories {
            if !repository.credentials.is_empty() {
                let feature = format!("repository-credentials:{}", repository.id);
                if !configuration
                    .unsupported_features
                    .iter()
                    .any(|existing| existing == &feature)
                {
                    configuration.unsupported_features.push(feature);
                }
            }
            let kernel_repository = KernelRepository {
                id: repository.id.clone(),
                url: repository.url.clone(),
                allow_insecure_protocol: repository.allow_insecure_protocol,
            };
            if !configuration.repositories.contains(&kernel_repository) {
                configuration.repositories.push(kernel_repository);
            }
        }
        for feature in &dependency.unsupported_features {
            let feature = feature.trim();
            if !feature.is_empty()
                && !configuration
                    .unsupported_features
                    .iter()
                    .any(|existing| existing == feature)
            {
                configuration.unsupported_features.push(feature.to_string());
            }
        }
        if dependency_kind != "dependency" && dependency_kind != "constraint" {
            configuration.unsupported_features.push(format!(
                "unsupported dependency kind '{}' for '{}'",
                dependency.kind, dependency.notation
            ));
            continue;
        }
        if let Some(request) = kernel_dependency_request_from_notation(&dependency.notation) {
            if dependency_kind == "constraint" {
                configuration.constraints.push(request);
            } else {
                configuration.dependencies.push(request);
            }
        } else if let Some(project_path) =
            project_dependency_path_from_notation(&dependency.notation)
        {
            configuration.project_dependencies.push(project_path);
        } else {
            configuration.unsupported_features.push(format!(
                "unsupported dependency notation '{}'",
                dependency.notation
            ));
        }
    }
    let mut configurations = configurations.into_values().collect::<Vec<_>>();
    configurations.sort_by(|a, b| a.name.cmp(&b.name));
    Some(KernelDependencyGraph { configurations })
}

fn unsupported_plan_dependency_reason(
    plan_dependencies: &[crate::proto::BuildPlanDependency],
) -> Option<String> {
    for dependency in plan_dependencies {
        for feature in &dependency.unsupported_features {
            let feature = feature.trim();
            if !feature.is_empty() {
                let configuration =
                    format!("{}:{}", dependency.project_path, dependency.configuration);
                return Some(
                    crate::server::execution_kernel::unsupported_dependency_feature_reason(
                        &configuration,
                        feature,
                    ),
                );
            }
        }
    }
    None
}

fn project_dependency_path_from_notation(notation: &str) -> Option<String> {
    let trimmed = notation.trim();
    if let Some(rest) = trimmed.strip_prefix("project ") {
        return normalize_project_dependency_path(rest);
    }
    if let Some(rest) = trimmed
        .strip_prefix("project(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return normalize_project_dependency_path(rest);
    }
    None
}

fn normalize_project_dependency_path(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches('"').trim_matches('\'').trim();
    if trimmed.starts_with(':') {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn kernel_dependency_request_from_notation(notation: &str) -> Option<KernelDependencyRequest> {
    let selector = graph_builder::parse_module_selector_notation(notation)?;
    Some(KernelDependencyRequest {
        group: selector.group,
        name: selector.name,
        version: selector.version,
    })
}

#[tonic::async_trait]
impl DagExecutorService for DagExecutorServiceImpl {
    async fn start_build(
        &self,
        request: Request<StartBuildRequest>,
    ) -> Result<Response<StartBuildResponse>, Status> {
        let req = request.into_inner();
        self.request_counter.fetch_add(1, Ordering::Relaxed);

        let build_id = BuildId::from(req.build_id.clone());

        // Ensure the build was registered by BootstrapService. This keeps the
        // DAG executor inside an explicit build/session scope instead of
        // silently creating untracked synthetic scopes.
        if let Some(ref registry) = self.scope_registry {
            if registry.session_for_build(&build_id).is_none() {
                return Err(Status::not_found(format!(
                    "Build '{}' is not registered in any session",
                    req.build_id
                )));
            }
        }

        // Resolve execution plan from TaskGraphService
        let plan_response = self
            .task_graph
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: req.build_id.clone(),
                prefer_build_plan_shadow: true,
            }))
            .await
            .map_err(|e| Status::internal(format!("Failed to resolve execution plan: {}", e)))?
            .into_inner();

        if let Some(reason) = unsupported_plan_dependency_reason(&plan_response.plan_dependencies) {
            return Ok(Response::new(StartBuildResponse {
                accepted: false,
                error_message: format!(
                    "Rust execution kernel rejected build '{}' before execution: {}",
                    req.build_id, reason
                ),
                total_tasks: 0,
                critical_path_ms: 0,
                plan_source: plan_response.plan_source,
                plan_dependencies: plan_response.plan_dependencies,
            }));
        }

        let kernel_plan = kernel_plan_from_execution_nodes(
            &req.build_id,
            &plan_response.execution_order,
            &plan_response.plan_dependencies,
        );
        let native_executor_types = plan_response
            .execution_order
            .iter()
            .map(|node| node.task_type.clone())
            .collect();
        if let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&kernel_plan, &native_executor_types)
        {
            return Ok(Response::new(StartBuildResponse {
                accepted: false,
                error_message: rejection.message(),
                total_tasks: plan_response.total_tasks,
                critical_path_ms: plan_response.critical_path_ms,
                plan_source: plan_response.plan_source,
                plan_dependencies: plan_response.plan_dependencies,
            }));
        }

        if plan_response.has_cycles {
            return Ok(Response::new(StartBuildResponse {
                accepted: false,
                error_message: "Task graph contains cycles".to_string(),
                total_tasks: 0,
                critical_path_ms: 0,
                plan_source: plan_response.plan_source,
                plan_dependencies: plan_response.plan_dependencies,
            }));
        }

        let task_filter: Option<HashSet<String>> = if req.task_filter.is_empty() {
            None
        } else {
            Some(req.task_filter.into_iter().collect())
        };

        // Build task slots and dependents map
        let task_count = plan_response.execution_order.len();
        let critical_path_remaining =
            Self::compute_critical_path_remaining(&plan_response.execution_order, &task_filter);
        let mut tasks = HashMap::with_capacity(task_count);
        let mut dependents: HashMap<String, Vec<String>> = HashMap::with_capacity(task_count);
        let ready_queue = BinaryHeap::new();

        for node in &plan_response.execution_order {
            if !Self::passes_filter(&node.task_path, &task_filter) {
                continue;
            }

            let deps: Vec<String> = node
                .dependencies
                .iter()
                .filter(|d| Self::passes_filter(d, &task_filter))
                .cloned()
                .collect();

            // Build reverse adjacency
            for dep in &deps {
                dependents
                    .entry(dep.clone())
                    .or_default()
                    .push(node.task_path.clone());
            }

            tasks.insert(
                node.task_path.clone(),
                TaskSlot {
                    task_path: node.task_path.clone(),
                    task_type: node.task_type.clone(),
                    status: "PENDING".to_string(),
                    start_time_ms: 0,
                    duration_ms: 0,
                    dependencies: deps,
                    work_metadata: None,
                    predicted_outcome: PredictedOutcome::PredictedUnknown as i32,
                    input_fingerprint: String::new(),
                    execution_context_json: node.execution_context_json.clone(),
                    estimated_duration_ms: node.estimated_duration_ms.max(0),
                    critical_path_remaining_ms: critical_path_remaining
                        .get(&node.task_path)
                        .copied()
                        .unwrap_or(node.estimated_duration_ms.max(0)),
                },
            );
        }

        let total_tasks = tasks.len() as i32;

        // If no tasks pass the filter
        if total_tasks == 0 {
            return Ok(Response::new(StartBuildResponse {
                accepted: false,
                error_message: "No tasks to execute".to_string(),
                total_tasks: 0,
                critical_path_ms: 0,
                plan_source: plan_response.plan_source,
                plan_dependencies: plan_response.plan_dependencies,
            }));
        }

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        let mut execution = BuildExecution {
            build_id: build_id.clone(),
            status: "EXECUTING".to_string(),
            start_time_ms: Self::now_ms(),
            ready_queue: std::sync::Mutex::new(ready_queue),
            ready_sequence: 0,
            executing: std::sync::Mutex::new(HashSet::new()),
            dependents,
            tasks,
            task_filter,
            total_tasks,
            max_parallelism: if req.max_parallelism > 0 {
                req.max_parallelism as usize
            } else {
                16
            },
            completion_notify: Arc::new(tokio::sync::Notify::new()),
            cancel_rx,
            cancel_tx,
            failure_message: String::new(),
        };

        let initial_ready: Vec<String> = execution
            .tasks
            .values()
            .filter(|slot| slot.dependencies.is_empty())
            .map(|slot| slot.task_path.clone())
            .collect();
        for task_path in initial_ready {
            Self::enqueue_ready_task(&mut execution, task_path);
        }

        self.builds.insert(build_id.clone(), execution);
        self.builds_started.fetch_add(1, Ordering::Relaxed);

        // Dispatch build_start event
        self.dispatch_event(&BuildEventMessage {
            build_id: req.build_id.clone(),
            timestamp_ms: Self::now_ms(),
            event_type: "build_start".to_string(),
            event_id: format!("dag-build-start-{}", req.build_id),
            properties: Default::default(),
            display_name: "Build".to_string(),
            parent_id: String::new(),
        });

        tracing::info!(
            build_id = %req.build_id,
            total_tasks = total_tasks,
            critical_path_ms = plan_response.critical_path_ms,
            ready_tasks = plan_response.ready_to_execute,
            plan_source = %plan_response.plan_source,
            "Build execution started"
        );

        Ok(Response::new(StartBuildResponse {
            accepted: true,
            error_message: String::new(),
            total_tasks,
            critical_path_ms: plan_response.critical_path_ms,
            plan_source: plan_response.plan_source,
            plan_dependencies: plan_response.plan_dependencies,
        }))
    }

    async fn run_build(
        &self,
        request: Request<RunBuildRequest>,
    ) -> Result<Response<RunBuildResponse>, Status> {
        let req = request.into_inner();
        let build_id_str = req.build_id.clone();
        let start_time = now_ms();

        // Phase 1: Start the build (sets up the execution plan and ready queue).
        let start_resp = self
            .start_build(Request::new(StartBuildRequest {
                build_id: build_id_str.clone(),
                max_parallelism: req.max_parallelism,
                task_filter: req.task_filter.clone(),
            }))
            .await?;

        if !start_resp.get_ref().accepted {
            let plan_source = start_resp.get_ref().plan_source.clone();
            return Ok(Response::new(RunBuildResponse {
                build_id: build_id_str.clone(),
                final_status: "FAILED".to_string(),
                total_tasks: 0,
                tasks_succeeded: 0,
                tasks_failed: 0,
                tasks_skipped: 0,
                tasks_forwarded_to_jvm: 0,
                total_duration_ms: now_ms() - start_time,
                failure_message: start_resp.get_ref().error_message.clone(),
                task_details: vec![],
                tasks_up_to_date: 0,
                tasks_from_cache: 0,
                plan_source,
            }));
        }

        let total_tasks = start_resp.get_ref().total_tasks;
        let plan_source = start_resp.get_ref().plan_source.clone();
        let plan_dependencies = start_resp.get_ref().plan_dependencies.clone();
        let max_parallelism = req.max_parallelism.max(1) as usize;
        let allow_jvm_forwarding = req.allow_jvm_forwarding;
        let task_contexts = req.task_contexts;

        if !allow_jvm_forwarding {
            let kernel_plan =
                match self.build_kernel_plan(&build_id_str, &task_contexts, &plan_dependencies) {
                    Ok(plan) => plan,
                    Err(error) => {
                        return Ok(Response::new(RunBuildResponse {
                            build_id: build_id_str.clone(),
                            final_status: "FAILED".to_string(),
                            total_tasks,
                            tasks_succeeded: 0,
                            tasks_failed: total_tasks,
                            tasks_skipped: 0,
                            tasks_forwarded_to_jvm: 0,
                            total_duration_ms: now_ms() - start_time,
                            failure_message: error,
                            task_details: vec![],
                            tasks_up_to_date: 0,
                            tasks_from_cache: 0,
                            plan_source,
                        }));
                    }
                };
            let native_executor_types = self
                .executor_registry
                .registered_types()
                .into_iter()
                .map(str::to_string)
                .collect();
            if let KernelAdmission::Rejected(rejection) =
                admit_build_plan(&kernel_plan, &native_executor_types)
            {
                return Ok(Response::new(RunBuildResponse {
                    build_id: build_id_str.clone(),
                    final_status: "FAILED".to_string(),
                    total_tasks,
                    tasks_succeeded: 0,
                    tasks_failed: total_tasks,
                    tasks_skipped: 0,
                    tasks_forwarded_to_jvm: 0,
                    total_duration_ms: now_ms() - start_time,
                    failure_message: rejection.message(),
                    task_details: vec![],
                    tasks_up_to_date: 0,
                    tasks_from_cache: 0,
                    plan_source,
                }));
            }
        }

        // Channel for task results (spawned tasks send back, main loop processes).
        let (result_tx, mut result_rx) =
            tokio::sync::mpsc::channel::<TaskExecResult>(total_tasks as usize);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_parallelism));
        let mut in_flight: usize = 0;
        let mut tasks_dispatched: usize = 0;
        let mut tasks_completed: usize = 0;
        let mut task_details: Vec<TaskExecutionDetail> = Vec::with_capacity(total_tasks as usize);
        let mut jvm_forward_count: i32 = 0;
        let mut up_to_date_count: i32 = 0;
        let mut from_cache_count: i32 = 0;
        let mut failure_message = String::new();
        let mut build_failed = false;

        loop {
            // Dispatch tasks while we have capacity and tasks are available.
            while in_flight < max_parallelism && !build_failed {
                let next = self
                    .get_next_task(Request::new(GetNextTaskRequest {
                        build_id: build_id_str.clone(),
                    }))
                    .await?
                    .into_inner();

                if next.task_path == BUILD_COMPLETE_SENTINEL {
                    build_failed = true;
                    break;
                }

                if next.task_path.is_empty() {
                    // Throttled — no ready tasks but in-flight work exists.
                    break;
                }

                let task_path = next.task_path.clone();
                let task_type = next.task_type.clone();

                // Phase 2a: Check execution plan for UP-TO-DATE / FROM_CACHE.
                let context_json = self.merge_daemon_vfs_delta_context(merged_task_context(
                    task_contexts.get(&task_path),
                    self.task_execution_context(&build_id_str, &task_path),
                ));
                if context_is_no_source(context_json.as_ref()) {
                    self.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                        build_id: build_id_str.clone(),
                        task_path: task_path.clone(),
                        success: true,
                        outcome: "NO_SOURCE".to_string(),
                        duration_ms: 0,
                        failure_message: String::new(),
                    }))
                    .await?;

                    tasks_completed += 1;
                    task_details.push(TaskExecutionDetail {
                        task_path,
                        task_type,
                        outcome: "NO_SOURCE".to_string(),
                        duration_ms: 0,
                        execution_mode: "skipped".to_string(),
                        error_message: "No source files for Rust-native task".to_string(),
                    });
                    continue;
                }
                let outputs_present = declared_outputs_present(&task_type, context_json.as_ref());
                let work_meta = context_json.as_ref().and_then(|json| {
                    serde_json::from_str::<serde_json::Value>(json)
                        .ok()
                        .and_then(|v| {
                            let mut meta = WorkMetadata {
                                work_identity: v.get("work_identity")?.as_str()?.to_string(),
                                display_name: v
                                    .get("display_name")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or(&task_path)
                                    .to_string(),
                                implementation_class: v
                                    .get("implementation_class")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or(&task_type)
                                    .to_string(),
                                input_properties: v
                                    .get("input_properties")
                                    .and_then(|p| p.as_object())
                                    .map(|obj| {
                                        obj.iter()
                                            .filter_map(|(k, val)| {
                                                val.as_str().map(|s| (k.clone(), s.to_string()))
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                                input_file_fingerprints: v
                                    .get("input_file_fingerprints")
                                    .and_then(|f| f.as_object())
                                    .map(|obj| {
                                        obj.iter()
                                            .filter_map(|(k, val)| {
                                                val.as_str().map(|s| (k.clone(), s.to_string()))
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                                caching_enabled: v
                                    .get("caching_enabled")
                                    .and_then(|c| c.as_bool())
                                    .unwrap_or(false),
                                can_load_from_cache: v
                                    .get("can_load_from_cache")
                                    .and_then(|c| c.as_bool())
                                    .unwrap_or(false),
                                has_previous_execution_state: v
                                    .get("has_previous_execution_state")
                                    .and_then(|c| c.as_bool())
                                    .unwrap_or(false),
                                rebuild_reasons: v
                                    .get("rebuild_reasons")
                                    .and_then(|r| r.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|val| val.as_str().map(|s| s.to_string()))
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                            };
                            apply_vfs_delta_to_work_metadata(&mut meta, context_json.as_ref());
                            Some(meta)
                        })
                });

                if let Some(ref meta) = work_meta {
                    // Store work_metadata on the task slot for later outcome recording.
                    let build_id_clone = build_id_str.clone();
                    if let Some(mut execution) = self.builds.get_mut(&BuildId::from(build_id_clone))
                    {
                        if let Some(slot) = execution.tasks.get_mut(&task_path) {
                            slot.work_metadata = Some(meta.clone());
                            slot.input_fingerprint =
                                super::execution_plan::ExecutionPlanServiceImpl::compute_fingerprint(
                                    meta,
                                );
                        }
                    }

                    let plan_resp = self
                        .execution_plan
                        .resolve_plan(Request::new(ResolvePlanRequest {
                            work: Some(meta.clone()),
                            authoritative: self.local_cache.is_none(),
                        }))
                        .await?
                        .into_inner();

                    let action = crate::proto::PlanAction::try_from(plan_resp.action)
                        .unwrap_or(crate::proto::PlanAction::Unknown);
                    let predicted_outcome = match action {
                        crate::proto::PlanAction::Execute => {
                            PredictedOutcome::PredictedExecute as i32
                        }
                        crate::proto::PlanAction::SkipUpToDate => {
                            PredictedOutcome::PredictedUpToDate as i32
                        }
                        crate::proto::PlanAction::LoadFromCache => {
                            PredictedOutcome::PredictedFromCache as i32
                        }
                        crate::proto::PlanAction::ShortCircuit => {
                            PredictedOutcome::PredictedShortCircuited as i32
                        }
                        crate::proto::PlanAction::Unknown => {
                            PredictedOutcome::PredictedUnknown as i32
                        }
                    };
                    if let Some(mut execution) =
                        self.builds.get_mut(&BuildId::from(build_id_str.clone()))
                    {
                        if let Some(slot) = execution.tasks.get_mut(&task_path) {
                            slot.predicted_outcome = predicted_outcome;
                        }
                    }
                    let up_to_date_enabled = context_allows_up_to_date(context_json.as_ref());

                    match action {
                        crate::proto::PlanAction::SkipUpToDate
                            if outputs_present && up_to_date_enabled =>
                        {
                            // Mark task as UP-TO-DATE without executing.
                            self.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                                build_id: build_id_str.clone(),
                                task_path: task_path.clone(),
                                success: true,
                                outcome: "UP_TO_DATE".to_string(),
                                duration_ms: 0,
                                failure_message: String::new(),
                            }))
                            .await?;

                            // Record outcome to execution plan.
                            let fp =
                                super::execution_plan::ExecutionPlanServiceImpl::compute_fingerprint(
                                    meta,
                                );
                            let _ = self
                                .execution_plan
                                .record_outcome(Request::new(RecordOutcomeRequest {
                                    work_identity: meta.work_identity.clone(),
                                    predicted_outcome: PredictedOutcome::PredictedUpToDate as i32,
                                    actual_outcome: "UP_TO_DATE".to_string(),
                                    prediction_correct: true,
                                    duration_ms: 0,
                                    input_fingerprint: fp,
                                }))
                                .await;

                            up_to_date_count += 1;
                            tasks_completed += 1;
                            task_details.push(TaskExecutionDetail {
                                task_path,
                                task_type,
                                outcome: "UP_TO_DATE".to_string(),
                                duration_ms: 0,
                                execution_mode: "skipped".to_string(),
                                error_message: plan_resp.reasoning,
                            });
                            continue;
                        }
                        crate::proto::PlanAction::LoadFromCache => {
                            match self
                                .restore_outputs_from_cache(
                                    &plan_resp.cache_key_hint,
                                    context_json.as_ref(),
                                )
                                .await
                            {
                                Ok(true) => {
                                    self.notify_task_finished(Request::new(
                                        NotifyTaskFinishedRequest {
                                            build_id: build_id_str.clone(),
                                            task_path: task_path.clone(),
                                            success: true,
                                            outcome: "FROM_CACHE".to_string(),
                                            duration_ms: 0,
                                            failure_message: String::new(),
                                        },
                                    ))
                                    .await?;

                                    let fp = super::execution_plan::ExecutionPlanServiceImpl::compute_fingerprint(meta);
                                    let _ = self
                                        .execution_plan
                                        .record_outcome(Request::new(RecordOutcomeRequest {
                                            work_identity: meta.work_identity.clone(),
                                            predicted_outcome: PredictedOutcome::PredictedFromCache
                                                as i32,
                                            actual_outcome: "FROM_CACHE".to_string(),
                                            prediction_correct: true,
                                            duration_ms: 0,
                                            input_fingerprint: fp,
                                        }))
                                        .await;

                                    from_cache_count += 1;
                                    tasks_completed += 1;
                                    task_details.push(TaskExecutionDetail {
                                        task_path,
                                        task_type,
                                        outcome: "FROM_CACHE".to_string(),
                                        duration_ms: 0,
                                        execution_mode: "cached".to_string(),
                                        error_message: plan_resp.reasoning,
                                    });
                                    continue;
                                }
                                Ok(false) => {
                                    tracing::debug!(
                                        task = %task_path,
                                        cache_key = %plan_resp.cache_key_hint,
                                        "Cache candidate missed; executing task"
                                    );
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        task = %task_path,
                                        cache_key = %plan_resp.cache_key_hint,
                                        error = %error,
                                        "Cache restore failed; executing task"
                                    );
                                }
                            }
                        }
                        _ => {
                            // EXECUTE or UNKNOWN — proceed with execution.
                        }
                    }
                }

                let registry = Arc::clone(&self.executor_registry);
                let context_for_task = self.merge_daemon_vfs_delta_context(merged_task_context(
                    task_contexts.get(&task_path),
                    self.task_execution_context(&build_id_str, &task_path),
                ));
                let tx = result_tx.clone();
                let allow_jvm_forwarding_for_task = allow_jvm_forwarding;
                let jvm_host_bridge = self.jvm_host_bridge.clone();
                let build_id_for_task = build_id_str.clone();
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| Status::internal("Semaphore closed during build execution"))?;

                tasks_dispatched += 1;
                in_flight += 1;

                // Notify started on the DAG executor (before spawning).
                self.notify_task_started(Request::new(NotifyTaskStartedRequest {
                    build_id: build_id_str.clone(),
                    task_path: task_path.clone(),
                    start_time_ms: now_ms(),
                }))
                .await?;

                tokio::spawn(async move {
                    let exec_start = now_ms();

                    let (success, outcome, exec_mode, error_msg) = if let Some(executor) =
                        registry.get(&task_type)
                    {
                        let input = build_task_input(&task_type, context_for_task.as_ref());
                        let result = executor.execute(&input).await;
                        (
                            result.success,
                            if result.success {
                                "EXECUTED".to_string()
                            } else {
                                "FAILED".to_string()
                            },
                            rust_execution_mode(&task_type).to_string(),
                            result.error_message,
                        )
                    } else if allow_jvm_forwarding_for_task {
                        if let Some(bridge) = jvm_host_bridge {
                            match bridge
                                .execute_task(
                                    &build_id_for_task,
                                    &task_path,
                                    &task_type,
                                    context_for_task.as_deref().unwrap_or("{}"),
                                    0,
                                )
                                .await
                            {
                                Ok(Some(response)) => (
                                    response.success,
                                    if response.outcome.is_empty() {
                                        if response.success {
                                            "EXECUTED".to_string()
                                        } else {
                                            "FAILED".to_string()
                                        }
                                    } else {
                                        response.outcome
                                    },
                                    if response.execution_mode.is_empty() {
                                        "jvm_host".to_string()
                                    } else {
                                        response.execution_mode
                                    },
                                    response.error_message,
                                ),
                                Ok(None) => (
                                    false,
                                    "FAILED".to_string(),
                                    "jvm_host_unavailable".to_string(),
                                    "JVM forwarding requested but JVM host is not connected"
                                        .to_string(),
                                ),
                                Err(status) => (
                                    false,
                                    "FAILED".to_string(),
                                    "jvm_host_error".to_string(),
                                    format!("JVM task execution RPC failed: {}", status),
                                ),
                            }
                        } else {
                            (
                                false,
                                "FAILED".to_string(),
                                "jvm_host_unavailable".to_string(),
                                "JVM forwarding requested but no JVM host bridge is configured"
                                    .to_string(),
                            )
                        }
                    } else {
                        (
                            false,
                            "FAILED".to_string(),
                            "missing_executor".to_string(),
                            format!(
                                "No native executor registered for task type {} and JVM forwarding is disabled",
                                task_type
                            ),
                        )
                    };

                    drop(permit);

                    let _ = tx
                        .send(TaskExecResult {
                            task_path,
                            task_type,
                            success,
                            outcome,
                            duration_ms: now_ms() - exec_start,
                            execution_mode: exec_mode,
                            error_message: error_msg,
                        })
                        .await;
                });
            }

            // All dispatched and no in-flight — we're done.
            if in_flight == 0 {
                break;
            }

            // Wait for at least one task to complete.
            if let Some(result) = result_rx.recv().await {
                in_flight -= 1;
                tasks_completed += 1;

                // Notify finished on the DAG executor (updates state, unblocks dependents).
                self.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                    build_id: build_id_str.clone(),
                    task_path: result.task_path.clone(),
                    success: result.success,
                    outcome: result.outcome.clone(),
                    duration_ms: result.duration_ms,
                    failure_message: result.error_message.clone(),
                }))
                .await?;

                // Record outcome to execution plan for executed tasks.
                let mut cache_store: Option<(String, String)> = None;
                {
                    let build_id_for_meta = build_id_str.clone();
                    let task_path_for_meta = result.task_path.clone();
                    let actual_outcome = result.outcome.clone();
                    let duration_for_record = result.duration_ms;

                    if let Some(execution) = self.builds.get(&BuildId::from(build_id_for_meta)) {
                        if let Some(slot) = execution.tasks.get(&task_path_for_meta) {
                            if let Some(ref meta) = slot.work_metadata {
                                let cache_context_json = task_contexts
                                    .get(&task_path_for_meta)
                                    .cloned()
                                    .filter(|context| !context.is_empty())
                                    .unwrap_or_else(|| slot.execution_context_json.clone());
                                let refreshed_meta =
                                    refreshed_work_metadata(meta, &slot.execution_context_json);
                                let predicted = slot.predicted_outcome;
                                let refreshed_fingerprint =
                                    super::execution_plan::ExecutionPlanServiceImpl::compute_fingerprint(
                                        &refreshed_meta,
                                    );
                                let prediction_correct = (predicted
                                    == PredictedOutcome::PredictedExecute as i32
                                    && actual_outcome == "EXECUTED")
                                    || (predicted == PredictedOutcome::PredictedUnknown as i32);

                                let _ = self
                                    .execution_plan
                                    .record_outcome(Request::new(RecordOutcomeRequest {
                                        work_identity: refreshed_meta.work_identity.clone(),
                                        predicted_outcome: predicted,
                                        actual_outcome: actual_outcome.clone(),
                                        prediction_correct,
                                        duration_ms: duration_for_record,
                                        input_fingerprint: refreshed_fingerprint,
                                    }))
                                    .await;

                                if result.success
                                    && actual_outcome == "EXECUTED"
                                    && !is_jvm_execution_mode(&result.execution_mode)
                                    && refreshed_meta.caching_enabled
                                {
                                    let cache_key =
                                        super::execution_plan::ExecutionPlanServiceImpl::compute_fingerprint(meta);
                                    cache_store = Some((cache_key, cache_context_json));
                                }
                            }
                        }
                    }
                }
                if let Some((cache_key, context_json)) = cache_store {
                    if let Err(error) = self
                        .store_outputs_in_cache(&cache_key, Some(&context_json))
                        .await
                    {
                        tracing::debug!(
                            task = %result.task_path,
                            cache_key = %cache_key,
                            error = %error,
                            "Skipping Rust cache store for executed task"
                        );
                    }
                }

                if is_jvm_execution_mode(&result.execution_mode) {
                    jvm_forward_count += 1;
                }
                if !result.success && failure_message.is_empty() {
                    failure_message =
                        format!("Task {} failed: {}", result.task_path, result.error_message);
                    build_failed = true;
                }

                task_details.push(TaskExecutionDetail {
                    task_path: result.task_path,
                    task_type: result.task_type,
                    outcome: result.outcome,
                    duration_ms: result.duration_ms,
                    execution_mode: result.execution_mode,
                    error_message: result.error_message,
                });
            }
        }

        // Phase 3: Read final build status.
        let final_status = self
            .get_build_status(Request::new(GetBuildStatusRequest {
                build_id: build_id_str.clone(),
            }))
            .await
            .map(|r| r.into_inner().status)
            .unwrap_or_else(|_| "FAILED".to_string());

        let detailed_paths: HashSet<String> = task_details
            .iter()
            .map(|detail| detail.task_path.clone())
            .collect();
        if let Some(execution) = self.builds.get(&BuildId::from(build_id_str.clone())) {
            let mut skipped_details: Vec<TaskExecutionDetail> = execution
                .tasks
                .values()
                .filter(|slot| {
                    slot.status == "SKIPPED" && !detailed_paths.contains(&slot.task_path)
                })
                .map(|slot| TaskExecutionDetail {
                    task_path: slot.task_path.clone(),
                    task_type: slot.task_type.clone(),
                    outcome: "SKIPPED".to_string(),
                    duration_ms: 0,
                    execution_mode: "skipped".to_string(),
                    error_message: if failure_message.is_empty() {
                        "Skipped by Rust DAG executor".to_string()
                    } else {
                        failure_message.clone()
                    },
                })
                .collect();
            skipped_details.sort_by(|a, b| a.task_path.cmp(&b.task_path));
            task_details.extend(skipped_details);
        }

        let total_duration = now_ms() - start_time;

        tracing::info!(
            build_id = %build_id_str,
            final_status = %final_status,
            total_tasks = total_tasks,
            dispatched = tasks_dispatched,
            completed = tasks_completed,
            jvm_forwarded = jvm_forward_count,
            up_to_date = up_to_date_count,
            from_cache = from_cache_count,
            plan_source = %plan_source,
            duration_ms = total_duration,
            "RunBuild completed"
        );

        Ok(Response::new(RunBuildResponse {
            build_id: build_id_str,
            final_status,
            total_tasks,
            tasks_succeeded: task_details
                .iter()
                .filter(|d| {
                    d.outcome == "EXECUTED"
                        || d.outcome == "UP_TO_DATE"
                        || d.outcome == "FROM_CACHE"
                        || d.outcome == "NO_SOURCE"
                })
                .count() as i32,
            tasks_failed: task_details
                .iter()
                .filter(|d| d.outcome == "FAILED")
                .count() as i32,
            tasks_skipped: task_details
                .iter()
                .filter(|d| d.outcome == "SKIPPED" || d.outcome == "NO_SOURCE")
                .count() as i32,
            tasks_forwarded_to_jvm: jvm_forward_count,
            total_duration_ms: total_duration,
            failure_message,
            task_details,
            tasks_up_to_date: up_to_date_count,
            tasks_from_cache: from_cache_count,
            plan_source,
        }))
    }

    async fn cancel_build(
        &self,
        request: Request<CancelBuildRequest>,
    ) -> Result<Response<CancelBuildResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id.clone());

        if let Some(mut execution) = self.builds.get_mut(&build_id) {
            if execution.is_terminal() {
                return Ok(Response::new(CancelBuildResponse { cancelled: false }));
            }

            // Send cancellation signal
            let _ = execution.cancel_tx.send(true);

            // Mark all pending tasks as SKIPPED
            for slot in execution.tasks.values_mut() {
                if slot.status == "PENDING" {
                    slot.status = "SKIPPED".to_string();
                }
            }

            // Clear ready queue
            if let Ok(mut queue) = execution.ready_queue.lock() {
                queue.clear();
            }

            execution.status = "CANCELLED".to_string();
            execution.completion_notify.notify_waiters();

            // Dispatch build_finish event
            self.dispatch_event(&BuildEventMessage {
                build_id: req.build_id.clone(),
                timestamp_ms: Self::now_ms(),
                event_type: "build_finish".to_string(),
                event_id: format!("dag-build-cancel-{}", req.build_id),
                properties: {
                    let mut p = std::collections::HashMap::new();
                    p.insert("outcome".to_string(), "CANCELLED".to_string());
                    if !req.reason.is_empty() {
                        p.insert("reason".to_string(), req.reason.clone());
                    }
                    p
                },
                display_name: "Build".to_string(),
                parent_id: String::new(),
            });

            let cancel_reason = req.reason.clone();
            tracing::info!(
                build_id = %req.build_id,
                reason = %cancel_reason,
                "Build cancelled"
            );

            Ok(Response::new(CancelBuildResponse { cancelled: true }))
        } else {
            Ok(Response::new(CancelBuildResponse { cancelled: false }))
        }
    }

    async fn get_next_task(
        &self,
        request: Request<GetNextTaskRequest>,
    ) -> Result<Response<GetNextTaskResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id.clone());

        if let Some(execution) = self.builds.get(&build_id) {
            // Check cancellation
            if execution.is_cancelled() {
                return Ok(Response::new(GetNextTaskResponse {
                    task_path: BUILD_COMPLETE_SENTINEL.to_string(),
                    task_type: String::new(),
                    estimated_duration_ms: 0,
                }));
            }

            // Check if build is complete
            if execution.is_terminal() {
                return Ok(Response::new(GetNextTaskResponse {
                    task_path: BUILD_COMPLETE_SENTINEL.to_string(),
                    task_type: String::new(),
                    estimated_duration_ms: 0,
                }));
            }

            // Check parallelism limit
            let exec_count = execution.executing_count();
            if exec_count >= execution.max_parallelism as i32 {
                return Ok(Response::new(GetNextTaskResponse {
                    task_path: String::new(),
                    task_type: String::new(),
                    estimated_duration_ms: 0,
                }));
            }

            // Pop from ready queue
            if let Ok(mut queue) = execution.ready_queue.lock() {
                while let Some(ready_task) = queue.pop() {
                    let task_path = ready_task.task_path;
                    // Mark as executing
                    if let Ok(mut executing) = execution.executing.lock() {
                        executing.insert(task_path.clone());
                    }
                    if let Some(slot) = execution.tasks.get(&task_path) {
                        return Ok(Response::new(GetNextTaskResponse {
                            task_path,
                            task_type: slot.task_type.clone(),
                            estimated_duration_ms: slot.estimated_duration_ms,
                        }));
                    }
                }
            }

            // No tasks ready
            Ok(Response::new(GetNextTaskResponse {
                task_path: String::new(),
                task_type: String::new(),
                estimated_duration_ms: 0,
            }))
        } else {
            Err(Status::not_found(format!(
                "No active build for build_id: {}",
                req.build_id
            )))
        }
    }

    async fn notify_task_started(
        &self,
        request: Request<NotifyTaskStartedRequest>,
    ) -> Result<Response<NotifyTaskStartedResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id.clone());

        if let Some(execution) = self.builds.get(&build_id) {
            // Update task status
            if let Some(slot) = execution.tasks.get(&req.task_path) {
                // Status already EXECUTING from GetNextTask dispatch
                let _ = slot; // used implicitly via the executing set
            }

            // Track in TaskGraphService for progress
            let _ = self
                .task_graph
                .task_started(Request::new(TaskStartedRequest {
                    build_id: req.build_id.clone(),
                    task_path: req.task_path.clone(),
                    start_time_ms: req.start_time_ms,
                }))
                .await;

            // Track in WorkerScheduler
            self.scheduler
                .start_work(req.task_path.clone(), req.start_time_ms);

            // Dispatch task_start event
            self.dispatch_event(&BuildEventMessage {
                build_id: req.build_id.clone(),
                timestamp_ms: Self::now_ms(),
                event_type: "task_start".to_string(),
                event_id: format!("dag-task-start-{}", req.task_path),
                properties: Default::default(),
                display_name: req.task_path.clone(),
                parent_id: String::new(),
            });

            tracing::debug!(
                build_id = %req.build_id,
                task_path = %req.task_path,
                start_time_ms = req.start_time_ms,
                "Task started"
            );

            Ok(Response::new(NotifyTaskStartedResponse {
                acknowledged: true,
            }))
        } else {
            Ok(Response::new(NotifyTaskStartedResponse {
                acknowledged: false,
            }))
        }
    }

    async fn notify_task_finished(
        &self,
        request: Request<NotifyTaskFinishedRequest>,
    ) -> Result<Response<NotifyTaskFinishedResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id.clone());

        // Phase 1: All synchronous mutations under the DashMap guard.
        // Collect data needed for async calls after dropping the guard.
        let (should_dispatch, newly_ready, build_just_finished, build_outcome_str, failure_msg) = {
            let mut execution = match self.builds.get_mut(&build_id) {
                Some(e) => e,
                None => {
                    return Ok(Response::new(NotifyTaskFinishedResponse {
                        acknowledged: false,
                        newly_ready_tasks: vec![],
                    }));
                }
            };

            if execution.is_terminal() {
                return Ok(Response::new(NotifyTaskFinishedResponse {
                    acknowledged: false,
                    newly_ready_tasks: vec![],
                }));
            }

            let outcome = if req.success {
                "SUCCEEDED".to_string()
            } else {
                "FAILED".to_string()
            };

            // Update task slot
            if let Some(slot) = execution.tasks.get_mut(&req.task_path) {
                slot.status = outcome.clone();
                slot.duration_ms = req.duration_ms;
                slot.start_time_ms = Self::now_ms().saturating_sub(req.duration_ms);
            }

            // Remove from executing set
            if let Ok(mut executing) = execution.executing.lock() {
                executing.remove(&req.task_path);
            }

            let newly_ready = if req.success {
                Self::try_unblock_dependents(&mut execution, &req.task_path)
            } else {
                execution.failure_message = req.failure_message.clone();
                Self::skip_transitive_dependents(&mut execution, &req.task_path);
                Vec::new()
            };

            let was_executing = execution.status == "EXECUTING";
            Self::check_build_completion(&mut execution);

            let build_just_finished = was_executing && execution.is_terminal();
            let build_outcome_str = if execution.status == "COMPLETED" {
                "SUCCESS".to_string()
            } else {
                "FAILED".to_string()
            };
            let failure_msg = execution.failure_message.clone();

            (
                true,
                newly_ready,
                build_just_finished,
                build_outcome_str,
                failure_msg,
            )
        };
        // DashMap guard is dropped here.

        if !should_dispatch {
            return Ok(Response::new(NotifyTaskFinishedResponse {
                acknowledged: false,
                newly_ready_tasks: vec![],
            }));
        }

        // Phase 2: Async calls (outside the DashMap guard).
        // Track in TaskGraphService for progress
        let _ = self
            .task_graph
            .task_finished(Request::new(TaskFinishedRequest {
                build_id: req.build_id.clone(),
                task_path: req.task_path.clone(),
                duration_ms: req.duration_ms,
                success: req.success,
                outcome: req.outcome.clone(),
            }))
            .await;

        // Track in WorkerScheduler
        self.scheduler.complete_work(&req.task_path);

        // Dispatch task_finish event
        self.dispatch_event(&BuildEventMessage {
            build_id: req.build_id.clone(),
            timestamp_ms: Self::now_ms(),
            event_type: "task_finish".to_string(),
            event_id: format!("dag-task-finish-{}", req.task_path),
            properties: {
                let mut p = std::collections::HashMap::new();
                p.insert("outcome".to_string(), req.outcome.clone());
                p.insert("duration_ms".to_string(), req.duration_ms.to_string());
                p
            },
            display_name: req.task_path.clone(),
            parent_id: String::new(),
        });

        // If build just completed, dispatch build_finish event
        if build_just_finished {
            self.dispatch_event(&BuildEventMessage {
                build_id: req.build_id.clone(),
                timestamp_ms: Self::now_ms(),
                event_type: "build_finish".to_string(),
                event_id: format!("dag-build-finish-{}", req.build_id),
                properties: {
                    let mut p = std::collections::HashMap::new();
                    p.insert("outcome".to_string(), build_outcome_str.clone());
                    if !failure_msg.is_empty() {
                        p.insert("failure_message".to_string(), failure_msg);
                    }
                    p
                },
                display_name: "Build".to_string(),
                parent_id: String::new(),
            });

            tracing::info!(
                build_id = %req.build_id,
                final_status = %build_outcome_str,
                "Build completed"
            );
        }

        tracing::debug!(
            build_id = %req.build_id,
            task_path = %req.task_path,
            outcome = %req.outcome,
            duration_ms = req.duration_ms,
            newly_ready = newly_ready.len(),
            "Task finished"
        );

        Ok(Response::new(NotifyTaskFinishedResponse {
            acknowledged: true,
            newly_ready_tasks: newly_ready,
        }))
    }

    async fn get_build_status(
        &self,
        request: Request<GetBuildStatusRequest>,
    ) -> Result<Response<GetBuildStatusResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id.clone());

        if let Some(execution) = self.builds.get(&build_id) {
            let elapsed = Self::now_ms() - execution.start_time_ms;

            Ok(Response::new(GetBuildStatusResponse {
                build_id: req.build_id,
                status: execution.status.clone(),
                total_tasks: execution.total_tasks,
                completed_tasks: execution.completed_count(),
                executing_tasks: execution.executing_count(),
                pending_tasks: execution.pending_count(),
                failed_tasks: execution.failed_count(),
                skipped_tasks: execution.skipped_count(),
                elapsed_ms: elapsed,
                task_statuses: Self::get_task_statuses(&execution),
            }))
        } else {
            Err(Status::not_found(format!(
                "No active build for build_id: {}",
                req.build_id
            )))
        }
    }

    async fn await_build_completion(
        &self,
        request: Request<AwaitBuildCompletionRequest>,
    ) -> Result<Response<AwaitBuildCompletionResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id.clone());

        let notify = if let Some(execution) = self.builds.get(&build_id) {
            if execution.is_terminal() {
                // Already done
                let succeeded =
                    execution.total_tasks - execution.failed_count() - execution.skipped_count();
                return Ok(Response::new(AwaitBuildCompletionResponse {
                    build_id: req.build_id,
                    final_status: execution.status.clone(),
                    tasks_succeeded: succeeded,
                    tasks_failed: execution.failed_count(),
                    tasks_skipped: execution.skipped_count(),
                    total_duration_ms: Self::now_ms() - execution.start_time_ms,
                    failure_message: execution.failure_message.clone(),
                }));
            }
            execution.completion_notify.clone()
        } else {
            return Err(Status::not_found(format!(
                "No active build for build_id: {}",
                req.build_id
            )));
        };

        // Wait for completion or timeout
        if req.timeout_ms > 0 {
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(req.timeout_ms as u64),
                notify.notified(),
            )
            .await;

            if result.is_err() {
                return Err(Status::deadline_exceeded(
                    "Timed out waiting for build completion",
                ));
            }
        } else {
            notify.notified().await;
        }

        // Read final state
        if let Some(execution) = self.builds.get(&build_id) {
            let succeeded =
                execution.total_tasks - execution.failed_count() - execution.skipped_count();
            Ok(Response::new(AwaitBuildCompletionResponse {
                build_id: req.build_id,
                final_status: execution.status.clone(),
                tasks_succeeded: succeeded,
                tasks_failed: execution.failed_count(),
                tasks_skipped: execution.skipped_count(),
                total_duration_ms: Self::now_ms() - execution.start_time_ms,
                failure_message: execution.failure_message.clone(),
            }))
        } else {
            Err(Status::internal("Build execution disappeared"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::jvm_host::JvmHostClient;
    use crate::client::jvm_host_bridge::JvmHostBridge;
    use crate::proto::jvm_host_service_server::{JvmHostService, JvmHostServiceServer};
    use crate::proto::{FileChangeEvent, RegisterTaskRequest};
    use crate::server::task_graph;
    use tokio::net::UnixListener;
    use tonic::transport::Server;

    struct MockJvmTaskHost;

    #[tonic::async_trait]
    impl JvmHostService for MockJvmTaskHost {
        async fn evaluate_script(
            &self,
            _request: Request<crate::proto::EvaluateScriptRequest>,
        ) -> Result<Response<crate::proto::EvaluateScriptResponse>, Status> {
            Ok(Response::new(crate::proto::EvaluateScriptResponse {
                success: true,
                error_message: String::new(),
                applied_plugins: Vec::new(),
            }))
        }

        async fn get_build_model(
            &self,
            _request: Request<crate::proto::GetBuildModelRequest>,
        ) -> Result<Response<crate::proto::GetBuildModelResponse>, Status> {
            Ok(Response::new(crate::proto::GetBuildModelResponse {
                projects: Vec::new(),
            }))
        }

        async fn resolve_configuration(
            &self,
            _request: Request<crate::proto::ResolveConfigRequest>,
        ) -> Result<Response<crate::proto::ResolveConfigResponse>, Status> {
            Ok(Response::new(crate::proto::ResolveConfigResponse {
                success: true,
                artifacts: Vec::new(),
                error_message: String::new(),
            }))
        }

        async fn get_build_environment(
            &self,
            _request: Request<crate::proto::GetBuildEnvironmentRequest>,
        ) -> Result<Response<crate::proto::GetBuildEnvironmentResponse>, Status> {
            Ok(Response::new(crate::proto::GetBuildEnvironmentResponse {
                java_version: "17".to_string(),
                java_home: "/mock/java".to_string(),
                gradle_version: "mock".to_string(),
                os_name: "mock".to_string(),
                os_arch: "mock".to_string(),
                available_processors: 1,
                max_memory_bytes: 1024,
                system_properties: Default::default(),
            }))
        }

        async fn get_build_plan(
            &self,
            request: Request<crate::proto::GetBuildPlanRequest>,
        ) -> Result<Response<crate::proto::GetBuildPlanResponse>, Status> {
            Ok(Response::new(crate::proto::GetBuildPlanResponse {
                success: true,
                error_message: String::new(),
                plan: Some(crate::proto::BuildPlan {
                    schema_version: super::super::build_plan_ir::BUILD_PLAN_SCHEMA_VERSION,
                    build_id: request.into_inner().build_id,
                    projects: Vec::new(),
                    tasks: Vec::new(),
                    dependencies: Vec::new(),
                    toolchains: Vec::new(),
                    metadata: Default::default(),
                }),
                source: "mock-jvm-host".to_string(),
            }))
        }

        async fn execute_task(
            &self,
            request: Request<crate::proto::ExecuteTaskRequest>,
        ) -> Result<Response<crate::proto::ExecuteTaskResponse>, Status> {
            let req = request.into_inner();
            let execution_mode = if req.task_type == "GradleEngineTask" {
                "jvm_gradle_task_executer".to_string()
            } else {
                format!("mock-jvm:{}", req.task_type)
            };
            Ok(Response::new(crate::proto::ExecuteTaskResponse {
                success: true,
                outcome: "EXECUTED".to_string(),
                error_message: String::new(),
                duration_ms: 11,
                execution_mode,
            }))
        }
    }

    #[test]
    fn kernel_dependency_graph_groups_build_plan_dependencies() {
        let graph = kernel_dependency_graph_from_plan_dependencies(&[
            plan_dependency(
                ":",
                "implementation",
                "org.example:demo:1.2.3",
                "dependency",
            ),
            plan_dependency(
                ":",
                "testImplementation",
                "org.example:test:4.5.6",
                "dependency",
            ),
            plan_dependency(
                ":",
                "runtimeClasspath",
                "org.example:native:7.8.9:linux@so",
                "dependency",
            ),
        ])
        .expect("dependency graph");

        assert_eq!(graph.configurations.len(), 3);
        assert_eq!(graph.configurations[0].name, "::implementation");
        assert_eq!(graph.configurations[0].dependencies[0].group, "org.example");
        assert_eq!(graph.configurations[0].dependencies[0].name, "demo");
        assert_eq!(graph.configurations[0].dependencies[0].version, "1.2.3");
        assert_eq!(graph.configurations[1].name, "::runtimeClasspath");
        assert_eq!(graph.configurations[1].dependencies[0].group, "org.example");
        assert_eq!(graph.configurations[1].dependencies[0].name, "native");
        assert_eq!(graph.configurations[1].dependencies[0].version, "7.8.9");
    }

    #[test]
    fn kernel_dependency_graph_marks_unrepresentable_notation_unsupported() {
        let graph = kernel_dependency_graph_from_plan_dependencies(&[plan_dependency(
            ":",
            "implementation",
            "files('libs/demo.jar')",
            "dependency",
        )])
        .expect("dependency graph");

        assert!(graph.configurations[0].dependencies.is_empty());
        assert!(graph.configurations[0].unsupported_features[0]
            .contains("unsupported dependency notation"));
    }

    #[test]
    fn kernel_dependency_graph_preserves_project_dependencies() {
        let graph = kernel_dependency_graph_from_plan_dependencies(&[plan_dependency(
            ":app",
            "implementation",
            "project(':lib')",
            "dependency",
        )])
        .expect("dependency graph");

        assert_eq!(graph.configurations[0].name, ":app:implementation");
        assert_eq!(graph.configurations[0].project_dependencies, vec![":lib"]);
        assert!(graph.configurations[0].unsupported_features.is_empty());
    }

    #[test]
    fn kernel_dependency_graph_preserves_dependency_constraints() {
        let graph = kernel_dependency_graph_from_plan_dependencies(&[plan_dependency(
            ":",
            "implementation",
            "org.example:constrained:1.2.3",
            "constraint",
        )])
        .expect("dependency graph");

        assert!(graph.configurations[0].dependencies.is_empty());
        assert_eq!(graph.configurations[0].constraints.len(), 1);
        assert_eq!(graph.configurations[0].constraints[0].group, "org.example");
        assert_eq!(graph.configurations[0].constraints[0].name, "constrained");
        assert_eq!(graph.configurations[0].constraints[0].version, "1.2.3");
    }

    #[test]
    fn kernel_dependency_graph_rejects_unknown_dependency_kind() {
        let graph = kernel_dependency_graph_from_plan_dependencies(&[plan_dependency(
            ":",
            "implementation",
            "org.example:demo:1.2.3",
            "platform",
        )])
        .expect("dependency graph");

        assert!(graph.configurations[0].dependencies.is_empty());
        assert!(graph.configurations[0].constraints.is_empty());
        assert!(graph.configurations[0].unsupported_features[0]
            .contains("unsupported dependency kind 'platform'"));
    }

    #[test]
    fn kernel_dependency_graph_defaults_empty_dependency_kind_to_dependency() {
        let graph = kernel_dependency_graph_from_plan_dependencies(&[plan_dependency(
            ":",
            "implementation",
            "org.example:demo:1.2.3",
            "",
        )])
        .expect("dependency graph");

        assert_eq!(graph.configurations[0].dependencies.len(), 1);
        assert!(graph.configurations[0].constraints.is_empty());
        assert!(graph.configurations[0].unsupported_features.is_empty());
    }

    #[test]
    fn kernel_dependency_graph_preserves_repositories_and_unsupported_markers() {
        let mut dependency = plan_dependency(
            ":",
            "implementation",
            "org.example:demo:1.2.3",
            "dependency",
        );
        dependency
            .repositories
            .push(crate::proto::RepositoryDescriptor {
                id: "unsupported".to_string(),
                url: "sftp://repo.example.test/maven".to_string(),
                m2compatible: true,
                allow_insecure_protocol: false,
                credentials: Default::default(),
                layout: String::new(),
                ivy_pattern: String::new(),
                include_groups: Vec::new(),
                exclude_groups: Vec::new(),
                include_group_prefixes: Vec::new(),
                exclude_group_prefixes: Vec::new(),
                include_modules: Vec::new(),
                exclude_modules: Vec::new(),
                include_module_versions: Vec::new(),
                exclude_module_versions: Vec::new(),
            });
        dependency
            .unsupported_features
            .push("repository-content-filter".to_string());

        let graph = kernel_dependency_graph_from_plan_dependencies(&[dependency])
            .expect("dependency graph");

        assert_eq!(graph.configurations[0].repositories.len(), 1);
        assert_eq!(graph.configurations[0].repositories[0].id, "unsupported");
        assert_eq!(
            graph.configurations[0].unsupported_features,
            vec!["repository-content-filter".to_string()]
        );
    }

    #[test]
    fn kernel_dependency_graph_marks_credentialed_repositories_unsupported() {
        let mut dependency = plan_dependency(
            ":",
            "implementation",
            "org.example:demo:1.2.3",
            "dependency",
        );
        dependency
            .repositories
            .push(crate::proto::RepositoryDescriptor {
                id: "private".to_string(),
                url: "https://repo.example.test/maven".to_string(),
                m2compatible: true,
                allow_insecure_protocol: false,
                credentials: [("username".to_string(), "user".to_string())].into(),
                layout: String::new(),
                ivy_pattern: String::new(),
                include_groups: Vec::new(),
                exclude_groups: Vec::new(),
                include_group_prefixes: Vec::new(),
                exclude_group_prefixes: Vec::new(),
                include_modules: Vec::new(),
                exclude_modules: Vec::new(),
                include_module_versions: Vec::new(),
                exclude_module_versions: Vec::new(),
            });

        let graph = kernel_dependency_graph_from_plan_dependencies(&[dependency])
            .expect("dependency graph");

        assert_eq!(graph.configurations[0].repositories.len(), 1);
        assert_eq!(
            graph.configurations[0].unsupported_features,
            vec!["repository-credentials:private".to_string()]
        );
    }

    fn plan_dependency(
        project_path: &str,
        configuration: &str,
        notation: &str,
        kind: &str,
    ) -> crate::proto::BuildPlanDependency {
        crate::proto::BuildPlanDependency {
            project_path: project_path.to_string(),
            configuration: configuration.to_string(),
            notation: notation.to_string(),
            kind: kind.to_string(),
            repositories: Vec::new(),
            unsupported_features: Vec::new(),
        }
    }

    fn make_svc() -> DagExecutorServiceImpl {
        let task_graph = Arc::new(super::super::task_graph::TaskGraphServiceImpl::new());
        let scheduler = Arc::new(WorkerScheduler::new(4));
        DagExecutorServiceImpl::new(
            scheduler,
            task_graph,
            Arc::new(super::super::execution_plan::ExecutionPlanServiceImpl::default()),
            Vec::new(),
        )
    }

    fn make_svc_with_task_graph(
        task_graph: Arc<super::super::task_graph::TaskGraphServiceImpl>,
    ) -> DagExecutorServiceImpl {
        DagExecutorServiceImpl::new(
            Arc::new(WorkerScheduler::new(4)),
            task_graph,
            Arc::new(super::super::execution_plan::ExecutionPlanServiceImpl::default()),
            Vec::new(),
        )
    }

    async fn make_mock_jvm_bridge() -> (SharedJvmHostBridge, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("jvm-task-host.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        tokio::spawn(async move {
            Server::builder()
                .add_service(JvmHostServiceServer::new(MockJvmTaskHost))
                .serve_with_incoming(tokio_stream::wrappers::UnixListenerStream::new(listener))
                .await
                .unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let bridge = Arc::new(JvmHostBridge::new());
        let client = JvmHostClient::connect(&socket_path.to_string_lossy())
            .await
            .unwrap();
        bridge.set_client(client).await;

        (bridge, dir)
    }

    async fn make_svc_with_mock_jvm_host() -> (DagExecutorServiceImpl, tempfile::TempDir) {
        let (bridge, dir) = make_mock_jvm_bridge().await;
        (make_svc().with_jvm_host_bridge(bridge), dir)
    }

    /// Helper to register tasks in the task graph before starting a build.
    async fn register_chain(
        svc: &DagExecutorServiceImpl,
        build_id: &str,
        tasks: &[(&str, &str, &[&str])],
    ) {
        for (path, task_type, deps) in tasks {
            let _ = svc
                .task_graph
                .register_task(Request::new(RegisterTaskRequest {
                    build_id: build_id.to_string(),
                    task_path: path.to_string(),
                    depends_on: deps.iter().map(|d| d.to_string()).collect(),
                    should_execute: true,
                    task_type: task_type.to_string(),
                    input_files: vec![],
                }))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn test_start_and_complete_simple_chain() {
        let svc = make_svc();
        register_chain(
            &svc,
            "build-1",
            &[
                (":a", "Task", &[]),
                (":b", "Task", &[":a"]),
                (":c", "Task", &[":b"]),
            ],
        )
        .await;

        // Start build
        let resp = svc
            .start_build(Request::new(StartBuildRequest {
                build_id: "build-1".to_string(),
                max_parallelism: 2,
                task_filter: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.accepted);
        assert_eq!(resp.total_tasks, 3);

        // Get first task (should be :a)
        let next = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(next.task_path, ":a");

        // Notify started
        svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: "build-1".to_string(),
            task_path: ":a".to_string(),
            start_time_ms: 100,
        }))
        .await
        .unwrap();

        // Notify finished
        let finish = svc
            .notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                build_id: "build-1".to_string(),
                task_path: ":a".to_string(),
                success: true,
                outcome: "EXECUTED".to_string(),
                duration_ms: 50,
                failure_message: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(finish.acknowledged);
        assert!(finish.newly_ready_tasks.contains(&":b".to_string()));

        // Get next task (should be :b)
        let next = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(next.task_path, ":b");

        // Finish :b
        svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: "build-1".to_string(),
            task_path: ":b".to_string(),
            start_time_ms: 200,
        }))
        .await
        .unwrap();

        svc.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
            build_id: "build-1".to_string(),
            task_path: ":b".to_string(),
            success: true,
            outcome: "EXECUTED".to_string(),
            duration_ms: 100,
            failure_message: String::new(),
        }))
        .await
        .unwrap();

        // Get next task (should be :c)
        let next = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(next.task_path, ":c");

        // Finish :c
        svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: "build-1".to_string(),
            task_path: ":c".to_string(),
            start_time_ms: 350,
        }))
        .await
        .unwrap();

        svc.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
            build_id: "build-1".to_string(),
            task_path: ":c".to_string(),
            success: true,
            outcome: "EXECUTED".to_string(),
            duration_ms: 50,
            failure_message: String::new(),
        }))
        .await
        .unwrap();

        // Next should be BUILD_COMPLETE
        let next = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(next.task_path, BUILD_COMPLETE_SENTINEL);

        // Check final status
        let status = svc
            .get_build_status(Request::new(GetBuildStatusRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.status, "COMPLETED");
        assert_eq!(status.total_tasks, 3);
        assert_eq!(status.completed_tasks, 3);
        assert_eq!(status.failed_tasks, 0);
    }

    #[tokio::test]
    async fn test_start_build_prefers_build_plan_shadow_over_registered_tasks() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(super::super::build_plan_shadow::BuildPlanShadowStore::new(
            temp.path().to_path_buf(),
        ));
        let history = Arc::new(
            super::super::execution_history::ExecutionHistoryServiceImpl::new(
                temp.path().join("history"),
            ),
        );
        let task_graph = Arc::new(
            super::super::task_graph::TaskGraphServiceImpl::with_history_and_shadow(
                history,
                Arc::clone(&store),
            ),
        );
        let (bridge, _jvm_host_dir) = make_mock_jvm_bridge().await;
        let svc = make_svc_with_task_graph(Arc::clone(&task_graph)).with_jvm_host_bridge(bridge);
        let build_id = "dag-shadow-build";

        register_chain(&svc, build_id, &[(":staleFallback", "StaleTask", &[])]).await;

        store
            .persist_plan(
                &super::super::build_plan_ir::CanonicalBuildPlan {
                    schema_version: super::super::build_plan_ir::BUILD_PLAN_SCHEMA_VERSION,
                    build_id: build_id.to_string(),
                    projects: Vec::new(),
                    tasks: vec![super::super::build_plan_ir::CanonicalBuildPlanTask {
                        path: ":fromShadow".to_string(),
                        project_path: ":".to_string(),
                        implementation_id: "ShadowTask".to_string(),
                        depends_on: Vec::new(),
                        inputs: Default::default(),
                        outputs: Vec::new(),
                        worker_isolation: "compat-jvm".to_string(),
                        should_run_after: Vec::new(),
                        must_run_after: Vec::new(),
                        finalized_by: Vec::new(),
                        cacheability: "unknown".to_string(),
                        local_state: Vec::new(),
                        destroyables: Vec::new(),
                        action_kind: "jvm-task".to_string(),
                        input_specs: Vec::new(),
                        output_specs: Vec::new(),
                        environment_inputs: Vec::new(),
                        system_property_inputs: Vec::new(),
                        diagnostics: Vec::new(),
                    }],
                    dependencies: Vec::new(),
                    toolchains: Vec::new(),
                    metadata: Default::default(),
                },
                "test-shadow",
            )
            .unwrap();

        let started = svc
            .start_build(Request::new(StartBuildRequest {
                build_id: build_id.to_string(),
                max_parallelism: 1,
                task_filter: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(started.accepted);
        assert_eq!(started.total_tasks, 1);
        assert_eq!(started.plan_source, "build-plan-shadow");

        let next = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: build_id.to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(next.task_path, ":fromShadow");
        assert_eq!(next.task_type, "ShadowTask");
    }

    #[tokio::test]
    async fn test_parallel_tasks() {
        let svc = make_svc();
        register_chain(
            &svc,
            "build-par",
            &[
                (":a", "Task", &[]),
                (":b", "Task", &[]),
                (":c", "Task", &[":a", ":b"]),
            ],
        )
        .await;

        svc.start_build(Request::new(StartBuildRequest {
            build_id: "build-par".to_string(),
            max_parallelism: 4,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        // Both :a and :b should be available
        let t1 = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-par".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let t2 = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-par".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let mut roots = vec![t1.task_path, t2.task_path];
        roots.sort_unstable();
        assert_eq!(roots, vec![":a".to_string(), ":b".to_string()]);

        // :c should NOT be available yet
        let t3 = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-par".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(t3.task_path.is_empty());

        // Finish both
        for task in &[":a", ":b"] {
            svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
                build_id: "build-par".to_string(),
                task_path: task.to_string(),
                start_time_ms: 100,
            }))
            .await
            .unwrap();

            svc.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                build_id: "build-par".to_string(),
                task_path: task.to_string(),
                success: true,
                outcome: "EXECUTED".to_string(),
                duration_ms: 50,
                failure_message: String::new(),
            }))
            .await
            .unwrap();
        }

        // Now :c should be ready
        let t4 = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-par".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(t4.task_path, ":c");
    }

    #[tokio::test]
    async fn test_failure_skips_dependents() {
        let svc = make_svc();
        register_chain(
            &svc,
            "build-fail",
            &[
                (":a", "Task", &[]),
                (":b", "Task", &[":a"]),
                (":c", "Task", &[":b"]),
                (":d", "Task", &[":c"]),
            ],
        )
        .await;

        svc.start_build(Request::new(StartBuildRequest {
            build_id: "build-fail".to_string(),
            max_parallelism: 4,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        // Execute and fail :a
        let _ = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-fail".to_string(),
            }))
            .await
            .unwrap();

        svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: "build-fail".to_string(),
            task_path: ":a".to_string(),
            start_time_ms: 100,
        }))
        .await
        .unwrap();

        svc.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
            build_id: "build-fail".to_string(),
            task_path: ":a".to_string(),
            success: false,
            outcome: "FAILED".to_string(),
            duration_ms: 50,
            failure_message: "Compilation error".to_string(),
        }))
        .await
        .unwrap();

        // Build should be FAILED, :b/:c/:d should be SKIPPED
        let status = svc
            .get_build_status(Request::new(GetBuildStatusRequest {
                build_id: "build-fail".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.status, "FAILED");
        assert_eq!(status.failed_tasks, 1);
        assert_eq!(status.skipped_tasks, 3);
        assert_eq!(status.status, "FAILED");
        assert_eq!(status.failed_tasks, 1);
        assert_eq!(status.skipped_tasks, 3);
    }

    #[tokio::test]
    async fn test_cancel_build() {
        let svc = make_svc();
        register_chain(
            &svc,
            "build-cancel",
            &[
                (":a", "Task", &[]),
                (":b", "Task", &[":a"]),
                (":c", "Task", &[]),
            ],
        )
        .await;

        svc.start_build(Request::new(StartBuildRequest {
            build_id: "build-cancel".to_string(),
            max_parallelism: 4,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        // Cancel
        let resp = svc
            .cancel_build(Request::new(CancelBuildRequest {
                build_id: "build-cancel".to_string(),
                reason: "User cancelled".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.cancelled);

        // Next task should return BUILD_COMPLETE
        let next = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-cancel".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(next.task_path, BUILD_COMPLETE_SENTINEL);

        let status = svc
            .get_build_status(Request::new(GetBuildStatusRequest {
                build_id: "build-cancel".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.status, "CANCELLED");
    }

    #[tokio::test]
    async fn test_concurrent_builds_isolated() {
        let svc = make_svc();

        // Build 1: :a -> :b
        register_chain(&svc, "b1", &[(":a", "Task", &[]), (":b", "Task", &[":a"])]).await;

        // Build 2: :x -> :y
        register_chain(&svc, "b2", &[(":x", "Task", &[]), (":y", "Task", &[":x"])]).await;

        svc.start_build(Request::new(StartBuildRequest {
            build_id: "b1".to_string(),
            max_parallelism: 4,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        svc.start_build(Request::new(StartBuildRequest {
            build_id: "b2".to_string(),
            max_parallelism: 4,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        // Each build should see its own tasks
        let t1 = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "b1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let t2 = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "b2".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(t1.task_path, ":a");
        assert_eq!(t2.task_path, ":x");
    }

    #[tokio::test]
    async fn test_empty_graph_rejected() {
        let svc = make_svc();

        let resp = svc
            .start_build(Request::new(StartBuildRequest {
                build_id: "empty".to_string(),
                max_parallelism: 4,
                task_filter: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.accepted);
        assert!(!resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_cycle_detection() {
        let svc = make_svc();

        // Register cycle: :a -> :b -> :a
        for (path, deps) in &[
            (":a", vec![":b".to_string()]),
            (":b", vec![":a".to_string()]),
        ] {
            let _ = svc
                .task_graph
                .register_task(Request::new(RegisterTaskRequest {
                    build_id: "cycle".to_string(),
                    task_path: path.to_string(),
                    depends_on: deps.clone(),
                    should_execute: true,
                    task_type: "Task".to_string(),
                    input_files: vec![],
                }))
                .await
                .unwrap();
        }

        let resp = svc
            .start_build(Request::new(StartBuildRequest {
                build_id: "cycle".to_string(),
                max_parallelism: 4,
                task_filter: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.accepted);
        assert!(resp.error_message.contains("cycles"));
    }

    #[tokio::test]
    async fn test_task_filter() {
        let svc = make_svc();
        register_chain(
            &svc,
            "build-filter",
            &[
                (":a", "Task", &[]),
                (":b", "Task", &[]),
                (":c", "Task", &[":a"]),
            ],
        )
        .await;

        // Only execute :b
        let resp = svc
            .start_build(Request::new(StartBuildRequest {
                build_id: "build-filter".to_string(),
                max_parallelism: 4,
                task_filter: vec![":b".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.accepted);
        assert_eq!(resp.total_tasks, 1);

        let next = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-filter".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(next.task_path, ":b");
    }

    #[tokio::test]
    async fn test_parallelism_limit() {
        let svc = make_svc();
        register_chain(
            &svc,
            "build-limit",
            &[
                (":a", "Task", &[]),
                (":b", "Task", &[]),
                (":c", "Task", &[]),
            ],
        )
        .await;

        // Max parallelism = 1
        svc.start_build(Request::new(StartBuildRequest {
            build_id: "build-limit".to_string(),
            max_parallelism: 1,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        // Get first task
        let t1 = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-limit".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!t1.task_path.is_empty(), "Should get a task");
        let first_task = t1.task_path.clone();

        // Second should be blocked by parallelism limit
        let t2 = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-limit".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(t2.task_path.is_empty());

        // Finish the first task
        svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: "build-limit".to_string(),
            task_path: first_task.clone(),
            start_time_ms: 100,
        }))
        .await
        .unwrap();

        svc.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
            build_id: "build-limit".to_string(),
            task_path: first_task,
            success: true,
            outcome: "EXECUTED".to_string(),
            duration_ms: 50,
            failure_message: String::new(),
        }))
        .await
        .unwrap();

        // Now should be able to get next task
        let t3 = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-limit".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!t3.task_path.is_empty());
    }

    #[tokio::test]
    async fn test_await_build_completion() {
        let svc = make_svc();
        register_chain(
            &svc,
            "build-await",
            &[(":a", "Task", &[]), (":b", "Task", &[":a"])],
        )
        .await;

        svc.start_build(Request::new(StartBuildRequest {
            build_id: "build-await".to_string(),
            max_parallelism: 4,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        // Complete all tasks in background
        let svc_clone = svc.clone();
        tokio::spawn(async move {
            for task in &[":a", ":b"] {
                let _ = svc_clone
                    .get_next_task(Request::new(GetNextTaskRequest {
                        build_id: "build-await".to_string(),
                    }))
                    .await;

                let _ = svc_clone
                    .notify_task_started(Request::new(NotifyTaskStartedRequest {
                        build_id: "build-await".to_string(),
                        task_path: task.to_string(),
                        start_time_ms: 100,
                    }))
                    .await;

                tokio::time::sleep(std::time::Duration::from_millis(10)).await;

                let _ = svc_clone
                    .notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                        build_id: "build-await".to_string(),
                        task_path: task.to_string(),
                        success: true,
                        outcome: "EXECUTED".to_string(),
                        duration_ms: 50,
                        failure_message: String::new(),
                    }))
                    .await;
            }
        });

        // Await completion
        let resp = svc
            .await_build_completion(Request::new(AwaitBuildCompletionRequest {
                build_id: "build-await".to_string(),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.tasks_succeeded, 2);
        assert_eq!(resp.tasks_failed, 0);
    }

    #[tokio::test]
    async fn test_cancel_already_completed() {
        let svc = make_svc();
        register_chain(&svc, "build-done", &[(":a", "Task", &[])]).await;

        svc.start_build(Request::new(StartBuildRequest {
            build_id: "build-done".to_string(),
            max_parallelism: 4,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        // Complete the build
        let _ = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-done".to_string(),
            }))
            .await;

        svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: "build-done".to_string(),
            task_path: ":a".to_string(),
            start_time_ms: 100,
        }))
        .await
        .unwrap();

        svc.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
            build_id: "build-done".to_string(),
            task_path: ":a".to_string(),
            success: true,
            outcome: "EXECUTED".to_string(),
            duration_ms: 50,
            failure_message: String::new(),
        }))
        .await
        .unwrap();

        // Try to cancel — should fail
        let resp = svc
            .cancel_build(Request::new(CancelBuildRequest {
                build_id: "build-done".to_string(),
                reason: "too late".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.cancelled);
    }

    #[tokio::test]
    async fn test_get_build_status_unknown() {
        let svc = make_svc();

        let result = svc
            .get_build_status(Request::new(GetBuildStatusRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_diamond_dependency() {
        let svc = make_svc();
        //    a
        //   / \
        //  b   c
        //   \ /
        //    d
        register_chain(
            &svc,
            "build-diamond",
            &[
                (":a", "Task", &[]),
                (":b", "Task", &[":a"]),
                (":c", "Task", &[":a"]),
                (":d", "Task", &[":b", ":c"]),
            ],
        )
        .await;

        svc.start_build(Request::new(StartBuildRequest {
            build_id: "build-diamond".to_string(),
            max_parallelism: 4,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        // Get :a
        let t = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-diamond".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(t.task_path, ":a");

        // No more ready (b and c depend on a)
        let t = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-diamond".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(t.task_path.is_empty());

        // Finish :a
        svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: "build-diamond".to_string(),
            task_path: ":a".to_string(),
            start_time_ms: 100,
        }))
        .await
        .unwrap();

        let finish = svc
            .notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                build_id: "build-diamond".to_string(),
                task_path: ":a".to_string(),
                success: true,
                outcome: "EXECUTED".to_string(),
                duration_ms: 50,
                failure_message: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        // Both :b and :c should be newly ready
        assert_eq!(finish.newly_ready_tasks.len(), 2);

        // Finish :b and :c
        for task in &[":b", ":c"] {
            let _ = svc
                .get_next_task(Request::new(GetNextTaskRequest {
                    build_id: "build-diamond".to_string(),
                }))
                .await
                .unwrap();

            svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
                build_id: "build-diamond".to_string(),
                task_path: task.to_string(),
                start_time_ms: 200,
            }))
            .await
            .unwrap();

            let finish = svc
                .notify_task_finished(Request::new(NotifyTaskFinishedRequest {
                    build_id: "build-diamond".to_string(),
                    task_path: task.to_string(),
                    success: true,
                    outcome: "EXECUTED".to_string(),
                    duration_ms: 50,
                    failure_message: String::new(),
                }))
                .await
                .unwrap()
                .into_inner();

            // :d is only ready after BOTH b and c finish
            if *task == ":b" {
                assert!(finish.newly_ready_tasks.is_empty());
            } else {
                assert!(finish.newly_ready_tasks.contains(&":d".to_string()));
            }
        }

        // Get and finish :d
        let t = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-diamond".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(t.task_path, ":d");

        svc.notify_task_started(Request::new(NotifyTaskStartedRequest {
            build_id: "build-diamond".to_string(),
            task_path: ":d".to_string(),
            start_time_ms: 350,
        }))
        .await
        .unwrap();

        svc.notify_task_finished(Request::new(NotifyTaskFinishedRequest {
            build_id: "build-diamond".to_string(),
            task_path: ":d".to_string(),
            success: true,
            outcome: "EXECUTED".to_string(),
            duration_ms: 50,
            failure_message: String::new(),
        }))
        .await
        .unwrap();

        // Build complete
        let status = svc
            .get_build_status(Request::new(GetBuildStatusRequest {
                build_id: "build-diamond".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.status, "COMPLETED");
        assert_eq!(status.total_tasks, 4);
    }

    #[tokio::test]
    async fn test_get_next_task_prioritizes_critical_path_remaining_time() {
        let history = Arc::new(
            super::super::execution_history::ExecutionHistoryServiceImpl::new(
                std::path::PathBuf::new(),
            ),
        );
        history.store_task_duration(":fastRoot", 10);
        history.store_task_duration(":fastLeaf", 10);
        history.store_task_duration(":slowRoot", 10);
        history.store_task_duration(":slowMid", 500);
        history.store_task_duration(":slowLeaf", 500);
        let task_graph =
            Arc::new(super::super::task_graph::TaskGraphServiceImpl::with_history(history));
        let svc = make_svc_with_task_graph(task_graph);

        register_chain(
            &svc,
            "build-critical-path",
            &[
                (":fastRoot", "Task", &[]),
                (":fastLeaf", "Task", &[":fastRoot"]),
                (":slowRoot", "Task", &[]),
                (":slowMid", "Task", &[":slowRoot"]),
                (":slowLeaf", "Task", &[":slowMid"]),
            ],
        )
        .await;

        svc.start_build(Request::new(StartBuildRequest {
            build_id: "build-critical-path".to_string(),
            max_parallelism: 1,
            task_filter: vec![],
        }))
        .await
        .unwrap();

        let first = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-critical-path".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(first.task_path, ":slowRoot");
        assert_eq!(first.estimated_duration_ms, 10);
    }

    #[tokio::test]
    async fn test_filtered_task_becomes_ready_when_filtered_dependencies_are_absent() {
        let svc = make_svc();
        register_chain(
            &svc,
            "build-filtered-root",
            &[(":a", "Task", &[]), (":b", "Task", &[":a"])],
        )
        .await;

        let start = svc
            .start_build(Request::new(StartBuildRequest {
                build_id: "build-filtered-root".to_string(),
                max_parallelism: 1,
                task_filter: vec![":b".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(start.accepted);
        assert_eq!(start.total_tasks, 1);

        let next = svc
            .get_next_task(Request::new(GetNextTaskRequest {
                build_id: "build-filtered-root".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(next.task_path, ":b");
    }

    // -----------------------------------------------------------------------
    // RunBuild tests (authoritative execution)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_run_build_simple_chain_jvm_forward() {
        let (svc, _jvm_host_dir) = make_svc_with_mock_jvm_host().await;
        register_chain(
            &svc,
            "rb-chain",
            &[
                (":a", "UnknownTask", &[]),
                (":b", "UnknownTask", &[":a"]),
                (":c", "UnknownTask", &[":b"]),
            ],
        )
        .await;

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-chain".to_string(),
                max_parallelism: 2,
                task_filter: vec![],
                task_contexts: Default::default(),
                allow_jvm_forwarding: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.total_tasks, 3);
        assert_eq!(resp.plan_source, "registered-tasks");
        assert_eq!(resp.tasks_forwarded_to_jvm, 3);
        assert_eq!(resp.tasks_succeeded, 3);
        assert_eq!(resp.tasks_failed, 0);
        assert_eq!(resp.task_details.len(), 3);
        // All tasks should have a real JVM-host response.
        for d in &resp.task_details {
            assert_eq!(d.execution_mode, "mock-jvm:UnknownTask");
            assert_eq!(d.outcome, "EXECUTED");
        }
    }

    #[tokio::test]
    async fn test_run_build_counts_gradle_engine_jvm_execution_mode() {
        let (svc, _jvm_host_dir) = make_svc_with_mock_jvm_host().await;
        register_chain(
            &svc,
            "rb-gradle-engine-jvm",
            &[(":legacy", "GradleEngineTask", &[])],
        )
        .await;

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-gradle-engine-jvm".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: Default::default(),
                allow_jvm_forwarding: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.tasks_forwarded_to_jvm, 1);
        assert_eq!(resp.tasks_succeeded, 1);
        assert_eq!(resp.task_details.len(), 1);
        assert_eq!(
            resp.task_details[0].execution_mode,
            "jvm_gradle_task_executer"
        );
        assert_eq!(resp.task_details[0].outcome, "EXECUTED");
    }

    #[tokio::test]
    async fn test_run_build_missing_executor_fails_without_jvm_forwarding() {
        let svc = make_svc();
        register_chain(&svc, "rb-no-fallback", &[(":legacy", "UnknownTask", &[])]).await;

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-no-fallback".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: Default::default(),
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "FAILED");
        assert_eq!(resp.tasks_forwarded_to_jvm, 0);
        assert_eq!(resp.tasks_failed, 1);
        assert_eq!(resp.task_details.len(), 0);
        assert!(resp.failure_message.contains("rejected build"));
        assert!(resp.failure_message.contains("has no Rust executor"));
    }

    #[tokio::test]
    async fn test_run_build_rejects_unsupported_contract_before_dispatch() {
        let svc = make_svc();
        register_chain(&svc, "rb-kernel-admission", &[(":copy", "Copy", &[])]).await;
        let mut task_contexts = HashMap::new();
        task_contexts.insert(
            ":copy".to_string(),
            serde_json::json!({
                "source_files": [],
                "target_dir": "build/out",
                "copy_unsupported_custom_actions": true
            })
            .to_string(),
        );

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-kernel-admission".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "FAILED");
        assert_eq!(resp.tasks_forwarded_to_jvm, 0);
        assert_eq!(resp.task_details.len(), 0);
        assert!(
            resp.failure_message
                .contains("unsupported contract marker 'copy_unsupported_custom_actions'"),
            "unexpected failure message: {}",
            resp.failure_message
        );
    }

    #[tokio::test]
    async fn test_run_build_reports_shadow_plan_source() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(super::super::build_plan_shadow::BuildPlanShadowStore::new(
            temp.path().to_path_buf(),
        ));
        let history = Arc::new(
            super::super::execution_history::ExecutionHistoryServiceImpl::new(
                temp.path().join("history"),
            ),
        );
        let task_graph = Arc::new(
            super::super::task_graph::TaskGraphServiceImpl::with_history_and_shadow(
                history,
                Arc::clone(&store),
            ),
        );
        let svc = make_svc_with_task_graph(Arc::clone(&task_graph));
        let build_id = "rb-shadow-source";
        let output_dir = temp.path().join("shadow-output");

        register_chain(&svc, build_id, &[(":staleFallback", "StaleTask", &[])]).await;

        store
            .persist_plan(
                &super::super::build_plan_ir::CanonicalBuildPlan {
                    schema_version: super::super::build_plan_ir::BUILD_PLAN_SCHEMA_VERSION,
                    build_id: build_id.to_string(),
                    projects: Vec::new(),
                    tasks: vec![super::super::build_plan_ir::CanonicalBuildPlanTask {
                        path: ":fromShadow".to_string(),
                        project_path: ":".to_string(),
                        implementation_id: "Mkdir".to_string(),
                        depends_on: Vec::new(),
                        inputs: Default::default(),
                        outputs: Vec::new(),
                        worker_isolation: "compat-jvm".to_string(),
                        should_run_after: Vec::new(),
                        must_run_after: Vec::new(),
                        finalized_by: Vec::new(),
                        cacheability: "unknown".to_string(),
                        local_state: Vec::new(),
                        destroyables: Vec::new(),
                        action_kind: "mkdir".to_string(),
                        input_specs: Vec::new(),
                        output_specs: vec![
                            super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                                name: "directory".to_string(),
                                kind: "directory".to_string(),
                                path: output_dir.to_string_lossy().into_owned(),
                            },
                        ],
                        environment_inputs: Vec::new(),
                        system_property_inputs: Vec::new(),
                        diagnostics: Vec::new(),
                    }],
                    dependencies: Vec::new(),
                    toolchains: Vec::new(),
                    metadata: Default::default(),
                },
                "test-shadow",
            )
            .unwrap();

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: build_id.to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: HashMap::new(),
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.plan_source, "build-plan-shadow");
        assert_eq!(resp.total_tasks, 1);
        assert_eq!(resp.tasks_forwarded_to_jvm, 0);
        assert_eq!(resp.tasks_succeeded, 1);
        assert_eq!(resp.task_details.len(), 1);
        assert_eq!(resp.task_details[0].task_path, ":fromShadow");
        assert_eq!(resp.task_details[0].task_type, "Mkdir");
        assert!(
            output_dir.exists(),
            "shadow IR context should create the directory"
        );
    }

    #[tokio::test]
    async fn test_run_build_rejects_unsupported_cached_shadow_plan_before_dispatch() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(super::super::build_plan_shadow::BuildPlanShadowStore::new(
            temp.path().to_path_buf(),
        ));
        let history = Arc::new(
            super::super::execution_history::ExecutionHistoryServiceImpl::new(
                temp.path().join("history"),
            ),
        );
        let task_graph = Arc::new(
            super::super::task_graph::TaskGraphServiceImpl::with_history_and_shadow(
                history,
                Arc::clone(&store),
            ),
        );
        let svc = make_svc_with_task_graph(Arc::clone(&task_graph));
        let build_id = "rb-shadow-unsupported";

        register_chain(&svc, build_id, &[(":staleFallback", "StaleTask", &[])]).await;

        store
            .persist_plan(
                &super::super::build_plan_ir::CanonicalBuildPlan {
                    schema_version: super::super::build_plan_ir::BUILD_PLAN_SCHEMA_VERSION,
                    build_id: build_id.to_string(),
                    projects: Vec::new(),
                    tasks: vec![super::super::build_plan_ir::CanonicalBuildPlanTask {
                        path: ":unsafeMkdir".to_string(),
                        project_path: ":".to_string(),
                        implementation_id: "Mkdir".to_string(),
                        depends_on: Vec::new(),
                        inputs: [(
                            "copy_unsupported_custom_actions".to_string(),
                            "true".to_string(),
                        )]
                        .into_iter()
                        .collect(),
                        outputs: Vec::new(),
                        worker_isolation: "compat-jvm".to_string(),
                        should_run_after: Vec::new(),
                        must_run_after: Vec::new(),
                        finalized_by: Vec::new(),
                        cacheability: "unknown".to_string(),
                        local_state: Vec::new(),
                        destroyables: Vec::new(),
                        action_kind: "mkdir".to_string(),
                        input_specs: Vec::new(),
                        output_specs: Vec::new(),
                        environment_inputs: Vec::new(),
                        system_property_inputs: Vec::new(),
                        diagnostics: Vec::new(),
                    }],
                    dependencies: Vec::new(),
                    toolchains: Vec::new(),
                    metadata: Default::default(),
                },
                "test-shadow",
            )
            .unwrap();

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: build_id.to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: HashMap::new(),
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "FAILED");
        assert_eq!(resp.plan_source, "build-plan-shadow");
        assert_eq!(resp.tasks_forwarded_to_jvm, 0);
        assert_eq!(resp.task_details.len(), 0);
        assert!(
            resp.failure_message
                .contains("unsupported contract marker 'copy_unsupported_custom_actions'"),
            "unexpected failure message: {}",
            resp.failure_message
        );
    }

    #[tokio::test]
    async fn test_run_build_executes_shadow_java_compile_natively() {
        let java_home = match std::env::var("JAVA_HOME") {
            Ok(value) => value,
            Err(_) => return,
        };
        let temp = tempfile::tempdir().unwrap();
        let source_dir = temp.path().join("src/main/java");
        let source_file = source_dir.join("HelloFromShadow.java");
        let output_dir = temp.path().join("build/classes/java/main");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(
            &source_file,
            "public class HelloFromShadow { public String value() { return \"native\"; } }",
        )
        .unwrap();

        let store = Arc::new(super::super::build_plan_shadow::BuildPlanShadowStore::new(
            temp.path().join("shadow-store"),
        ));
        let history = Arc::new(
            super::super::execution_history::ExecutionHistoryServiceImpl::new(
                temp.path().join("history"),
            ),
        );
        let task_graph = Arc::new(
            super::super::task_graph::TaskGraphServiceImpl::with_history_and_shadow(
                history,
                Arc::clone(&store),
            ),
        );
        let svc = make_svc_with_task_graph(Arc::clone(&task_graph));
        let build_id = "rb-shadow-java-compile";

        store
            .persist_plan(
                &super::super::build_plan_ir::CanonicalBuildPlan {
                    schema_version: super::super::build_plan_ir::BUILD_PLAN_SCHEMA_VERSION,
                    build_id: build_id.to_string(),
                    projects: Vec::new(),
                    tasks: vec![super::super::build_plan_ir::CanonicalBuildPlanTask {
                        path: ":compileJava".to_string(),
                        project_path: ":".to_string(),
                        implementation_id: "org.gradle.api.tasks.compile.JavaCompile".to_string(),
                        depends_on: Vec::new(),
                        inputs: Default::default(),
                        outputs: Vec::new(),
                        worker_isolation: "process".to_string(),
                        should_run_after: Vec::new(),
                        must_run_after: Vec::new(),
                        finalized_by: Vec::new(),
                        cacheability: "unknown".to_string(),
                        local_state: Vec::new(),
                        destroyables: Vec::new(),
                        action_kind: "compile".to_string(),
                        input_specs: vec![
                            super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                                name: "source0".to_string(),
                                kind: "source".to_string(),
                                value: source_file.to_string_lossy().into_owned(),
                                normalization: "absolute-path".to_string(),
                                optional: false,
                            },
                            super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                                name: "java_home".to_string(),
                                kind: "value".to_string(),
                                value: java_home,
                                normalization: "scalar".to_string(),
                                optional: false,
                            },
                        ],
                        output_specs: vec![
                            super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                                name: "classes".to_string(),
                                kind: "directory".to_string(),
                                path: output_dir.to_string_lossy().into_owned(),
                            },
                        ],
                        environment_inputs: Vec::new(),
                        system_property_inputs: Vec::new(),
                        diagnostics: Vec::new(),
                    }],
                    dependencies: Vec::new(),
                    toolchains: Vec::new(),
                    metadata: Default::default(),
                },
                "test-shadow",
            )
            .unwrap();

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: build_id.to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: HashMap::new(),
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.plan_source, "build-plan-shadow");
        assert_eq!(resp.tasks_succeeded, 1);
        assert_eq!(resp.tasks_forwarded_to_jvm, 0);
        assert_eq!(resp.task_details[0].task_type, "JavaCompile");
        assert_eq!(resp.task_details[0].execution_mode, "rust_process");
        assert!(
            output_dir.join("HelloFromShadow.class").exists(),
            "shadow JavaCompile should produce a class file through Rust javac execution"
        );
    }

    #[tokio::test]
    async fn test_run_build_native_mkdir() {
        let svc = make_svc();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("output");

        register_chain(&svc, "rb-mkdir", &[(":createDir", "Mkdir", &[])]).await;

        let ctx = serde_json::json!({
            "source_files": [target.to_string_lossy()],
            "target_dir": "",
            "options": {}
        })
        .to_string();

        let mut contexts = HashMap::new();
        contexts.insert(":createDir".to_string(), ctx);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-mkdir".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.total_tasks, 1);
        assert_eq!(resp.tasks_succeeded, 1);
        assert!(target.exists(), "Mkdir should create the directory");
    }

    #[tokio::test]
    async fn test_run_build_native_copy() {
        let svc = make_svc();
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src_file.txt");
        let target_dir = dir.path().join("dest");
        std::fs::write(&src, "hello world").unwrap();

        register_chain(&svc, "rb-copy", &[(":copyFiles", "Copy", &[])]).await;

        let ctx = serde_json::json!({
            "source_files": [src.to_string_lossy()],
            "target_dir": target_dir.to_string_lossy(),
            "options": {}
        })
        .to_string();

        let mut contexts = HashMap::new();
        contexts.insert(":copyFiles".to_string(), ctx);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-copy".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.tasks_succeeded, 1);
        assert!(target_dir.exists(), "Copy should create target dir");
    }

    #[tokio::test]
    async fn test_run_build_native_delete() {
        let svc = make_svc();
        let dir = tempfile::tempdir().unwrap();
        let stale_dir = dir.path().join("build");
        let stale_file = stale_dir.join("stale.txt");
        std::fs::create_dir_all(&stale_dir).unwrap();
        std::fs::write(&stale_file, "delete me").unwrap();

        register_chain(&svc, "rb-delete", &[(":clean", "Delete", &[])]).await;

        let ctx = serde_json::json!({
            "source_files": [stale_dir.to_string_lossy()],
            "target_dir": "",
            "options": {}
        })
        .to_string();

        let mut contexts = HashMap::new();
        contexts.insert(":clean".to_string(), ctx);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-delete".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.tasks_succeeded, 1);
        assert!(
            !stale_dir.exists(),
            "Delete should remove destroyable paths"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_run_build_native_exec() {
        let svc = make_svc();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("generated.txt");

        register_chain(&svc, "rb-exec", &[(":generateFile", "Exec", &[])]).await;

        let ctx = serde_json::json!({
            "source_files": [],
            "target_dir": "",
            "options": {
                "executable": "/usr/bin/touch",
                "args": target.to_string_lossy(),
                "ignore_exit_value": "false"
            }
        })
        .to_string();

        let mut contexts = HashMap::new();
        contexts.insert(":generateFile".to_string(), ctx);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-exec".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.tasks_succeeded, 1);
        assert!(
            target.exists(),
            "Exec should run through the native task executor"
        );
    }

    #[tokio::test]
    async fn test_run_build_diamond_jvm_forward() {
        let (svc, _jvm_host_dir) = make_svc_with_mock_jvm_host().await;
        //    :root
        //   /     \
        //  :left  :right
        //   \     /
        //    :join
        register_chain(
            &svc,
            "rb-diamond",
            &[
                (":root", "UnknownTask", &[]),
                (":left", "UnknownTask", &[":root"]),
                (":right", "UnknownTask", &[":root"]),
                (":join", "UnknownTask", &[":left", ":right"]),
            ],
        )
        .await;

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-diamond".to_string(),
                max_parallelism: 4,
                task_filter: vec![],
                task_contexts: Default::default(),
                allow_jvm_forwarding: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.total_tasks, 4);
        assert_eq!(resp.tasks_forwarded_to_jvm, 4);
    }

    #[tokio::test]
    async fn test_run_build_mixed_native_and_jvm() {
        let (svc, _jvm_host_dir) = make_svc_with_mock_jvm_host().await;
        let dir = tempfile::tempdir().unwrap();
        let mkdir_target = dir.path().join("classes");

        register_chain(
            &svc,
            "rb-mixed",
            &[
                (":mkdir", "Mkdir", &[]),
                (":compileJava", "JavaCompile", &[":mkdir"]),
                (":processResources", "Copy", &[":mkdir"]),
                (
                    ":classes",
                    "UnknownTask",
                    &[":compileJava", ":processResources"],
                ),
            ],
        )
        .await;

        let mkdir_ctx = serde_json::json!({
            "source_files": [mkdir_target.to_string_lossy()],
            "target_dir": ""
        })
        .to_string();

        let copy_ctx = serde_json::json!({
            "source_files": [],
            "target_dir": dir.path().join("resources").to_string_lossy()
        })
        .to_string();

        let mut contexts = HashMap::new();
        contexts.insert(":mkdir".to_string(), mkdir_ctx);
        contexts.insert(":processResources".to_string(), copy_ctx);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-mixed".to_string(),
                max_parallelism: 4,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.total_tasks, 4);
        // :mkdir, :processResources, and :compileJava all have native executors.
        // :classes has no native executor → JVM-forwarded.
        assert_eq!(resp.tasks_forwarded_to_jvm, 1);
        let native_count = resp
            .task_details
            .iter()
            .filter(|d| d.execution_mode.starts_with("rust_"))
            .count();
        assert_eq!(native_count, 3);
        assert!(mkdir_target.exists(), "Mkdir should have run");
    }

    #[tokio::test]
    async fn test_run_build_failure_propagation() {
        let svc = make_svc();
        let dir = tempfile::tempdir().unwrap();
        // :copy will fail (nonexistent source), :downstream should be skipped
        let bad_target = dir.path().join("nowhere");

        register_chain(
            &svc,
            "rb-fail",
            &[(":copy", "Copy", &[]), (":downstream", "Mkdir", &[":copy"])],
        )
        .await;

        let ctx = serde_json::json!({
            "source_files": ["/nonexistent/file.txt"],
            "target_dir": bad_target.to_string_lossy()
        })
        .to_string();

        let mut contexts = HashMap::new();
        contexts.insert(":copy".to_string(), ctx);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-fail".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "FAILED");
        assert_eq!(resp.total_tasks, 2);
        assert_eq!(resp.tasks_failed, 1);
        assert_eq!(resp.tasks_skipped, 1);
        assert_eq!(resp.task_details.len(), 2);
        assert!(resp
            .task_details
            .iter()
            .any(|detail| detail.task_path == ":downstream"
                && detail.outcome == "SKIPPED"
                && detail.execution_mode == "skipped"));
        assert!(!resp.failure_message.is_empty());
    }

    #[tokio::test]
    async fn test_run_build_empty_graph() {
        let svc = make_svc();

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-empty".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: Default::default(),
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "FAILED");
        assert_eq!(resp.total_tasks, 0);
    }

    #[tokio::test]
    async fn test_run_build_parallel_native_tasks() {
        let svc = make_svc();
        let dir = tempfile::tempdir().unwrap();

        // Three independent Mkdir tasks should run in parallel
        register_chain(
            &svc,
            "rb-par",
            &[
                (":dir1", "Mkdir", &[]),
                (":dir2", "Mkdir", &[]),
                (":dir3", "Mkdir", &[]),
            ],
        )
        .await;

        let mut contexts = HashMap::new();
        for (i, name) in ["dir1", "dir2", "dir3"].iter().enumerate() {
            let dir_path = dir.path().join(format!("out{}", i));
            let ctx = serde_json::json!({
                "source_files": [dir_path.to_string_lossy()],
                "target_dir": ""
            })
            .to_string();
            contexts.insert(format!(":{}", name), ctx);
        }

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-par".to_string(),
                max_parallelism: 3,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.final_status, "COMPLETED");
        assert_eq!(resp.total_tasks, 3);
        assert_eq!(resp.tasks_succeeded, 3);
        assert!(dir.path().join("out0").exists());
        assert!(dir.path().join("out1").exists());
        assert!(dir.path().join("out2").exists());
    }

    #[tokio::test]
    async fn test_run_build_returns_task_details() {
        let (svc, _jvm_host_dir) = make_svc_with_mock_jvm_host().await;

        register_chain(
            &svc,
            "rb-details",
            &[(":a", "UnknownTask", &[]), (":b", "Mkdir", &[":a"])],
        )
        .await;

        let ctx = serde_json::json!({"source_files": ["/tmp/rb-details-test"], "target_dir": ""})
            .to_string();
        let mut contexts = HashMap::new();
        contexts.insert(":b".to_string(), ctx);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "rb-details".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.task_details.len(), 2);
        assert_eq!(resp.task_details[0].task_path, ":a");
        assert_eq!(resp.task_details[0].execution_mode, "mock-jvm:UnknownTask");
        assert_eq!(resp.task_details[1].task_path, ":b");
        assert_eq!(resp.task_details[1].execution_mode, "rust_in_process");
        assert!(resp.task_details[1].duration_ms >= 0);
    }

    /// Test that tasks with matching execution history are skipped as UP-TO-DATE.
    #[tokio::test]
    async fn test_run_build_skips_up_to_date_tasks() {
        let svc = make_svc();

        register_chain(&svc, "build-utd", &[(":compileJava", "JavaCompile", &[])]).await;

        let dir = tempfile::tempdir().unwrap();

        // First run: execute the task (no history → always executes)
        let ctx1 = serde_json::json!({
            "work_identity": ":project:compileJava",
            "display_name": ":project:compileJava",
            "implementation_class": "org.gradle.api.tasks.compile.JavaCompile",
            "input_properties": {"classpath": "libs/a.jar"},
            "input_file_fingerprints": {"src/Main.java": "abc123"},
            "caching_enabled": false,
            "can_load_from_cache": false,
            "up_to_date_enabled": true,
            "has_previous_execution_state": false,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("src").to_string_lossy()],
            "target_dir": dir.path().join("classes").to_string_lossy()
        })
        .to_string();

        let mut contexts1 = HashMap::new();
        contexts1.insert(":compileJava".to_string(), ctx1);

        let resp1 = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-utd".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts1,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp1.total_tasks, 1);
        // First run: no history, so it executes (JavaCompile has no real javac, so it may fail
        // or succeed depending on the environment — either way, history is recorded)

        // Second run: same inputs → should be UP-TO-DATE
        register_chain(&svc, "build-utd-2", &[(":compileJava", "JavaCompile", &[])]).await;

        let ctx2 = serde_json::json!({
            "work_identity": ":project:compileJava",
            "display_name": ":project:compileJava",
            "implementation_class": "org.gradle.api.tasks.compile.JavaCompile",
            "input_properties": {"classpath": "libs/a.jar"},
            "input_file_fingerprints": {"src/Main.java": "abc123"},
            "caching_enabled": false,
            "can_load_from_cache": false,
            "up_to_date_enabled": true,
            "has_previous_execution_state": true,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("src").to_string_lossy()],
            "target_dir": dir.path().join("classes").to_string_lossy()
        })
        .to_string();

        let mut contexts2 = HashMap::new();
        contexts2.insert(":compileJava".to_string(), ctx2);

        let resp2 = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-utd-2".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts2,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp2.total_tasks, 1);
        // Second run with same fingerprint should be UP-TO-DATE
        assert_eq!(
            resp2.tasks_up_to_date, 1,
            "task should be UP-TO-DATE on second run"
        );
        assert_eq!(resp2.tasks_succeeded, 1);
    }

    #[tokio::test]
    async fn test_run_build_vfs_delta_intersecting_input_prevents_up_to_date_skip() {
        let svc = make_svc();

        register_chain(
            &svc,
            "build-vfs-delta",
            &[(":compileJava", "JavaCompile", &[])],
        )
        .await;

        let dir = tempfile::tempdir().unwrap();
        let base_ctx = serde_json::json!({
            "work_identity": ":project:compileJava",
            "display_name": ":project:compileJava",
            "implementation_class": "org.gradle.api.tasks.compile.JavaCompile",
            "input_properties": {"classpath": "libs/a.jar"},
            "input_file_fingerprints": {"src/Main.java": "abc123"},
            "caching_enabled": false,
            "can_load_from_cache": false,
            "up_to_date_enabled": true,
            "has_previous_execution_state": true,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("src").to_string_lossy()],
            "target_dir": dir.path().join("classes").to_string_lossy()
        });

        let mut contexts1 = HashMap::new();
        contexts1.insert(":compileJava".to_string(), base_ctx.to_string());

        let _ = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-vfs-delta".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts1,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        register_chain(
            &svc,
            "build-vfs-delta-2",
            &[(":compileJava", "JavaCompile", &[])],
        )
        .await;

        let mut changed_ctx = base_ctx;
        changed_ctx["trusted_vfs_delta"] = serde_json::Value::Bool(true);
        changed_ctx["trusted_changed_paths"] = serde_json::json!(["src/Main.java"]);
        let mut contexts2 = HashMap::new();
        contexts2.insert(":compileJava".to_string(), changed_ctx.to_string());

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-vfs-delta-2".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts2,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(
            resp.tasks_up_to_date, 0,
            "trusted VFS delta intersecting an input should force execution"
        );
    }

    #[tokio::test]
    async fn test_run_build_daemon_vfs_delta_prevents_up_to_date_skip() {
        let store = Arc::new(VfsDeltaStore::default());
        let svc = make_svc().with_vfs_delta_store(Arc::clone(&store));

        register_chain(
            &svc,
            "build-daemon-vfs-delta",
            &[(":compileJava", "JavaCompile", &[])],
        )
        .await;

        let dir = tempfile::tempdir().unwrap();
        let base_ctx = serde_json::json!({
            "work_identity": ":project:compileJava",
            "display_name": ":project:compileJava",
            "implementation_class": "org.gradle.api.tasks.compile.JavaCompile",
            "input_properties": {"classpath": "libs/a.jar"},
            "input_file_fingerprints": {"src/Main.java": "abc123"},
            "caching_enabled": false,
            "can_load_from_cache": false,
            "up_to_date_enabled": true,
            "has_previous_execution_state": true,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("src").to_string_lossy()],
            "target_dir": dir.path().join("classes").to_string_lossy()
        });

        let mut contexts1 = HashMap::new();
        contexts1.insert(":compileJava".to_string(), base_ctx.to_string());

        let _ = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-daemon-vfs-delta".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts1,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        store.record(FileChangeEvent {
            path: "src/Main.java".to_string(),
            change_type: "MODIFIED".to_string(),
            timestamp_ms: 1,
            file_size: 0,
            is_directory: false,
        });

        register_chain(
            &svc,
            "build-daemon-vfs-delta-2",
            &[(":compileJava", "JavaCompile", &[])],
        )
        .await;

        let mut contexts2 = HashMap::new();
        contexts2.insert(":compileJava".to_string(), base_ctx.to_string());

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-daemon-vfs-delta-2".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts2,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(
            resp.tasks_up_to_date, 0,
            "daemon-retained VFS delta intersecting an input should force execution"
        );
    }

    /// Test that UP-TO-DATE count is reflected in RunBuildResponse.
    #[tokio::test]
    async fn test_run_build_up_to_date_counted_in_response() {
        let svc = make_svc();

        register_chain(
            &svc,
            "build-utd-count",
            &[(":task1", "Mkdir", &[]), (":task2", "Mkdir", &[])],
        )
        .await;

        let dir = tempfile::tempdir().unwrap();

        // Run once to establish history
        let ctx1 = serde_json::json!({
            "work_identity": ":project:task1",
            "input_properties": {"key": "value"},
            "input_file_fingerprints": {"f": "hash1"},
            "up_to_date_enabled": true,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("t1").to_string_lossy()],
            "target_dir": ""
        })
        .to_string();

        let ctx2 = serde_json::json!({
            "work_identity": ":project:task2",
            "input_properties": {"key": "value"},
            "input_file_fingerprints": {"f": "hash2"},
            "up_to_date_enabled": true,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("t2").to_string_lossy()],
            "target_dir": ""
        })
        .to_string();

        let mut contexts1 = HashMap::new();
        contexts1.insert(":task1".to_string(), ctx1);
        contexts1.insert(":task2".to_string(), ctx2);

        let _ = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-utd-count".to_string(),
                max_parallelism: 2,
                task_filter: vec![],
                task_contexts: contexts1,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Second run — both should be UP-TO-DATE
        register_chain(
            &svc,
            "build-utd-count-2",
            &[(":task1", "Mkdir", &[]), (":task2", "Mkdir", &[])],
        )
        .await;

        let ctx1b = serde_json::json!({
            "work_identity": ":project:task1",
            "input_properties": {"key": "value"},
            "input_file_fingerprints": {"f": "hash1"},
            "up_to_date_enabled": true,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("t1").to_string_lossy()],
            "target_dir": ""
        })
        .to_string();

        let ctx2b = serde_json::json!({
            "work_identity": ":project:task2",
            "input_properties": {"key": "value"},
            "input_file_fingerprints": {"f": "hash2"},
            "up_to_date_enabled": true,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("t2").to_string_lossy()],
            "target_dir": ""
        })
        .to_string();

        let mut contexts2 = HashMap::new();
        contexts2.insert(":task1".to_string(), ctx1b);
        contexts2.insert(":task2".to_string(), ctx2b);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-utd-count-2".to_string(),
                max_parallelism: 2,
                task_filter: vec![],
                task_contexts: contexts2,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.tasks_up_to_date, 2);
        assert_eq!(resp.tasks_from_cache, 0);
        assert_eq!(resp.tasks_succeeded, 2);
    }

    #[tokio::test]
    async fn test_run_build_executes_cache_candidate_until_outputs_are_restored() {
        let svc = make_svc();

        register_chain(&svc, "build-cache-candidate", &[(":task", "Mkdir", &[])]).await;

        let dir = tempfile::tempdir().unwrap();
        let output_dir = dir.path().join("created-by-native-exec");
        let context = serde_json::json!({
            "work_identity": ":project:cacheCandidate",
            "display_name": ":project:cacheCandidate",
            "implementation_class": "org.gradle.api.DefaultTask",
            "input_properties": {"key": "value"},
            "input_file_fingerprints": {"input": "hash"},
            "caching_enabled": true,
            "can_load_from_cache": true,
            "up_to_date_enabled": true,
            "has_previous_execution_state": false,
            "rebuild_reasons": [],
            "source_files": [output_dir.to_string_lossy()],
            "output_files": [output_dir.to_string_lossy()]
        })
        .to_string();
        let mut contexts = HashMap::new();
        contexts.insert(":task".to_string(), context);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-cache-candidate".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.tasks_from_cache, 0);
        assert_eq!(resp.tasks_succeeded, 1);
        assert_eq!(resp.task_details[0].outcome, "EXECUTED");
        assert!(
            output_dir.is_dir(),
            "cache candidates must restore or execute"
        );
    }

    #[tokio::test]
    async fn test_run_build_restores_outputs_from_rust_local_cache() {
        let cache_dir = tempfile::tempdir().unwrap();
        let local_cache = Arc::new(LocalCacheStore::new(cache_dir.path().to_path_buf()));
        let svc = make_svc().with_local_cache(Arc::clone(&local_cache));

        register_chain(&svc, "build-cache-hit", &[(":task", "Mkdir", &[])]).await;

        let dir = tempfile::tempdir().unwrap();
        let output_dir = dir.path().join("restored-output");
        let work_meta = WorkMetadata {
            work_identity: ":project:cacheHit".to_string(),
            display_name: ":project:cacheHit".to_string(),
            implementation_class: "org.gradle.api.DefaultTask".to_string(),
            input_properties: [("key".to_string(), "value".to_string())].into(),
            input_file_fingerprints: [("input".to_string(), "hash".to_string())].into(),
            caching_enabled: true,
            can_load_from_cache: true,
            has_previous_execution_state: false,
            rebuild_reasons: Vec::new(),
        };
        let cache_key =
            super::super::execution_plan::ExecutionPlanServiceImpl::compute_fingerprint(&work_meta);
        let (packaged_bytes, _) = BuildCachePackagingServiceImpl::pack(PackCacheEntryRequest {
            build_id: "build-cache-hit".to_string(),
            files: vec![BuildCachePackFile {
                path: "out0/restored.txt".to_string(),
                content: b"from rust cache".to_vec(),
                executable: false,
            }],
            origin_metadata: [(
                "output_kinds_json".to_string(),
                serde_json::json!(["dir"]).to_string(),
            )]
            .into(),
            gzip: true,
        })
        .unwrap();
        local_cache
            .store(&cache_key, &packaged_bytes)
            .await
            .unwrap();

        let context = serde_json::json!({
            "work_identity": work_meta.work_identity,
            "display_name": work_meta.display_name,
            "implementation_class": work_meta.implementation_class,
            "input_properties": {"key": "value"},
            "input_file_fingerprints": {"input": "hash"},
            "caching_enabled": true,
            "can_load_from_cache": true,
            "up_to_date_enabled": true,
            "has_previous_execution_state": false,
            "rebuild_reasons": [],
            "source_files": [output_dir.to_string_lossy()],
            "output_files": [output_dir.to_string_lossy()]
        })
        .to_string();
        let mut contexts = HashMap::new();
        contexts.insert(":task".to_string(), context);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-cache-hit".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.tasks_from_cache, 1);
        assert_eq!(resp.tasks_succeeded, 1);
        assert_eq!(resp.task_details[0].outcome, "FROM_CACHE");
        assert_eq!(
            std::fs::read_to_string(output_dir.join("restored.txt")).unwrap(),
            "from rust cache"
        );
    }

    #[tokio::test]
    async fn test_run_build_stores_native_outputs_in_rust_local_cache() {
        let cache_dir = tempfile::tempdir().unwrap();
        let local_cache = Arc::new(LocalCacheStore::new(cache_dir.path().to_path_buf()));
        let svc = make_svc().with_local_cache(Arc::clone(&local_cache));

        register_chain(&svc, "build-cache-store", &[(":task", "Mkdir", &[])]).await;

        let dir = tempfile::tempdir().unwrap();
        let output_dir = dir.path().join("stored-output");
        let work_meta = WorkMetadata {
            work_identity: ":project:cacheStore".to_string(),
            display_name: ":project:cacheStore".to_string(),
            implementation_class: "org.gradle.api.DefaultTask".to_string(),
            input_properties: [("key".to_string(), "value".to_string())].into(),
            input_file_fingerprints: [("input".to_string(), "hash".to_string())].into(),
            caching_enabled: true,
            can_load_from_cache: true,
            has_previous_execution_state: false,
            rebuild_reasons: Vec::new(),
        };
        let cache_key =
            super::super::execution_plan::ExecutionPlanServiceImpl::compute_fingerprint(&work_meta);
        let context = serde_json::json!({
            "work_identity": work_meta.work_identity,
            "display_name": work_meta.display_name,
            "implementation_class": work_meta.implementation_class,
            "input_properties": {"key": "value"},
            "input_file_fingerprints": {"input": "hash"},
            "caching_enabled": true,
            "can_load_from_cache": true,
            "up_to_date_enabled": true,
            "has_previous_execution_state": false,
            "rebuild_reasons": [],
            "source_files": [output_dir.to_string_lossy()],
            "output_files": [output_dir.to_string_lossy()]
        })
        .to_string();
        let mut contexts = HashMap::new();
        contexts.insert(":task".to_string(), context);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-cache-store".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.tasks_from_cache, 0);
        assert_eq!(resp.task_details[0].outcome, "EXECUTED");
        assert!(local_cache.contains(&cache_key).await.unwrap());
    }

    #[tokio::test]
    async fn test_run_build_skips_no_source_tasks_without_executor_work() {
        let svc = make_svc();

        register_chain(
            &svc,
            "build-no-source",
            &[(":processResources", "Copy", &[])],
        )
        .await;

        let context = serde_json::json!({
            "no_source": true,
            "source_files": ["/definitely/missing/src/main/resources"],
            "target_dir": "/definitely/missing/build/resources/main",
            "output_files": ["/definitely/missing/build/resources/main"],
            "up_to_date_enabled": true
        })
        .to_string();
        let mut contexts = HashMap::new();
        contexts.insert(":processResources".to_string(), context);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-no-source".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_tasks, 1);
        assert_eq!(resp.tasks_failed, 0);
        assert_eq!(resp.tasks_succeeded, 1);
        assert_eq!(resp.tasks_skipped, 1);
        assert_eq!(resp.task_details[0].outcome, "NO_SOURCE");
        assert_eq!(resp.task_details[0].execution_mode, "skipped");
    }

    /// Test that tasks without work_metadata in context always execute.
    #[tokio::test]
    async fn test_run_build_no_metadata_always_executes() {
        let (svc, _jvm_host_dir) = make_svc_with_mock_jvm_host().await;

        register_chain(&svc, "build-no-meta", &[(":a", "UnknownTask", &[])]).await;

        // No task_contexts at all → no work_metadata → always execute
        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-no-meta".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: Default::default(),
                allow_jvm_forwarding: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_tasks, 1);
        assert_eq!(resp.tasks_up_to_date, 0);
        assert_eq!(resp.tasks_forwarded_to_jvm, 1);
    }

    /// Test that rebuild_reasons in metadata forces execution even with matching history.
    #[tokio::test]
    async fn test_run_build_rebuild_reason_forces_execution() {
        let svc = make_svc();

        register_chain(
            &svc,
            "build-rebuild",
            &[(":compileJava", "JavaCompile", &[])],
        )
        .await;

        let dir = tempfile::tempdir().unwrap();

        // First run: establish history
        let ctx1 = serde_json::json!({
            "work_identity": ":project:compileJava",
            "input_properties": {"cp": "old.jar"},
            "input_file_fingerprints": {"src/A.java": "aaa"},
            "up_to_date_enabled": true,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("src").to_string_lossy()],
            "target_dir": dir.path().join("classes").to_string_lossy()
        })
        .to_string();

        let mut contexts1 = HashMap::new();
        contexts1.insert(":compileJava".to_string(), ctx1);

        let _ = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-rebuild".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts1,
                allow_jvm_forwarding: false,
            }))
            .await;

        // Second run: same inputs but with rebuild_reasons → must execute
        register_chain(
            &svc,
            "build-rebuild-2",
            &[(":compileJava", "JavaCompile", &[])],
        )
        .await;

        let ctx2 = serde_json::json!({
            "work_identity": ":project:compileJava",
            "input_properties": {"cp": "old.jar"},
            "input_file_fingerprints": {"src/A.java": "aaa"},
            "up_to_date_enabled": true,
            "rebuild_reasons": ["output file deleted"],
            "source_files": [dir.path().join("src").to_string_lossy()],
            "target_dir": dir.path().join("classes").to_string_lossy()
        })
        .to_string();

        let mut contexts2 = HashMap::new();
        contexts2.insert(":compileJava".to_string(), ctx2);

        let resp = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-rebuild-2".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts2,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Should NOT be UP-TO-DATE despite matching fingerprint
        assert_eq!(
            resp.tasks_up_to_date, 0,
            "rebuild_reasons should force execution"
        );
        assert_eq!(resp.total_tasks, 1);
    }

    /// Test that execution plan receives record_outcome calls after task execution.
    #[tokio::test]
    async fn test_run_build_records_outcome_to_history() {
        let svc = make_svc();

        register_chain(&svc, "build-record", &[(":task", "Mkdir", &[])]).await;

        let dir = tempfile::tempdir().unwrap();

        let ctx = serde_json::json!({
            "work_identity": ":project:task",
            "input_properties": {"key": "val"},
            "input_file_fingerprints": {"f": "h1"},
            "up_to_date_enabled": true,
            "rebuild_reasons": [],
            "source_files": [dir.path().join("t").to_string_lossy()],
            "target_dir": ""
        })
        .to_string();

        let mut contexts = HashMap::new();
        contexts.insert(":task".to_string(), ctx.clone());

        // Run once — this should record outcome to execution plan history
        let _ = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-record".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Verify history was recorded by checking internal state
        let has_entry = svc
            .execution_plan
            .history
            .entries
            .contains_key(":project:task");

        assert!(
            has_entry,
            "execution plan should have recorded history for :project:task"
        );

        // Run again with same inputs → should be UP-TO-DATE (proves history is being used)
        register_chain(&svc, "build-record-2", &[(":task", "Mkdir", &[])]).await;

        let mut contexts2 = HashMap::new();
        contexts2.insert(":task".to_string(), ctx);

        let resp2 = svc
            .run_build(Request::new(RunBuildRequest {
                build_id: "build-record-2".to_string(),
                max_parallelism: 1,
                task_filter: vec![],
                task_contexts: contexts2,
                allow_jvm_forwarding: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp2.tasks_up_to_date, 1, "second run should be UP-TO-DATE");
    }

    #[test]
    fn test_vfs_delta_context_merges_with_shadow_work_metadata() {
        let base = serde_json::json!({
            "work_identity": ":project:compileJava",
            "input_file_fingerprints": {"src/Main.java": "hash"},
            "up_to_date_enabled": true
        })
        .to_string();
        let delta = serde_json::json!({
            "trusted_vfs_delta": true,
            "trusted_changed_paths": ["/repo/README.md"]
        })
        .to_string();

        let merged = merged_task_context(Some(&delta), Some(base)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(value["work_identity"], ":project:compileJava");
        assert_eq!(value["trusted_vfs_delta"], true);
        assert_eq!(value["trusted_changed_paths"][0], "/repo/README.md");
    }

    #[test]
    fn test_vfs_delta_intersecting_input_forces_rebuild_reason() {
        let context = serde_json::json!({
            "trusted_vfs_delta": true,
            "trusted_changed_paths": ["/repo/src/Main.java"]
        })
        .to_string();
        let mut meta = WorkMetadata {
            work_identity: ":compileJava".to_string(),
            display_name: ":compileJava".to_string(),
            implementation_class: "JavaCompile".to_string(),
            input_properties: Default::default(),
            input_file_fingerprints: [("/repo/src/Main.java".to_string(), "hash".to_string())]
                .into(),
            caching_enabled: false,
            can_load_from_cache: false,
            has_previous_execution_state: true,
            rebuild_reasons: Vec::new(),
        };

        apply_vfs_delta_to_work_metadata(&mut meta, Some(&context));

        assert!(meta
            .rebuild_reasons
            .iter()
            .any(|reason| reason.contains("VFS delta")));
    }

    #[test]
    fn test_vfs_delta_unrelated_input_preserves_work_fingerprint_inputs() {
        let context = serde_json::json!({
            "trusted_vfs_delta": true,
            "trusted_changed_paths": ["/repo/README.md"]
        })
        .to_string();
        let mut meta = WorkMetadata {
            work_identity: ":compileJava".to_string(),
            display_name: ":compileJava".to_string(),
            implementation_class: "JavaCompile".to_string(),
            input_properties: Default::default(),
            input_file_fingerprints: [("/repo/src/Main.java".to_string(), "hash".to_string())]
                .into(),
            caching_enabled: false,
            can_load_from_cache: false,
            has_previous_execution_state: true,
            rebuild_reasons: Vec::new(),
        };

        apply_vfs_delta_to_work_metadata(&mut meta, Some(&context));

        assert!(meta.rebuild_reasons.is_empty());
        assert!(meta.input_properties.is_empty());
    }

    #[test]
    fn test_refreshed_work_metadata_rehashes_source_files_after_execution() {
        let dir = tempfile::tempdir().unwrap();
        let generated = dir.path().join("generated.txt");
        let before =
            task_graph::work_input_file_fingerprints(&[generated.to_string_lossy().into_owned()]);

        std::fs::write(&generated, "generated after task execution\n").unwrap();
        let context = serde_json::json!({
            "source_files": [generated.to_string_lossy()]
        })
        .to_string();
        let meta = WorkMetadata {
            work_identity: ":jar".to_string(),
            display_name: ":jar".to_string(),
            implementation_class: "org.gradle.api.tasks.bundling.Jar".to_string(),
            input_properties: Default::default(),
            input_file_fingerprints: before.into_iter().collect(),
            caching_enabled: false,
            can_load_from_cache: false,
            has_previous_execution_state: false,
            rebuild_reasons: Vec::new(),
        };

        let refreshed = refreshed_work_metadata(&meta, &context);

        assert_ne!(
            meta.input_file_fingerprints,
            refreshed.input_file_fingerprints
        );
        assert_ne!(
            refreshed
                .input_file_fingerprints
                .get(generated.to_string_lossy().as_ref())
                .map(String::as_str),
            Some("missing")
        );
    }

    #[test]
    fn test_declared_outputs_present_allows_java_compile_optional_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let classes = dir.path().join("build/classes/java/main");
        std::fs::create_dir_all(&classes).unwrap();
        let context = serde_json::json!({
            "output_files": [
                classes.to_string_lossy(),
                dir.path().join("build/generated/sources/annotationProcessor/java/main").to_string_lossy(),
                dir.path().join("build/generated/sources/headers/java/main").to_string_lossy(),
                dir.path().join("build/tmp/compileJava/previous-compilation-data.bin").to_string_lossy()
            ]
        })
        .to_string();

        assert!(declared_outputs_present("JavaCompile", Some(&context)));
    }

    #[test]
    fn test_declared_outputs_present_requires_java_compile_real_output() {
        let dir = tempfile::tempdir().unwrap();
        let context = serde_json::json!({
            "output_files": [
                dir.path().join("build/classes/java/main").to_string_lossy(),
                dir.path().join("build/generated/sources/annotationProcessor/java/main").to_string_lossy()
            ]
        })
        .to_string();

        assert!(!declared_outputs_present("JavaCompile", Some(&context)));
    }

    #[test]
    fn test_declared_outputs_present_keeps_non_java_compile_strict() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("build/libs/app.jar");
        let missing = dir.path().join("build/tmp/missing-marker");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, b"jar").unwrap();
        let context = serde_json::json!({
            "output_files": [
                existing.to_string_lossy(),
                missing.to_string_lossy()
            ]
        })
        .to_string();

        assert!(!declared_outputs_present("Jar", Some(&context)));
    }
}
