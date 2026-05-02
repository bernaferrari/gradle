use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tonic::{Request, Response, Status};

use crate::proto::{
    task_graph_service_server::TaskGraphService, ClearBuildTasksRequest, ClearBuildTasksResponse,
    ExecutionNode, GetProgressRequest, GetProgressResponse, RegisterTaskRequest,
    RegisterTaskResponse, ResolveExecutionPlanRequest, ResolveExecutionPlanResponse,
    TaskFinishedRequest, TaskFinishedResponse, TaskProgress, TaskStartedRequest,
    TaskStartedResponse,
};

use super::build_plan_ir::{CanonicalBuildPlanDependency, CanonicalBuildPlanTask};
use super::build_plan_shadow::BuildPlanShadowStore;
use super::execution_history::ExecutionHistoryServiceImpl;
use super::scopes::BuildId;

/// Task graph node stored internally.
#[derive(Clone)]
struct TaskNode {
    task_path: String,
    depends_on: Vec<String>,
    should_execute: bool,
    task_type: String,
    execution_context_json: String,
    estimated_duration_ms: i64,
    status: String,
    start_time_ms: i64,
    duration_ms: i64,
}

/// Rust-native task graph service.
/// Manages dependency resolution and execution scheduling.
/// Tasks are scoped by (BuildId, task_path) to prevent concurrent builds from mixing state.
pub struct TaskGraphServiceImpl {
    tasks: Arc<DashMap<(BuildId, String), TaskNode>>,
    request_counter: Arc<AtomicI64>,
    /// Optional reference to execution history for duration estimates.
    history: Option<Arc<ExecutionHistoryServiceImpl>>,
    /// Reverse index: file path -> list of (build_id, task_path) that depend on it.
    /// Used by file-watch integration to invalidate tasks when their inputs change.
    file_to_tasks: Arc<DashMap<String, Vec<(BuildId, String)>>>,
    /// Optional persisted canonical build-plan source populated by the JVM host.
    build_plan_shadow_store: Option<Arc<BuildPlanShadowStore>>,
}

impl Clone for TaskGraphServiceImpl {
    fn clone(&self) -> Self {
        Self {
            tasks: Arc::clone(&self.tasks),
            request_counter: Arc::clone(&self.request_counter),
            history: self.history.clone(),
            file_to_tasks: Arc::clone(&self.file_to_tasks),
            build_plan_shadow_store: self.build_plan_shadow_store.clone(),
        }
    }
}

impl Default for TaskGraphServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskGraphServiceImpl {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(DashMap::new()),
            request_counter: Arc::new(AtomicI64::new(0)),
            history: None,
            file_to_tasks: Arc::new(DashMap::new()),
            build_plan_shadow_store: None,
        }
    }

    pub fn with_history(history: Arc<ExecutionHistoryServiceImpl>) -> Self {
        Self {
            tasks: Arc::new(DashMap::new()),
            request_counter: Arc::new(AtomicI64::new(0)),
            history: Some(history),
            file_to_tasks: Arc::new(DashMap::new()),
            build_plan_shadow_store: None,
        }
    }

    pub fn with_history_and_shadow(
        history: Arc<ExecutionHistoryServiceImpl>,
        build_plan_shadow_store: Arc<BuildPlanShadowStore>,
    ) -> Self {
        Self {
            tasks: Arc::new(DashMap::new()),
            request_counter: Arc::new(AtomicI64::new(0)),
            history: Some(history),
            file_to_tasks: Arc::new(DashMap::new()),
            build_plan_shadow_store: Some(build_plan_shadow_store),
        }
    }

    /// Look up estimated duration from execution history for a task path.
    fn lookup_historical_duration(&self, task_path: &str) -> i64 {
        match &self.history {
            Some(h) => h.get_task_duration(task_path),
            None => 0,
        }
    }

    /// Invalidate tasks whose input files have changed.
    /// Marks affected tasks as `should_execute = true` so they will be
    /// included in the next execution plan. Returns the number of tasks invalidated.
    pub fn invalidate_tasks_for_files(&self, changed_files: &[String]) -> usize {
        let mut invalidated = std::collections::HashSet::with_capacity(changed_files.len() * 4);
        for file_path in changed_files {
            if let Some(entries) = self.file_to_tasks.get(file_path) {
                for (bid, task_path) in entries.iter() {
                    let key = (bid.clone(), task_path.clone());
                    if !invalidated.contains(&key) {
                        if let Some(mut task) = self.tasks.get_mut(&key) {
                            task.should_execute = true;
                            invalidated.insert(key);
                        }
                    }
                }
            }
        }
        invalidated.len()
    }

    /// Remove all tasks and reverse-index entries for a given build_id.
    pub fn cleanup_build(&self, build_id: &BuildId) {
        self.tasks.retain(|(bid, _), _| bid != build_id);
        self.file_to_tasks.retain(|_, tasks| {
            tasks.retain(|(bid, _)| bid != build_id);
            !tasks.is_empty()
        });
    }

    fn build_task_count(&self, build_id: &BuildId) -> usize {
        self.tasks
            .iter()
            .filter(|entry| entry.key().0 == *build_id)
            .count()
    }

    fn has_registered_tasks(&self, build_id: &BuildId) -> bool {
        self.tasks.iter().any(|entry| entry.key().0 == *build_id)
    }

    fn hydrate_from_shadow_plan(
        &self,
        build_id: &BuildId,
        build_id_str: &str,
        replace_existing: bool,
    ) -> usize {
        let Some(store) = self.build_plan_shadow_store.as_ref() else {
            return 0;
        };
        let artifact = match store.load_plan(build_id_str) {
            Ok(Some(artifact)) => artifact,
            Ok(None) => return 0,
            Err(error) => {
                tracing::warn!(
                    build_id = %build_id_str,
                    error = %error,
                    "Failed loading build-plan shadow artifact for task graph hydration"
                );
                return 0;
            }
        };

        if replace_existing {
            self.cleanup_build(build_id);
        }

        let plan_dependencies = artifact.plan.dependencies.clone();
        let mut plan_tasks = artifact.plan.tasks;
        let task_outputs: HashMap<String, Vec<String>> = plan_tasks
            .iter()
            .map(|task| (task.path.clone(), task.outputs.clone()))
            .collect();

        let clean_tasks: Vec<String> = plan_tasks
            .iter()
            .filter(|task| is_clean_task_path(&task.path))
            .map(|task| task.path.clone())
            .collect();

        let mut loaded = 0usize;
        for mut task in plan_tasks.drain(..) {
            enrich_task_contract_from_graph(&mut task, &task_outputs, &plan_dependencies);
            if !clean_tasks.is_empty() && !is_clean_task_path(&task.path) {
                for clean_task in &clean_tasks {
                    if !task.depends_on.contains(clean_task) {
                        task.depends_on.push(clean_task.clone());
                    }
                }
            }
            let estimated = self.lookup_historical_duration(&task.path);
            let task_type = executable_task_type(&task);
            let execution_context_json = execution_context_json(&task, &task_type);
            self.tasks.insert(
                (build_id.clone(), task.path.clone()),
                TaskNode {
                    task_path: task.path,
                    depends_on: task.depends_on,
                    should_execute: true,
                    task_type,
                    execution_context_json,
                    estimated_duration_ms: estimated,
                    status: "PENDING".to_string(),
                    start_time_ms: 0,
                    duration_ms: 0,
                },
            );
            loaded += 1;
        }

        if loaded > 0 {
            tracing::info!(
                build_id = %build_id_str,
                task_count = loaded,
                source = %artifact.source,
                "Hydrated task graph from build-plan shadow artifact"
            );
        }
        loaded
    }

    /// Kahn's algorithm for topological sort with parallel scheduling.
    /// Tasks with `should_execute == false` are excluded from the execution order
    /// since they are already resolved (UP-TO-DATE, SKIPPED, etc.).
    /// Only resolves tasks belonging to the given build_id.
    fn resolve_plan(&self, build_id: &BuildId) -> (Vec<ExecutionNode>, i64, bool) {
        let build_task_count = self.tasks.iter().filter(|e| e.key().0 == *build_id).count();
        let mut in_degree: HashMap<String, usize> = HashMap::with_capacity(build_task_count);
        let mut dependents: HashMap<String, Vec<String>> = HashMap::with_capacity(build_task_count);
        let mut all_tasks: HashSet<String> = HashSet::with_capacity(build_task_count);
        let mut skipped_count = 0usize;
        let mut task_type_counts: HashMap<String, usize> = HashMap::new();

        // Build adjacency info (only tasks for this build)
        for entry in self.tasks.iter() {
            // Skip tasks from other builds
            if entry.key().0 != *build_id {
                continue;
            }

            let path = entry.task_path.clone();
            all_tasks.insert(path.clone());

            // Track task type distribution for logging
            *task_type_counts.entry(entry.task_type.clone()).or_default() += 1;

            // Skip tasks that should not execute — they are pre-resolved
            if !entry.should_execute {
                skipped_count += 1;
                tracing::debug!(
                    task_path = %entry.task_path,
                    task_type = %entry.task_type,
                    "Excluding non-executing task from execution plan"
                );
                continue;
            }

            in_degree.entry(path.clone()).or_insert(0);

            for dep in &entry.depends_on {
                if self.tasks.contains_key(&(build_id.clone(), dep.clone())) {
                    *in_degree.entry(path.clone()).or_insert(0) += 1;
                    dependents
                        .entry(dep.clone())
                        .or_default()
                        .push(path.clone());
                }
            }
        }

        if skipped_count > 0 {
            tracing::info!(
                skipped = skipped_count,
                total = all_tasks.len(),
                "Excluded non-executing tasks from execution plan"
            );
        }

        // Log task type distribution
        for (ty, count) in &task_type_counts {
            tracing::debug!(task_type = %ty, count = *count, "Task type in graph");
        }

        // Initialize queue with tasks that have no dependencies
        let mut queue: VecDeque<String> = VecDeque::new();
        for task in &all_tasks {
            if *in_degree.get(task).unwrap_or(&0) == 0 {
                queue.push_back(task.clone());
            }
        }

        let mut execution_order = Vec::with_capacity(all_tasks.len());
        let mut order = 0i64;
        let mut visited_count = 0;
        let mut has_cycles = false;

        while let Some(task) = queue.pop_front() {
            visited_count += 1;

            if let Some(entry) = self.tasks.get(&(build_id.clone(), task.clone())) {
                if entry.should_execute {
                    order += 1;
                    let estimated = entry.estimated_duration_ms;
                    execution_order.push(ExecutionNode {
                        task_path: entry.task_path.clone(),
                        dependencies: entry.depends_on.clone(),
                        execution_order: order,
                        estimated_duration_ms: estimated,
                        task_type: entry.task_type.clone(),
                        execution_context_json: entry.execution_context_json.clone(),
                    });
                }
            }

            // Reduce in-degree for dependents
            if let Some(deps) = dependents.get(&task) {
                for dep in deps {
                    if let Some(degree) = in_degree.get_mut(dep) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dep.clone());
                        }
                    }
                }
            }
        }

        // Only consider tasks that participate in the plan for cycle detection
        let participating: HashSet<String> = all_tasks
            .iter()
            .filter(|t| {
                self.tasks
                    .get(&(build_id.clone(), (*t).clone()))
                    .map(|n| n.should_execute)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        if visited_count != participating.len() {
            has_cycles = true;
        }

        // Calculate critical path (longest path through DAG)
        let critical_path_ms = self.calculate_critical_path(build_id);

        (execution_order, critical_path_ms, has_cycles)
    }

    fn calculate_critical_path(&self, build_id: &BuildId) -> i64 {
        // Topological sort via Kahn's algorithm, then DP for longest path.
        // DashMap iteration order is non-deterministic, so we must sort first.
        let build_task_count = self.tasks.iter().filter(|e| e.key().0 == *build_id).count();
        let mut in_degree: HashMap<String, usize> = HashMap::with_capacity(build_task_count);
        let mut dependents: HashMap<String, Vec<String>> = HashMap::with_capacity(build_task_count);

        for entry in self.tasks.iter() {
            // Skip tasks from other builds
            if entry.key().0 != *build_id {
                continue;
            }

            let path = entry.task_path.clone();
            in_degree.entry(path.clone()).or_insert(0);
            for dep in &entry.depends_on {
                if self.tasks.contains_key(&(build_id.clone(), dep.clone())) {
                    *in_degree.entry(path.clone()).or_insert(0) += 1;
                    dependents
                        .entry(dep.clone())
                        .or_default()
                        .push(path.clone());
                }
            }
        }

        let mut queue: VecDeque<String> = VecDeque::new();
        for (task, &deg) in &in_degree {
            if deg == 0 {
                queue.push_back(task.clone());
            }
        }

        let mut topo_order = Vec::with_capacity(in_degree.len());
        while let Some(task) = queue.pop_front() {
            topo_order.push(task.clone());
            if let Some(deps) = dependents.get(&task) {
                for dep in deps {
                    if let Some(degree) = in_degree.get_mut(dep) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dep.clone());
                        }
                    }
                }
            }
        }

        // DP: longest path in topological order
        let mut longest: HashMap<String, i64> = HashMap::with_capacity(build_task_count);
        for task in &topo_order {
            if let Some(entry) = self.tasks.get(&(build_id.clone(), task.clone())) {
                let mut max_dep = 0i64;
                for dep in &entry.depends_on {
                    max_dep = max_dep.max(longest.get(dep).copied().unwrap_or(0));
                }
                longest.insert(task.clone(), entry.estimated_duration_ms + max_dep);
            }
        }

        longest.values().copied().max().unwrap_or(0)
    }
}

fn executable_task_type(task: &CanonicalBuildPlanTask) -> String {
    let simple = task
        .implementation_id
        .rsplit('.')
        .next()
        .unwrap_or(task.implementation_id.as_str());
    let logical = logical_gradle_task_type(simple);
    match (task.action_kind.as_str(), logical.as_str()) {
        ("mkdir", _) | ("create-directory", _) | (_, "Mkdir") => "Mkdir".to_string(),
        ("compile", "JavaCompile") | (_, "JavaCompile") if java_compile_no_source(task) => {
            "Lifecycle".to_string()
        }
        ("compile", "JavaCompile") | (_, "JavaCompile") if java_compile_contract_complete(task) => {
            "JavaCompile".to_string()
        }
        ("compile", "JavaCompile") | (_, "JavaCompile") => {
            compat_task_type(task, "org.gradle.api.tasks.compile.JavaCompile")
        }
        ("archive", "Jar") | (_, "Jar") => "Jar".to_string(),
        ("archive", "Zip") | (_, "Zip") => "Zip".to_string(),
        ("archive", "War") | (_, "War") => "War".to_string(),
        ("archive", "Ear") | (_, "Ear") => "Ear".to_string(),
        ("archive", "Tar") | (_, "Tar") => "Tar".to_string(),
        ("test", "Test") | (_, "Test") if test_no_source(task) => "Lifecycle".to_string(),
        ("test", "Test") | (_, "Test") if test_exec_contract_complete(task) => {
            "TestExec".to_string()
        }
        ("test", "Test") | (_, "Test") => {
            compat_task_type(task, "org.gradle.api.tasks.testing.Test")
        }
        ("file-transform", "ProcessResources") | (_, "ProcessResources")
            if process_resources_no_source(task) =>
        {
            "Lifecycle".to_string()
        }
        ("file-transform", "ProcessResources") | (_, "ProcessResources")
            if process_resources_contract_complete(task) =>
        {
            "Copy".to_string()
        }
        ("file-transform", "Copy") | (_, "Copy") if copy_contract_complete(task) => {
            "Copy".to_string()
        }
        ("file-transform", "Sync") | (_, "Sync") if copy_contract_complete(task) => {
            "Sync".to_string()
        }
        ("delete", "Delete") | (_, "Delete") if has_destroyables(task) => "Delete".to_string(),
        ("external-process", "Exec") | (_, "Exec") if exec_contract_complete(task) => {
            "Exec".to_string()
        }
        ("external-process", "JavaExec") | (_, "JavaExec") if java_exec_contract_complete(task) => {
            "JavaExec".to_string()
        }
        ("start-scripts", "CreateStartScripts") | (_, "CreateStartScripts") => {
            "CreateStartScripts".to_string()
        }
        ("documentation", "Javadoc") | (_, "Javadoc") if javadoc_contract_complete(task) => {
            "Javadoc".to_string()
        }
        ("documentation", "Javadoc") | (_, "Javadoc") => {
            compat_task_type(task, "org.gradle.api.tasks.javadoc.Javadoc")
        }
        ("jvm-task", "DefaultTask") | (_, "DefaultTask")
            if static_write_file_contract_complete(task) =>
        {
            "WriteFile".to_string()
        }
        ("lifecycle", _) | (_, "Lifecycle") if no_task_actions(task) => "Lifecycle".to_string(),
        _ => task.implementation_id.clone(),
    }
}

fn logical_gradle_task_type(simple: &str) -> String {
    const KNOWN_TASK_TYPES: [&str; 20] = [
        "JavaCompile",
        "Jar",
        "Zip",
        "War",
        "Ear",
        "Tar",
        "Test",
        "ProcessResources",
        "Copy",
        "Sync",
        "Delete",
        "Exec",
        "JavaExec",
        "CreateStartScripts",
        "Javadoc",
        "Lifecycle",
        "Mkdir",
        "Symlink",
        "Help",
        "Wrapper",
    ];
    for task_type in KNOWN_TASK_TYPES {
        if simple == task_type || simple.starts_with(&format!("{task_type}_")) {
            return task_type.to_string();
        }
    }
    simple.to_string()
}

fn is_clean_task_path(path: &str) -> bool {
    path == ":clean" || path.ends_with(":clean")
}

fn enrich_task_contract_from_graph(
    task: &mut CanonicalBuildPlanTask,
    task_outputs: &HashMap<String, Vec<String>>,
    plan_dependencies: &[CanonicalBuildPlanDependency],
) {
    if is_java_compile_task(task) && !has_input_value(task, "classpath") {
        let classpath = task
            .depends_on
            .iter()
            .filter_map(|dependency| task_outputs.get(dependency))
            .flat_map(|outputs| outputs.iter())
            .filter(|output| is_java_classes_output(output))
            .cloned()
            .collect::<Vec<_>>();

        if !classpath.is_empty() {
            if let Ok(joined) = std::env::join_paths(classpath.iter().map(std::path::Path::new)) {
                set_value_input(
                    task,
                    "classpath",
                    joined.to_string_lossy().as_ref(),
                    "graph-inferred-project-classpath",
                    "classpath",
                );
            }
        }
    }

    if is_test_task(task) && !has_input_value(task, "classpath") && has_test_sources(task) {
        let classpath = test_class_dir_paths(task)
            .into_iter()
            .chain(
                task.depends_on
                    .iter()
                    .filter_map(|dependency| task_outputs.get(dependency))
                    .flat_map(|outputs| outputs.iter())
                    .filter(|output| is_java_classes_output(output))
                    .cloned(),
            )
            .collect::<Vec<_>>();

        if !classpath.is_empty() {
            if let Ok(joined) = std::env::join_paths(classpath.iter().map(std::path::Path::new)) {
                set_value_input(
                    task,
                    "classpath",
                    joined.to_string_lossy().as_ref(),
                    "graph-inferred-test-classpath",
                    "classpath",
                );
            }
        }
    }

    if is_java_compile_task(task) || is_test_task(task) {
        let existing = input_value(task, "classpath").unwrap_or_default();
        let external = external_classpath_candidates(task, plan_dependencies);
        if let Some(merged) = merge_classpaths(&existing, &external) {
            set_value_input(
                task,
                "classpath",
                &merged,
                "gradle-resolved-external-classpath",
                "classpath",
            );
        }
    }

    if is_distribution_archive_task(task) && !has_input_value(task, "graph_distribution_libs") {
        let libs = task
            .depends_on
            .iter()
            .filter_map(|dependency| task_outputs.get(dependency))
            .flat_map(|outputs| outputs.iter())
            .filter(|output| is_jar_output(output))
            .cloned()
            .collect::<Vec<_>>();

        if !libs.is_empty() {
            if let Ok(joined) = std::env::join_paths(libs.iter().map(std::path::Path::new)) {
                set_value_input(
                    task,
                    "graph_distribution_libs",
                    joined.to_string_lossy().as_ref(),
                    "graph-inferred-distribution-libs",
                    "classpath",
                );
            }
        }
    }
}

fn is_test_task(task: &CanonicalBuildPlanTask) -> bool {
    logical_gradle_task_type(
        task.implementation_id
            .rsplit('.')
            .next()
            .unwrap_or(task.implementation_id.as_str()),
    ) == "Test"
}

fn merge_classpaths(existing: &str, additional: &[String]) -> Option<String> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for path in std::env::split_paths(existing) {
        let value = path.to_string_lossy().into_owned();
        if !value.is_empty() && seen.insert(value.clone()) {
            entries.push(value);
        }
    }
    for value in additional {
        if !value.is_empty() && seen.insert(value.clone()) {
            entries.push(value.clone());
        }
    }
    if entries.is_empty() {
        return None;
    }
    std::env::join_paths(entries.iter().map(std::path::Path::new))
        .ok()
        .map(|paths| paths.to_string_lossy().into_owned())
}

fn external_classpath_candidates(
    task: &CanonicalBuildPlanTask,
    plan_dependencies: &[CanonicalBuildPlanDependency],
) -> Vec<String> {
    if is_test_task(task)
        && infer_project_root(task)
            .as_ref()
            .map(|root| !has_test_sources_under(root))
            .unwrap_or(false)
    {
        return Vec::new();
    }
    let include_test_scope = is_test_scoped_task(task);
    let coordinates = plan_dependencies
        .iter()
        .filter(|dependency| dependency.project_path == task.project_path)
        .filter(|dependency| {
            dependency_configuration_matches_task(&dependency.configuration, include_test_scope)
        })
        .filter_map(|dependency| parse_coordinate(&dependency.notation))
        .collect::<Vec<_>>();

    let mut seen = HashSet::new();
    coordinates
        .into_iter()
        .filter(|coordinate| seen.insert(coordinate.clone()))
        .flat_map(|(group, name, version)| gradle_module_cache_jars(&group, &name, &version))
        .collect()
}

fn dependency_configuration_matches_task(configuration: &str, include_test_scope: bool) -> bool {
    let normalized = configuration.to_ascii_lowercase();
    if normalized.starts_with("test") {
        return include_test_scope;
    }
    matches!(
        normalized.as_str(),
        "compileclasspath"
            | "runtimeclasspath"
            | "api"
            | "implementation"
            | "compileonly"
            | "runtimeonly"
    )
}

fn has_test_sources(task: &CanonicalBuildPlanTask) -> bool {
    infer_project_root(task)
        .as_ref()
        .map(|root| has_test_sources_under(root))
        .unwrap_or(false)
}

fn has_test_sources_under(project_root: &std::path::Path) -> bool {
    let test_source_dir = project_root.join("src").join("test").join("java");
    contains_java_source(&test_source_dir)
}

fn contains_java_source(path: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if contains_java_source(&path) {
                return true;
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "java")
        {
            return true;
        }
    }
    false
}

fn infer_project_root(task: &CanonicalBuildPlanTask) -> Option<std::path::PathBuf> {
    task.input_specs
        .iter()
        .map(|input| input.value.as_str())
        .chain(task.outputs.iter().map(String::as_str))
        .filter_map(|path| {
            path.split_once("/src/")
                .or_else(|| path.split_once("/build/"))
                .map(|(root, _)| std::path::PathBuf::from(root))
        })
        .next()
}

fn is_test_scoped_task(task: &CanonicalBuildPlanTask) -> bool {
    input_value(task, "taskName")
        .map(|name| name.to_ascii_lowercase().contains("test"))
        .unwrap_or(false)
        || is_test_task(task)
}

fn parse_coordinate(value: &str) -> Option<(String, String, String)> {
    let mut parts = value.split(':');
    let group = parts.next()?.trim();
    let name = parts.next()?.trim();
    let version = parts.next()?.trim();
    if group.is_empty() || name.is_empty() || version.is_empty() || parts.next().is_some() {
        return None;
    }
    Some((group.to_string(), name.to_string(), version.to_string()))
}

fn gradle_module_cache_jars(group: &str, name: &str, version: &str) -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let module_dir = std::path::PathBuf::from(home)
        .join(".gradle")
        .join("caches")
        .join("modules-2")
        .join("files-2.1")
        .join(group)
        .join(name)
        .join(version);
    let Ok(hash_dirs) = std::fs::read_dir(module_dir) else {
        return Vec::new();
    };
    let mut jars = Vec::new();
    for hash_dir in hash_dirs.flatten() {
        let Ok(entries) = std::fs::read_dir(hash_dir.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "jar") {
                jars.push(path.to_string_lossy().into_owned());
            }
        }
    }
    jars.sort();
    jars
}

fn is_distribution_archive_task(task: &CanonicalBuildPlanTask) -> bool {
    let simple = task
        .implementation_id
        .rsplit('.')
        .next()
        .unwrap_or(task.implementation_id.as_str());
    matches!(logical_gradle_task_type(simple).as_str(), "Zip" | "Tar")
        && input_value(task, "archive_file")
            .map(|path| path.replace('\\', "/").contains("/build/distributions/"))
            .unwrap_or(false)
}

fn is_jar_output(output: &str) -> bool {
    std::path::Path::new(output)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("jar"))
        .unwrap_or(false)
}

fn is_java_compile_task(task: &CanonicalBuildPlanTask) -> bool {
    let simple = task
        .implementation_id
        .rsplit('.')
        .next()
        .unwrap_or(task.implementation_id.as_str());
    logical_gradle_task_type(simple) == "JavaCompile"
}

fn is_java_classes_output(output: &str) -> bool {
    let normalized = output.replace('\\', "/");
    normalized.contains("/build/classes/") && std::path::Path::new(output).extension().is_none()
}

fn set_value_input(
    task: &mut CanonicalBuildPlanTask,
    name: &str,
    value: &str,
    source: &str,
    normalization: &str,
) {
    task.inputs.insert(name.to_string(), value.to_string());
    if let Some(input) = task
        .input_specs
        .iter_mut()
        .find(|input| input.kind == "value" && input.name == name)
    {
        input.value = value.to_string();
        return;
    }
    task.input_specs
        .push(super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
            name: name.to_string(),
            kind: "value".to_string(),
            value: value.to_string(),
            normalization: normalization.to_string(),
            optional: false,
        });
    task.diagnostics
        .push(super::build_plan_ir::CanonicalBuildPlanTaskDiagnostic {
            severity: "info".to_string(),
            code: source.to_string(),
            message: format!("Inferred {} from selected task graph dependencies", name),
            source: "rust-task-graph".to_string(),
        });
}

fn execution_context_json(task: &CanonicalBuildPlanTask, task_type: &str) -> String {
    let source_files = match task_type {
        "JavaCompile" => java_source_paths(task),
        "Javadoc" => java_source_paths(task),
        "TestExec" => test_class_dir_paths(task),
        "Delete" => destroyable_paths(task),
        "Copy" if is_process_resources_task(task) => process_resources_source_paths(task),
        _ => input_paths(task),
    };
    let output_paths = output_paths(task);
    let target_dir = match task_type {
        "Mkdir" | "Delete" => String::new(),
        "Jar" | "Zip" | "War" | "Ear" | "Tar" => output_paths
            .first()
            .and_then(|path| std::path::Path::new(path).parent())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => output_paths.first().cloned().unwrap_or_default(),
    };
    let mut options = task_options(task, task_type);
    if is_archive_executor(task_type) {
        if let Some(path) = output_paths.first() {
            let option_name = if task_type == "Tar" {
                "tarName"
            } else {
                "jarName"
            };
            if !options.contains_key(option_name) {
                if let Some(name) = std::path::Path::new(path).file_name() {
                    options.insert(
                        option_name.to_string(),
                        serde_json::Value::String(name.to_string_lossy().into_owned()),
                    );
                }
            }
        }
    }

    let execution_source_files = match task_type {
        "Mkdir" => output_paths.clone(),
        "Jar" | "Zip" | "War" | "Ear" | "Tar" => {
            archive_source_paths(task_type, &source_files, &output_paths)
        }
        _ => source_files,
    };
    let input_file_fingerprints = work_input_file_fingerprints(&execution_source_files);

    serde_json::json!({
        "source_files": execution_source_files,
        "target_dir": target_dir,
        "output_files": output_paths,
        "options": options,
        "work_identity": task.path,
        "display_name": task.path,
        "implementation_class": task.implementation_id,
        "input_properties": work_input_properties(task, task_type, &target_dir),
        "input_file_fingerprints": input_file_fingerprints,
        "caching_enabled": cacheability_allows_cache(&task.cacheability),
        "can_load_from_cache": cacheability_allows_cache(&task.cacheability),
        "up_to_date_enabled": up_to_date_planning_enabled(task_type),
        "no_source": no_source_task(task),
        "has_previous_execution_state": false,
        "rebuild_reasons": [],
    })
    .to_string()
}

fn cacheability_allows_cache(cacheability: &str) -> bool {
    let value = cacheability.trim().to_ascii_lowercase();
    value == "cacheable" || value == "enabled" || value == "true"
}

fn work_input_properties(
    task: &CanonicalBuildPlanTask,
    task_type: &str,
    target_dir: &str,
) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::new();
    properties.insert("task_type".to_string(), task_type.to_string());
    properties.insert(
        "implementation_class".to_string(),
        task.implementation_id.clone(),
    );
    properties.insert("action_kind".to_string(), task.action_kind.clone());
    properties.insert("cacheability".to_string(), task.cacheability.clone());
    properties.insert("target_dir".to_string(), target_dir.to_string());
    properties.insert("outputs".to_string(), output_paths(task).join("\n"));
    properties.insert("local_state".to_string(), task.local_state.join("\n"));
    properties.insert("destroyables".to_string(), task.destroyables.join("\n"));
    for (key, value) in &task.inputs {
        if !execution_relevant_input_property(task, task_type, key, value) {
            continue;
        }
        properties.insert(format!("input.{}", key), value.clone());
    }
    for input in &task.input_specs {
        if !execution_relevant_input_spec(task, task_type, input) {
            continue;
        }
        if matches!(
            input.kind.as_str(),
            "file" | "directory" | "path" | "source"
        ) {
            properties.insert(format!("input_path.{}", input.name), input.value.clone());
        } else {
            properties.insert(format!("input_value.{}", input.name), input.value.clone());
        }
        properties.insert(
            format!("input_normalization.{}", input.name),
            input.normalization.clone(),
        );
        properties.insert(
            format!("input_optional.{}", input.name),
            input.optional.to_string(),
        );
    }
    for output in &task.output_specs {
        properties.insert(format!("output.{}", output.name), output.path.clone());
        properties.insert(format!("output_kind.{}", output.name), output.kind.clone());
    }
    properties
}

fn execution_relevant_input_name(name: &str) -> bool {
    !matches!(name, "description" | "group")
}

fn execution_relevant_input_property(
    task: &CanonicalBuildPlanTask,
    task_type: &str,
    name: &str,
    value: &str,
) -> bool {
    if !execution_relevant_input_name(name) {
        return false;
    }
    if name == "copy_file_mappings"
        && redundant_convention_jar_copy_mappings(task, task_type, value)
    {
        return false;
    }
    true
}

fn execution_relevant_input_spec(
    task: &CanonicalBuildPlanTask,
    task_type: &str,
    input: &super::build_plan_ir::CanonicalBuildPlanTaskInputSpec,
) -> bool {
    if !execution_relevant_input_name(&input.name) {
        return false;
    }
    if input.name == "copy_file_mappings"
        && redundant_convention_jar_copy_mappings(task, task_type, &input.value)
    {
        return false;
    }
    if redundant_convention_jar_path_spec(task, task_type, input) {
        return false;
    }
    true
}

fn redundant_convention_jar_copy_mappings(
    task: &CanonicalBuildPlanTask,
    task_type: &str,
    value: &str,
) -> bool {
    if task_type != "Jar" || has_input_value_equal(task, "copy_contains_symlinks", "true") {
        return false;
    }
    let Some(roots) = convention_jar_roots(task) else {
        return false;
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    trimmed.split(',').all(|mapping| {
        let parts = mapping.split('>').collect::<Vec<_>>();
        if parts.len() != 3 {
            return false;
        }
        let Some(source) = decode_mapping_component(parts[0]) else {
            return false;
        };
        let Some(dest) = decode_mapping_component(parts[1]) else {
            return false;
        };
        matches!(parts[2], "D" | "F") && convention_jar_mapping_is_redundant(&roots, &source, &dest)
    })
}

fn redundant_convention_jar_path_spec(
    task: &CanonicalBuildPlanTask,
    task_type: &str,
    input: &super::build_plan_ir::CanonicalBuildPlanTaskInputSpec,
) -> bool {
    if task_type != "Jar" {
        return false;
    }
    if !matches!(
        input.kind.as_str(),
        "file" | "directory" | "path" | "source"
    ) {
        return false;
    }
    let Some(index) = input.name.strip_prefix("input") else {
        return false;
    };
    if index.is_empty() || !index.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let Some(roots) = convention_jar_roots(task) else {
        return false;
    };
    let path = Path::new(&input.value);
    path == roots.manifest || path.starts_with(&roots.classes) || path.starts_with(&roots.resources)
}

struct ConventionJarRoots {
    classes: std::path::PathBuf,
    resources: std::path::PathBuf,
    manifest: std::path::PathBuf,
}

fn convention_jar_roots(task: &CanonicalBuildPlanTask) -> Option<ConventionJarRoots> {
    let archive_file =
        input_value(task, "archive_file").or_else(|| output_paths(task).into_iter().next())?;
    let archive_path = Path::new(&archive_file);
    let build_dir = archive_path.parent()?.parent()?;
    Some(ConventionJarRoots {
        classes: build_dir.join("classes/java/main"),
        resources: build_dir.join("resources/main"),
        manifest: build_dir.join("tmp/jar/MANIFEST.MF"),
    })
}

fn decode_mapping_component(value: &str) -> Option<String> {
    URL_SAFE_NO_PAD
        .decode(value)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

fn convention_jar_mapping_is_redundant(
    roots: &ConventionJarRoots,
    source: &str,
    dest: &str,
) -> bool {
    let source_path = Path::new(source);
    if dest == "META-INF/MANIFEST.MF" && source_path == roots.manifest {
        return true;
    }
    relative_dest_matches(source_path, &roots.classes, dest)
        || relative_dest_matches(source_path, &roots.resources, dest)
}

fn relative_dest_matches(source_path: &Path, root: &Path, dest: &str) -> bool {
    let Ok(relative) = source_path.strip_prefix(root) else {
        return false;
    };
    let relative = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    !relative.is_empty() && relative == dest
}

fn up_to_date_planning_enabled(task_type: &str) -> bool {
    matches!(
        task_type,
        "Lifecycle"
            | "JavaCompile"
            | "Javadoc"
            | "Copy"
            | "Sync"
            | "Jar"
            | "Zip"
            | "War"
            | "Ear"
            | "Tar"
            | "TestExec"
            | "CreateStartScripts"
            | "WriteFile"
            | "Mkdir"
            | "Delete"
    )
}

fn no_source_task(task: &CanonicalBuildPlanTask) -> bool {
    let simple = task
        .implementation_id
        .rsplit('.')
        .next()
        .unwrap_or(task.implementation_id.as_str());
    match logical_gradle_task_type(simple).as_str() {
        "JavaCompile" => java_compile_no_source(task),
        "ProcessResources" => process_resources_no_source(task),
        "Test" => test_no_source(task),
        _ => false,
    }
}

pub(crate) fn work_input_file_fingerprints(paths: &[String]) -> BTreeMap<String, String> {
    let mut fingerprints = BTreeMap::new();
    for path in paths.iter().filter(|path| !path.trim().is_empty()) {
        fingerprints.insert(path.clone(), fingerprint_path(Path::new(path)));
    }
    fingerprints
}

fn archive_source_paths(
    task_type: &str,
    declared_sources: &[String],
    output_paths: &[String],
) -> Vec<String> {
    let mut sources = declared_sources.to_vec();
    let Some(archive_path) = output_paths.first().map(std::path::Path::new) else {
        return sources;
    };
    let Some(build_dir) = archive_path.parent().and_then(|path| path.parent()) else {
        return sources;
    };
    let Some(project_dir) = build_dir.parent() else {
        return sources;
    };
    let archive_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match task_type {
        "Jar" if archive_name.ends_with("-sources.jar") => {
            let java = project_dir.join("src/main/java");
            let resources = project_dir.join("src/main/resources");
            retain_sources_not_under(&mut sources, &[java.as_path(), resources.as_path()]);
            push_unique_path(&mut sources, java);
            push_unique_path(&mut sources, resources);
        }
        "Jar" if archive_name.ends_with(".jar") => {
            let classes = build_dir.join("classes/java/main");
            let resources = build_dir.join("resources/main");
            retain_sources_not_under(&mut sources, &[classes.as_path(), resources.as_path()]);
            push_unique_path(&mut sources, classes);
            push_unique_path(&mut sources, resources);
        }
        "War" => {
            let webapp = project_dir.join("src/main/webapp");
            let classes = build_dir.join("classes/java/main");
            let resources = build_dir.join("resources/main");
            retain_sources_not_under(
                &mut sources,
                &[webapp.as_path(), classes.as_path(), resources.as_path()],
            );
            push_unique_path(&mut sources, webapp);
            push_unique_path(&mut sources, classes);
            push_unique_path(&mut sources, resources);
        }
        "Ear" => {
            let application = project_dir.join("src/main/application");
            retain_sources_not_under(&mut sources, &[application.as_path()]);
            push_unique_path(&mut sources, application);
        }
        _ => {}
    }
    sources
}

fn retain_sources_not_under(sources: &mut Vec<String>, roots: &[&Path]) {
    sources.retain(|source| {
        let path = Path::new(source);
        !roots
            .iter()
            .any(|root| path != *root && path.starts_with(root))
    });
}

fn push_unique_path(paths: &mut Vec<String>, path: std::path::PathBuf) {
    let value = path.to_string_lossy().into_owned();
    if !value.is_empty() && !paths.iter().any(|existing| existing == &value) {
        paths.push(value);
    }
}

fn fingerprint_path(path: &Path) -> String {
    if !path.exists() {
        return "missing".to_string();
    }
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return "unreadable".to_string();
    };
    if metadata.file_type().is_symlink() {
        return std::fs::read_link(path)
            .map(|target| format!("symlink:{}", target.to_string_lossy()))
            .unwrap_or_else(|_| "symlink:unreadable".to_string());
    }
    if metadata.is_file() {
        return fingerprint_file(path);
    }
    if metadata.is_dir() {
        return fingerprint_directory(path);
    }
    format!("other:{}", metadata.len())
}

fn fingerprint_file(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"file\0");
    let Ok(mut file) = std::fs::File::open(path) else {
        return "file:unreadable".to_string();
    };
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(_) => return "file:unreadable".to_string(),
        }
    }
    crate::server::cache::hex::encode(hasher.finalize().as_ref())
}

fn fingerprint_directory(path: &Path) -> String {
    let mut entries = Vec::new();
    collect_directory_fingerprints(path, path, &mut entries);
    entries.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(b"dir\0");
    for entry in entries {
        hasher.update(entry.as_bytes());
        hasher.update(b"\0");
    }
    crate::server::cache::hex::encode(hasher.finalize().as_ref())
}

fn collect_directory_fingerprints(root: &Path, path: &Path, entries: &mut Vec<String>) {
    let Ok(read_dir) = std::fs::read_dir(path) else {
        entries.push(format!("{}=unreadable", relative_path(root, path)));
        return;
    };
    for entry in read_dir.flatten() {
        let entry_path = entry.path();
        let relative = relative_path(root, &entry_path);
        let Ok(metadata) = std::fs::symlink_metadata(&entry_path) else {
            entries.push(format!("{}=unreadable", relative));
            continue;
        };
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&entry_path)
                .map(|target| target.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "unreadable".to_string());
            entries.push(format!("{}=symlink:{}", relative, target));
        } else if metadata.is_file() {
            entries.push(format!(
                "{}=file:{}",
                relative,
                fingerprint_file(&entry_path)
            ));
        } else if metadata.is_dir() {
            entries.push(format!("{}=dir", relative));
            collect_directory_fingerprints(root, &entry_path, entries);
        } else {
            entries.push(format!("{}=other:{}", relative, metadata.len()));
        }
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn java_compile_contract_complete(task: &CanonicalBuildPlanTask) -> bool {
    !java_source_paths(task).is_empty() && has_outputs(task)
}

fn java_compile_no_source(task: &CanonicalBuildPlanTask) -> bool {
    java_source_paths(task).is_empty() && has_outputs(task)
}

fn test_exec_contract_complete(task: &CanonicalBuildPlanTask) -> bool {
    has_input_value(task, "classpath")
        && has_input_value(task, "test_classes_dirs")
        && !has_input_value_equal(task, "test_unsupported_filters", "true")
}

fn test_no_source(task: &CanonicalBuildPlanTask) -> bool {
    if input_value(task, "working_dir")
        .map(|working_dir| !has_test_sources_under(std::path::Path::new(&working_dir)))
        .unwrap_or(false)
    {
        return true;
    }
    !has_input_value(task, "classpath") && input_value_paths_missing(task, "test_classes_dirs")
}

fn copy_contract_complete(task: &CanonicalBuildPlanTask) -> bool {
    has_input_paths(task)
        && has_outputs(task)
        && !has_input_value_equal(task, "copy_unsupported_custom_actions", "true")
}

fn process_resources_contract_complete(task: &CanonicalBuildPlanTask) -> bool {
    !process_resources_source_paths(task).is_empty()
        && has_outputs(task)
        && !has_input_value_equal(task, "copy_unsupported_custom_actions", "true")
}

fn process_resources_no_source(task: &CanonicalBuildPlanTask) -> bool {
    process_resources_source_paths(task)
        .iter()
        .all(|path| !std::path::Path::new(path).exists())
}

fn exec_contract_complete(task: &CanonicalBuildPlanTask) -> bool {
    has_input_value(task, "executable")
}

fn java_exec_contract_complete(task: &CanonicalBuildPlanTask) -> bool {
    has_input_value(task, "main_class")
        && (has_input_value(task, "classpath") || inferred_java_exec_classpath(task).is_some())
}

fn javadoc_contract_complete(task: &CanonicalBuildPlanTask) -> bool {
    !java_source_paths(task).is_empty() && has_outputs(task)
}

fn static_write_file_contract_complete(task: &CanonicalBuildPlanTask) -> bool {
    has_input_value(task, "static_output_text_b64") && task.outputs.len() == 1
}

fn compat_task_type(task: &CanonicalBuildPlanTask, fallback: &str) -> String {
    if task.implementation_id.contains('.') {
        task.implementation_id.clone()
    } else {
        fallback.to_string()
    }
}

fn task_options(
    task: &CanonicalBuildPlanTask,
    task_type: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut options = serde_json::Map::new();
    if task_type == "JavaCompile" {
        insert_input_option(task, &mut options, "java_home", "java_home");
        insert_input_option(task, &mut options, "classpath", "classpath");
        insert_input_option(task, &mut options, "processor_path", "processor_path");
        insert_input_option(task, &mut options, "encoding", "encoding");
        insert_input_option(task, &mut options, "parameters", "parameters");
        insert_input_option(task, &mut options, "debug", "debug");
        insert_input_option(task, &mut options, "compiler_args", "compiler_args");
        insert_input_option(
            task,
            &mut options,
            "compiler_args_json",
            "compiler_args_json",
        );
        insert_input_option(task, &mut options, "release", "release");
        if !options.contains_key("release") {
            insert_input_option(task, &mut options, "source_version", "source_version");
            insert_input_option(task, &mut options, "sourceCompatibility", "source_version");
            insert_input_option(task, &mut options, "target_version", "target_version");
            insert_input_option(task, &mut options, "targetCompatibility", "target_version");
        }
    } else if is_zip_archive_executor(task_type) {
        insert_input_option(task, &mut options, "archive_file_name", "jarName");
        insert_input_option(task, &mut options, "archive_file", "archive_file");
        insert_input_option(task, &mut options, "main_class", "mainClass");
        insert_manifest_options(task, &mut options);
        insert_input_option(
            task,
            &mut options,
            "duplicates_strategy",
            "duplicates_strategy",
        );
        insert_input_option(
            task,
            &mut options,
            "include_empty_dirs",
            "include_empty_dirs",
        );
        insert_input_option(
            task,
            &mut options,
            "copy_file_mappings",
            "copy_file_mappings",
        );
        insert_input_option(
            task,
            &mut options,
            "graph_distribution_libs",
            "graph_distribution_libs",
        );
        insert_input_option(task, &mut options, "file_permissions", "file_permissions");
        insert_input_option(task, &mut options, "dir_permissions", "dir_permissions");
    } else if task_type == "Tar" {
        insert_input_option(task, &mut options, "archive_file_name", "tarName");
        insert_input_option(task, &mut options, "archive_file", "archive_file");
        insert_input_option(task, &mut options, "archive_compression", "compression");
        insert_input_option(
            task,
            &mut options,
            "duplicates_strategy",
            "duplicates_strategy",
        );
        insert_input_option(
            task,
            &mut options,
            "include_empty_dirs",
            "include_empty_dirs",
        );
        insert_input_option(
            task,
            &mut options,
            "copy_file_mappings",
            "copy_file_mappings",
        );
        insert_input_option(
            task,
            &mut options,
            "graph_distribution_libs",
            "graph_distribution_libs",
        );
        insert_input_option(task, &mut options, "file_permissions", "file_permissions");
        insert_input_option(task, &mut options, "dir_permissions", "dir_permissions");
    } else if matches!(task_type, "Copy" | "Sync") {
        insert_input_option(task, &mut options, "expand_properties", "expand_properties");
        insert_input_option(
            task,
            &mut options,
            "duplicates_strategy",
            "duplicates_strategy",
        );
        insert_input_option(task, &mut options, "filtering_charset", "filtering_charset");
        insert_input_option(task, &mut options, "include_patterns", "include_patterns");
        insert_input_option(task, &mut options, "exclude_patterns", "exclude_patterns");
        insert_input_option(task, &mut options, "case_sensitive", "case_sensitive");
        insert_input_option(
            task,
            &mut options,
            "copy_file_mappings",
            "copy_file_mappings",
        );
        insert_input_option(
            task,
            &mut options,
            "copy_line_replace_filter",
            "copy_line_replace_filter",
        );
        insert_input_option(
            task,
            &mut options,
            "include_empty_dirs",
            "include_empty_dirs",
        );
        insert_input_option(task, &mut options, "file_permissions", "file_permissions");
        insert_input_option(task, &mut options, "dir_permissions", "dir_permissions");
    } else if task_type == "TestExec" {
        insert_input_option(task, &mut options, "java_home", "java_home");
        insert_input_option(task, &mut options, "classpath", "classpath");
        insert_input_option(task, &mut options, "working_dir", "working_dir");
        insert_input_option(task, &mut options, "xml_report_dir", "xml_report_dir");
        insert_input_option(task, &mut options, "jvm_args", "jvm_args");
        insert_input_option(task, &mut options, "jvm_args_json", "jvm_args_json");
        insert_input_option(task, &mut options, "system_properties", "system_properties");
        insert_input_option(task, &mut options, "scan_classpath", "scan_classpath");
        insert_input_option(task, &mut options, "test_filter", "test_filter");
        insert_input_option(
            task,
            &mut options,
            "test_filter_includes",
            "test_filter_includes",
        );
        insert_input_option(
            task,
            &mut options,
            "test_filter_excludes",
            "test_filter_excludes",
        );
        insert_input_option(task, &mut options, "include_tags", "include_tags");
        insert_input_option(task, &mut options, "exclude_tags", "exclude_tags");
        insert_max_heap_option(task, &mut options);
    } else if task_type == "Exec" {
        insert_input_option(task, &mut options, "executable", "executable");
        insert_input_option(task, &mut options, "args", "args");
        insert_input_option(task, &mut options, "args_json", "args_json");
        insert_input_option(task, &mut options, "environment", "environment");
        insert_input_option(task, &mut options, "environment_json", "environment_json");
        insert_input_option(task, &mut options, "working_dir", "working_dir");
        insert_input_option(task, &mut options, "ignore_exit_value", "ignore_exit_value");
    } else if task_type == "JavaExec" {
        insert_input_option(task, &mut options, "java_home", "java_home");
        insert_input_option(task, &mut options, "classpath", "classpath");
        if !options.contains_key("classpath") {
            if let Some(classpath) = inferred_java_exec_classpath(task) {
                options.insert(
                    "classpath".to_string(),
                    serde_json::Value::String(classpath),
                );
            }
        }
        insert_input_option(task, &mut options, "main_class", "main_class");
        insert_input_option(task, &mut options, "args", "args");
        insert_input_option(task, &mut options, "args_json", "args_json");
        insert_input_option(task, &mut options, "jvm_args", "jvm_args");
        insert_input_option(task, &mut options, "jvm_args_json", "jvm_args_json");
        insert_input_option(task, &mut options, "max_heap_size", "max_heap_size");
        insert_input_option(task, &mut options, "system_properties", "system_properties");
        insert_input_option(task, &mut options, "environment", "environment");
        insert_input_option(task, &mut options, "environment_json", "environment_json");
        insert_input_option(task, &mut options, "working_dir", "working_dir");
        insert_input_option(task, &mut options, "ignore_exit_value", "ignore_exit_value");
    } else if task_type == "CreateStartScripts" {
        insert_input_option(task, &mut options, "application_name", "application_name");
        insert_input_option(task, &mut options, "main_class", "main_class");
        insert_input_option(task, &mut options, "main_module", "main_module");
        insert_input_option(task, &mut options, "classpath", "classpath");
        insert_input_option(task, &mut options, "output_dir", "output_dir");
        insert_input_option(task, &mut options, "executable_dir", "executable_dir");
        insert_input_option(task, &mut options, "default_jvm_opts", "default_jvm_opts");
        insert_input_option(
            task,
            &mut options,
            "opts_environment_var",
            "opts_environment_var",
        );
        insert_input_option(task, &mut options, "git_ref", "git_ref");
        insert_input_option(task, &mut options, "unix_script", "unix_script");
        insert_input_option(task, &mut options, "windows_script", "windows_script");
    } else if task_type == "Javadoc" {
        insert_input_option(task, &mut options, "java_home", "java_home");
        insert_input_option(task, &mut options, "classpath", "classpath");
        insert_input_option(task, &mut options, "destination_dir", "destination_dir");
        insert_input_option(task, &mut options, "title", "title");
        insert_input_option(task, &mut options, "max_memory", "max_memory");
        insert_input_option(task, &mut options, "encoding", "encoding");
        insert_input_option(task, &mut options, "no_timestamp", "no_timestamp");
    } else if task_type == "WriteFile" {
        insert_input_option(
            task,
            &mut options,
            "static_output_text_b64",
            "static_output_text_b64",
        );
    }
    options
}

fn insert_manifest_options(
    task: &CanonicalBuildPlanTask,
    options: &mut serde_json::Map<String, serde_json::Value>,
) {
    for input in &task.input_specs {
        if input.kind == "value"
            && input.name.starts_with("manifest.")
            && !input.value.trim().is_empty()
        {
            options.insert(
                input.name.clone(),
                serde_json::Value::String(input.value.trim().to_string()),
            );
        }
    }
}

fn is_zip_archive_executor(task_type: &str) -> bool {
    matches!(task_type, "Jar" | "Zip" | "War" | "Ear")
}

fn is_archive_executor(task_type: &str) -> bool {
    is_zip_archive_executor(task_type) || task_type == "Tar"
}

fn has_input_value(task: &CanonicalBuildPlanTask, input_name: &str) -> bool {
    task.input_specs.iter().any(|input| {
        input.kind == "value" && input.name == input_name && !input.value.trim().is_empty()
    })
}

fn has_input_value_equal(task: &CanonicalBuildPlanTask, input_name: &str, expected: &str) -> bool {
    task.input_specs.iter().any(|input| {
        input.kind == "value"
            && input.name == input_name
            && input.value.trim().eq_ignore_ascii_case(expected)
    })
}

fn input_value_paths_missing(task: &CanonicalBuildPlanTask, input_name: &str) -> bool {
    task.input_specs
        .iter()
        .find(|input| input.kind == "value" && input.name == input_name)
        .map(|input| {
            std::env::split_paths(&input.value)
                .all(|path| path.as_os_str().is_empty() || !path.exists())
        })
        .unwrap_or(true)
}

fn insert_max_heap_option(
    task: &CanonicalBuildPlanTask,
    options: &mut serde_json::Map<String, serde_json::Value>,
) {
    if let Some(value) = task
        .input_specs
        .iter()
        .find(|input| input.kind == "value" && input.name == "max_heap_size")
        .map(|input| input.value.trim())
        .filter(|value| !value.is_empty())
    {
        options.insert(
            "max_heap_mb".to_string(),
            serde_json::Value::String(normalize_heap_megabytes(value)),
        );
    }
}

fn normalize_heap_megabytes(value: &str) -> String {
    let trimmed = value.trim().to_ascii_lowercase();
    if let Some(mb) = trimmed.strip_suffix('m') {
        mb.to_string()
    } else if let Some(gb) = trimmed.strip_suffix('g') {
        gb.parse::<u64>()
            .map(|n| (n * 1024).to_string())
            .unwrap_or_else(|_| value.to_string())
    } else {
        value.to_string()
    }
}

fn insert_input_option(
    task: &CanonicalBuildPlanTask,
    options: &mut serde_json::Map<String, serde_json::Value>,
    input_name: &str,
    option_name: &str,
) {
    if let Some(value) = task
        .input_specs
        .iter()
        .find(|input| input.kind == "value" && input.name == input_name)
        .map(|input| input.value.trim())
        .filter(|value| !value.is_empty())
    {
        options.insert(
            option_name.to_string(),
            serde_json::Value::String(normalize_java_option(value)),
        );
    }
}

fn input_value(task: &CanonicalBuildPlanTask, input_name: &str) -> Option<String> {
    task.input_specs
        .iter()
        .find(|input| input.kind == "value" && input.name == input_name)
        .map(|input| input.value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn inferred_java_exec_classpath(task: &CanonicalBuildPlanTask) -> Option<String> {
    let project_dir = input_value(task, "working_dir")?;
    let mut entries = Vec::new();
    for relative in ["build/classes/java/main", "build/resources/main"] {
        let path = std::path::Path::new(&project_dir).join(relative);
        if path.exists() {
            entries.push(path);
        }
    }
    if entries.is_empty() {
        return None;
    }
    std::env::join_paths(entries)
        .ok()
        .map(|value| value.to_string_lossy().into_owned())
}

fn normalize_java_option(value: &str) -> String {
    let normalized = value
        .trim()
        .trim_start_matches("JavaVersion.")
        .trim_start_matches("VERSION_")
        .to_string();
    if normalized.starts_with("1_") {
        normalized.replace('_', ".")
    } else {
        normalized
    }
}

fn has_input_paths(task: &CanonicalBuildPlanTask) -> bool {
    !input_paths(task).is_empty()
}

fn has_outputs(task: &CanonicalBuildPlanTask) -> bool {
    !output_paths(task).is_empty()
}

fn has_destroyables(task: &CanonicalBuildPlanTask) -> bool {
    !destroyable_paths(task).is_empty()
}

fn no_task_actions(task: &CanonicalBuildPlanTask) -> bool {
    task.inputs
        .get("action_count")
        .map(|value| value == "0")
        .unwrap_or(false)
        || task.input_specs.iter().any(|input| {
            input.kind == "value" && input.name == "action_count" && input.value == "0"
        })
}

fn destroyable_paths(task: &CanonicalBuildPlanTask) -> Vec<String> {
    task.destroyables
        .iter()
        .filter(|path| !path.is_empty())
        .cloned()
        .collect()
}

fn input_paths(task: &CanonicalBuildPlanTask) -> Vec<String> {
    task.input_specs
        .iter()
        .filter(|input| {
            matches!(
                input.kind.as_str(),
                "file" | "directory" | "path" | "source"
            )
        })
        .map(|input| input.value.clone())
        .filter(|path| !path.is_empty())
        .collect()
}

fn java_source_paths(task: &CanonicalBuildPlanTask) -> Vec<String> {
    input_paths(task)
        .into_iter()
        .filter(|path| path.ends_with(".java"))
        .collect()
}

fn is_process_resources_task(task: &CanonicalBuildPlanTask) -> bool {
    logical_gradle_task_type(
        task.implementation_id
            .rsplit('.')
            .next()
            .unwrap_or(task.implementation_id.as_str()),
    ) == "ProcessResources"
}

fn process_resources_source_paths(task: &CanonicalBuildPlanTask) -> Vec<String> {
    let declared = input_paths(task);
    if !declared.is_empty() {
        return declared;
    }
    infer_process_resources_source_dir(task)
        .into_iter()
        .collect()
}

fn infer_process_resources_source_dir(task: &CanonicalBuildPlanTask) -> Option<String> {
    let output = output_paths(task).into_iter().next()?;
    let marker = format!(
        "{}build{}resources{}",
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    );
    let (project_root, rest) = output.split_once(&marker)?;
    let source_set = rest
        .split(std::path::MAIN_SEPARATOR)
        .next()
        .filter(|value| !value.is_empty())?;
    Some(format!(
        "{}{}src{}{}{}resources",
        project_root,
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR,
        source_set,
        std::path::MAIN_SEPARATOR
    ))
}

fn test_class_dir_paths(task: &CanonicalBuildPlanTask) -> Vec<String> {
    task.input_specs
        .iter()
        .find(|input| input.kind == "value" && input.name == "test_classes_dirs")
        .map(|input| {
            std::env::split_paths(&input.value)
                .map(|path| path.to_string_lossy().into_owned())
                .filter(|path| !path.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn output_paths(task: &CanonicalBuildPlanTask) -> Vec<String> {
    if task.output_specs.is_empty() {
        task.outputs.clone()
    } else {
        task.output_specs
            .iter()
            .map(|output| output.path.clone())
            .filter(|path| !path.is_empty())
            .collect()
    }
}

#[tonic::async_trait]
impl TaskGraphService for TaskGraphServiceImpl {
    async fn register_task(
        &self,
        request: Request<RegisterTaskRequest>,
    ) -> Result<Response<RegisterTaskResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id.clone());

        // Look up estimated duration from execution history
        let estimated = self.lookup_historical_duration(&req.task_path);

        tracing::debug!(
            build_id = %req.build_id,
            task_path = %req.task_path,
            task_type = %req.task_type,
            should_execute = req.should_execute,
            dependency_count = req.depends_on.len(),
            estimated_duration_ms = estimated,
            "Registered task in graph"
        );

        // Validate: if a task should not execute, its dependencies are irrelevant
        // for scheduling but we still store them for graph consistency.
        if !req.should_execute && !req.depends_on.is_empty() {
            tracing::debug!(
                task_path = %req.task_path,
                dependency_count = req.depends_on.len(),
                "Non-executing task has dependencies; they will still be scheduled independently"
            );
        }

        self.tasks.insert(
            (build_id.clone(), req.task_path.clone()),
            TaskNode {
                task_path: req.task_path.clone(),
                depends_on: req.depends_on,
                should_execute: req.should_execute,
                task_type: req.task_type,
                execution_context_json: String::new(),
                estimated_duration_ms: estimated,
                status: "PENDING".to_string(),
                start_time_ms: 0,
                duration_ms: 0,
            },
        );

        // Populate the reverse index: file -> tasks that depend on it
        if !req.input_files.is_empty() {
            for file_path in &req.input_files {
                self.file_to_tasks
                    .entry(file_path.clone())
                    .or_default()
                    .push((build_id.clone(), req.task_path.clone()));
            }
        }

        Ok(Response::new(RegisterTaskResponse { success: true }))
    }

    async fn clear_build_tasks(
        &self,
        request: Request<ClearBuildTasksRequest>,
    ) -> Result<Response<ClearBuildTasksResponse>, Status> {
        let req = request.into_inner();
        if req.build_id.is_empty() {
            return Err(Status::invalid_argument("build_id must not be empty"));
        }

        let build_id = BuildId::from(req.build_id.clone());
        let cleared_tasks = self.build_task_count(&build_id) as i32;
        self.cleanup_build(&build_id);
        tracing::debug!(
            build_id = %req.build_id,
            cleared_tasks = cleared_tasks,
            "Cleared task graph state for build"
        );
        Ok(Response::new(ClearBuildTasksResponse { cleared_tasks }))
    }

    async fn resolve_execution_plan(
        &self,
        request: Request<ResolveExecutionPlanRequest>,
    ) -> Result<Response<ResolveExecutionPlanResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id.clone());
        self.request_counter.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(build_id = %req.build_id, "Resolving execution plan");

        let had_registered_tasks = self.has_registered_tasks(&build_id);
        let hydrated_task_count = if req.prefer_build_plan_shadow {
            self.hydrate_from_shadow_plan(&build_id, &req.build_id, true)
        } else if had_registered_tasks {
            0
        } else {
            self.hydrate_from_shadow_plan(&build_id, &req.build_id, false)
        };
        let has_registered_after_hydration = self.has_registered_tasks(&build_id);
        let plan_source = if hydrated_task_count > 0 {
            "build-plan-shadow"
        } else if has_registered_after_hydration {
            "registered-tasks"
        } else {
            "empty"
        };

        let (execution_order, critical_path_ms, has_cycles) = self.resolve_plan(&build_id);

        let total = self.tasks.iter().filter(|e| e.key().0 == build_id).count() as i32;
        let skipped = self
            .tasks
            .iter()
            .filter(|e| e.key().0 == build_id && !e.should_execute)
            .count() as i32;
        let ready = execution_order
            .iter()
            .filter(|n| n.dependencies.is_empty())
            .count() as i32;

        if skipped > 0 {
            tracing::info!(
                build_id = %req.build_id,
                total_tasks = total,
                skipped = skipped,
                executable = total - skipped,
                ready_to_execute = ready,
                critical_path_ms = critical_path_ms,
                has_cycles = has_cycles,
                plan_source = plan_source,
                "Execution plan resolved with excluded tasks"
            );
        }

        Ok(Response::new(ResolveExecutionPlanResponse {
            execution_order,
            total_tasks: total,
            ready_to_execute: ready,
            critical_path_ms,
            has_cycles,
            plan_source: plan_source.to_string(),
        }))
    }

    async fn task_started(
        &self,
        request: Request<TaskStartedRequest>,
    ) -> Result<Response<TaskStartedResponse>, Status> {
        let req = request.into_inner();

        if !req.build_id.is_empty() {
            // Direct lookup via composite key
            let build_id = BuildId::from(req.build_id.clone());
            if let Some(mut entry) = self.tasks.get_mut(&(build_id, req.task_path.clone())) {
                entry.status = "EXECUTING".to_string();
                entry.start_time_ms = req.start_time_ms;
                tracing::debug!(
                    build_id = %req.build_id,
                    task_path = %req.task_path,
                    task_type = %entry.task_type,
                    start_time_ms = req.start_time_ms,
                    "Task started executing"
                );
            }
        } else {
            // Legacy: scan all builds when build_id not provided
            for mut entry in self.tasks.iter_mut() {
                if entry.task_path == req.task_path {
                    entry.status = "EXECUTING".to_string();
                    entry.start_time_ms = req.start_time_ms;
                    tracing::debug!(
                        task_path = %req.task_path,
                        task_type = %entry.task_type,
                        start_time_ms = req.start_time_ms,
                        "Task started executing (legacy scan)"
                    );
                    break;
                }
            }
        }

        Ok(Response::new(TaskStartedResponse { acknowledged: true }))
    }

    async fn task_finished(
        &self,
        request: Request<TaskFinishedRequest>,
    ) -> Result<Response<TaskFinishedResponse>, Status> {
        let req = request.into_inner();

        if !req.build_id.is_empty() {
            // Direct lookup via composite key
            let build_id = BuildId::from(req.build_id.clone());
            if let Some(mut entry) = self.tasks.get_mut(&(build_id, req.task_path.clone())) {
                entry.status = if req.success {
                    req.outcome.clone()
                } else {
                    "FAILED".to_string()
                };
                entry.duration_ms = req.duration_ms;
                entry.estimated_duration_ms = req.duration_ms;

                tracing::debug!(
                    build_id = %req.build_id,
                    task_path = %req.task_path,
                    task_type = %entry.task_type,
                    outcome = %entry.status,
                    duration_ms = req.duration_ms,
                    should_execute = entry.should_execute,
                    "Task finished"
                );

                if !entry.should_execute && entry.status == "FAILED" {
                    tracing::warn!(
                        build_id = %req.build_id,
                        task_path = %req.task_path,
                        task_type = %entry.task_type,
                        "Non-executing task reported failure; this may indicate a configuration issue"
                    );
                }
            }
        } else {
            // Legacy: scan all builds when build_id not provided
            for mut entry in self.tasks.iter_mut() {
                if entry.task_path == req.task_path {
                    entry.status = if req.success {
                        req.outcome.clone()
                    } else {
                        "FAILED".to_string()
                    };
                    entry.duration_ms = req.duration_ms;
                    entry.estimated_duration_ms = req.duration_ms;

                    tracing::debug!(
                        task_path = %req.task_path,
                        task_type = %entry.task_type,
                        outcome = %entry.status,
                        duration_ms = req.duration_ms,
                        should_execute = entry.should_execute,
                        "Task finished (legacy scan)"
                    );

                    if !entry.should_execute && entry.status == "FAILED" {
                        tracing::warn!(
                            task_path = %req.task_path,
                            task_type = %entry.task_type,
                            "Non-executing task reported failure; this may indicate a configuration issue"
                        );
                    }
                    break;
                }
            }
        }

        // Persist duration to execution history for future builds
        if let Some(history) = &self.history {
            history.store_task_duration(&req.task_path, req.duration_ms);
        }

        Ok(Response::new(TaskFinishedResponse { acknowledged: true }))
    }

    async fn get_progress(
        &self,
        request: Request<GetProgressRequest>,
    ) -> Result<Response<GetProgressResponse>, Status> {
        let req = request.into_inner();
        let filter_build_id = if req.build_id.is_empty() {
            None
        } else {
            Some(BuildId::from(req.build_id))
        };

        let mut tasks = Vec::with_capacity(self.tasks.len());
        let mut completed = 0i32;
        let mut executing = 0i32;
        let mut skipped = 0i32;

        for entry in self.tasks.iter() {
            // Filter by build_id if specified
            if let Some(ref bid) = filter_build_id {
                if entry.key().0 != *bid {
                    continue;
                }
            }

            // Tasks marked as not-should-execute are already resolved
            if !entry.should_execute {
                skipped += 1;
                tasks.push(TaskProgress {
                    task_path: entry.task_path.clone(),
                    status: "SKIPPED".to_string(),
                    duration_ms: 0,
                });
                continue;
            }

            let is_completed = matches!(
                entry.status.as_str(),
                "SUCCEEDED"
                    | "FAILED"
                    | "SKIPPED"
                    | "UP_TO_DATE"
                    | "FROM_CACHE"
                    | "EXECUTED"
                    | "EXECUTED_INCREMENTALLY"
                    | "EXECUTED_NON_INCREMENTALLY"
            );
            if is_completed {
                completed += 1;
            }
            if entry.status == "EXECUTING" {
                executing += 1;
            }
            tasks.push(TaskProgress {
                task_path: entry.task_path.clone(),
                status: entry.status.clone(),
                duration_ms: entry.duration_ms,
            });
        }

        let total = tasks.len() as i32;

        if skipped > 0 {
            tracing::debug!(
                total = total,
                completed = completed,
                executing = executing,
                skipped = skipped,
                "Progress includes skipped (non-executing) tasks"
            );
        }

        Ok(Response::new(GetProgressResponse {
            tasks,
            completed,
            total,
            executing,
            elapsed_ms: 0, // Could track from init
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_svc() -> TaskGraphServiceImpl {
        TaskGraphServiceImpl::new()
    }

    fn canonical_task(
        path: &str,
        implementation_id: &str,
        depends_on: Vec<String>,
        outputs: Vec<String>,
    ) -> CanonicalBuildPlanTask {
        CanonicalBuildPlanTask {
            path: path.to_string(),
            project_path: ":".to_string(),
            implementation_id: implementation_id.to_string(),
            depends_on,
            inputs: Default::default(),
            outputs,
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "unknown".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: String::new(),
            input_specs: Vec::new(),
            output_specs: Vec::new(),
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn test_execution_context_includes_work_metadata_for_rust_up_to_date_planning() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src/main/java/App.java");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "public class App {}\n").unwrap();
        let output = temp.path().join("build/classes/java/main");

        let mut task = canonical_task(
            ":compileJava",
            "org.gradle.api.tasks.compile.JavaCompile",
            Vec::new(),
            vec![output.to_string_lossy().into_owned()],
        );
        task.cacheability = "cacheable".to_string();
        task.input_specs.push(
            super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                name: "source0".to_string(),
                kind: "source".to_string(),
                value: source.to_string_lossy().into_owned(),
                normalization: "absolute-path".to_string(),
                optional: false,
            },
        );
        set_value_input(&mut task, "release", "17", "test", "scalar");

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(context["work_identity"], ":compileJava");
        assert_eq!(context["display_name"], ":compileJava");
        assert_eq!(
            context["implementation_class"],
            "org.gradle.api.tasks.compile.JavaCompile"
        );
        assert_eq!(context["input_properties"]["input_value.release"], "17");
        assert_eq!(context["caching_enabled"], true);
        assert_eq!(context["can_load_from_cache"], true);
        assert_eq!(context["up_to_date_enabled"], true);
        assert!(
            context["input_file_fingerprints"][source.to_string_lossy().as_ref()]
                .as_str()
                .unwrap()
                .len()
                >= 32
        );
    }

    #[test]
    fn test_work_input_file_fingerprint_changes_when_source_changes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("App.java");
        std::fs::write(&source, "class App { int value() { return 1; } }\n").unwrap();
        let paths = vec![source.to_string_lossy().into_owned()];

        let first = work_input_file_fingerprints(&paths);
        std::fs::write(&source, "class App { int value() { return 2; } }\n").unwrap();
        let second = work_input_file_fingerprints(&paths);

        assert_ne!(first, second);
    }

    #[test]
    fn test_work_input_properties_ignore_cosmetic_gradle_metadata() {
        let mut first = canonical_task(
            ":classes",
            "org.gradle.api.DefaultTask",
            Vec::new(),
            Vec::new(),
        );
        first.inputs.insert(
            "description".to_string(),
            "Assembles main classes.".to_string(),
        );
        first
            .inputs
            .insert("group".to_string(), "build".to_string());
        first
            .inputs
            .insert("enabled".to_string(), "true".to_string());

        let mut second = canonical_task(
            ":classes",
            "org.gradle.api.DefaultTask",
            Vec::new(),
            Vec::new(),
        );
        second
            .inputs
            .insert("enabled".to_string(), "true".to_string());

        assert_eq!(
            work_input_properties(&first, "Lifecycle", ""),
            work_input_properties(&second, "Lifecycle", "")
        );
        assert!(!up_to_date_planning_enabled("Exec"));
        assert!(!up_to_date_planning_enabled("JavaExec"));
        assert!(up_to_date_planning_enabled("JavaCompile"));
    }

    #[test]
    fn test_work_input_properties_ignore_redundant_convention_jar_mappings() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

        fn mapping(source: &str, dest: &str, kind: &str) -> String {
            format!(
                "{}>{}>{}",
                URL_SAFE_NO_PAD.encode(source),
                URL_SAFE_NO_PAD.encode(dest),
                kind
            )
        }

        let mut task = canonical_task(
            ":jar",
            "org.gradle.api.tasks.bundling.Jar",
            Vec::new(),
            vec!["/repo/build/libs/app.jar".to_string()],
        );
        set_value_input(
            &mut task,
            "archive_file",
            "/repo/build/libs/app.jar",
            "test",
            "scalar",
        );
        set_value_input(
            &mut task,
            "copy_contains_symlinks",
            "false",
            "test",
            "scalar",
        );
        set_value_input(
            &mut task,
            "copy_file_mappings",
            &[
                mapping(
                    "/repo/build/tmp/jar/MANIFEST.MF",
                    "META-INF/MANIFEST.MF",
                    "F",
                ),
                mapping("/repo/build/classes/java/main/org", "org", "D"),
                mapping(
                    "/repo/build/classes/java/main/org/gradle/App.class",
                    "org/gradle/App.class",
                    "F",
                ),
            ]
            .join(","),
            "test",
            "scalar",
        );
        task.input_specs.push(
            super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                name: "input0".to_string(),
                kind: "path".to_string(),
                value: "/repo/build/classes/java/main/org/gradle/App.class".to_string(),
                normalization: "absolute-path".to_string(),
                optional: false,
            },
        );

        let properties = work_input_properties(&task, "Jar", "/repo/build/libs");

        assert!(!properties.contains_key("input.copy_file_mappings"));
        assert!(!properties.contains_key("input_value.copy_file_mappings"));
        assert!(!properties.contains_key("input_path.input0"));
    }

    #[test]
    fn test_work_input_properties_keep_custom_jar_mappings() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

        let custom_mapping = format!(
            "{}>{}>F",
            URL_SAFE_NO_PAD.encode("/repo/build/classes/java/main/org/gradle/App.class"),
            URL_SAFE_NO_PAD.encode("custom/org/gradle/App.class")
        );
        let mut task = canonical_task(
            ":jar",
            "org.gradle.api.tasks.bundling.Jar",
            Vec::new(),
            vec!["/repo/build/libs/app.jar".to_string()],
        );
        set_value_input(
            &mut task,
            "archive_file",
            "/repo/build/libs/app.jar",
            "test",
            "scalar",
        );
        set_value_input(
            &mut task,
            "copy_contains_symlinks",
            "false",
            "test",
            "scalar",
        );
        set_value_input(
            &mut task,
            "copy_file_mappings",
            &custom_mapping,
            "test",
            "scalar",
        );

        let properties = work_input_properties(&task, "Jar", "/repo/build/libs");

        assert_eq!(
            properties
                .get("input.copy_file_mappings")
                .map(String::as_str),
            Some(custom_mapping.as_str())
        );
    }

    #[test]
    fn test_archive_context_includes_convention_source_roots_for_warm_fingerprints() {
        let mut task = canonical_task(
            ":jar",
            "org.gradle.api.tasks.bundling.Jar",
            vec![":classes".to_string()],
            vec!["/repo/build/libs/app.jar".to_string()],
        );
        set_value_input(
            &mut task,
            "archive_file",
            "/repo/build/libs/app.jar",
            "test",
            "scalar",
        );

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();
        let source_files = context["source_files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>();

        assert!(source_files.contains(&"/repo/build/classes/java/main"));
        assert!(source_files.contains(&"/repo/build/resources/main"));
    }

    #[test]
    fn test_archive_context_deduplicates_sources_under_convention_roots() {
        let mut task = canonical_task(
            ":jar",
            "org.gradle.api.tasks.bundling.Jar",
            Vec::new(),
            vec!["/repo/build/libs/app.jar".to_string()],
        );
        task.input_specs.push(
            super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                name: "input0".to_string(),
                kind: "path".to_string(),
                value: "/repo/build/classes/java/main/org/gradle/App.class".to_string(),
                normalization: "absolute-path".to_string(),
                optional: false,
            },
        );
        task.input_specs.push(
            super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                name: "input1".to_string(),
                kind: "path".to_string(),
                value: "/repo/build/tmp/jar/MANIFEST.MF".to_string(),
                normalization: "absolute-path".to_string(),
                optional: false,
            },
        );

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();
        let source_files = context["source_files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>();

        assert!(!source_files.contains(&"/repo/build/classes/java/main/org/gradle/App.class"));
        assert!(source_files.contains(&"/repo/build/classes/java/main"));
        assert!(source_files.contains(&"/repo/build/tmp/jar/MANIFEST.MF"));
    }

    #[test]
    fn test_process_resources_context_marks_no_source() {
        let task = canonical_task(
            ":processResources",
            "org.gradle.language.jvm.tasks.ProcessResources",
            Vec::new(),
            vec!["/repo/build/resources/main".to_string()],
        );

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "Lifecycle");
        assert_eq!(context["no_source"], true);
    }

    #[test]
    fn test_graph_enrichment_adds_project_compile_outputs_to_missing_java_classpath() {
        let mut task = canonical_task(
            ":app:compileJava",
            "org.gradle.api.tasks.compile.JavaCompile",
            vec![":lib:compileJava".to_string()],
            vec!["/repo/app/build/classes/java/main".to_string()],
        );
        task.input_specs.push(
            super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                name: "source0".to_string(),
                kind: "source".to_string(),
                value: "/repo/app/src/main/java/App.java".to_string(),
                normalization: "absolute-path".to_string(),
                optional: false,
            },
        );
        let task_outputs = HashMap::from([(
            ":lib:compileJava".to_string(),
            vec![
                "/repo/lib/build/classes/java/main".to_string(),
                "/repo/lib/build/generated/sources/headers/java/main".to_string(),
            ],
        )]);

        enrich_task_contract_from_graph(&mut task, &task_outputs, &[]);

        assert_eq!(
            input_value(&task, "classpath").as_deref(),
            Some("/repo/lib/build/classes/java/main")
        );
        assert_eq!(executable_task_type(&task), "JavaCompile");
    }

    #[test]
    fn test_dependency_configuration_matching_respects_test_scope() {
        assert!(dependency_configuration_matches_task(
            "compileClasspath",
            false
        ));
        assert!(dependency_configuration_matches_task(
            "runtimeClasspath",
            false
        ));
        assert!(!dependency_configuration_matches_task(
            "testRuntimeClasspath",
            false
        ));
        assert!(dependency_configuration_matches_task(
            "testRuntimeClasspath",
            true
        ));
        assert!(parse_coordinate("com.google.guava:guava:33.2.1-jre").is_some());
    }

    #[test]
    fn test_graph_enrichment_adds_project_jars_to_distribution_archives() {
        let mut task = canonical_task(
            ":app:distZip",
            "org.gradle.api.tasks.bundling.Zip",
            vec![":app:jar".to_string(), ":lib:jar".to_string()],
            Vec::new(),
        );
        set_value_input(
            &mut task,
            "archive_file",
            "/repo/app/build/distributions/app-1.0.zip",
            "test",
            "scalar",
        );
        let task_outputs = HashMap::from([
            (
                ":app:jar".to_string(),
                vec!["/repo/app/build/libs/app-1.0.jar".to_string()],
            ),
            (
                ":lib:jar".to_string(),
                vec!["/repo/lib/build/libs/lib-1.0.jar".to_string()],
            ),
        ]);

        enrich_task_contract_from_graph(&mut task, &task_outputs, &[]);

        let libs = input_value(&task, "graph_distribution_libs").unwrap();
        let split = std::env::split_paths(&libs)
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            split,
            vec![
                "/repo/app/build/libs/app-1.0.jar",
                "/repo/lib/build/libs/lib-1.0.jar"
            ]
        );
    }

    #[test]
    fn test_static_write_file_contract_lowers_default_task_to_native_executor() {
        let mut task = canonical_task(
            ":apiContractReport",
            "org.gradle.api.DefaultTask",
            Vec::new(),
            vec!["/repo/build/reports/api-contract.txt".to_string()],
        );
        task.action_kind = "jvm-task".to_string();
        set_value_input(
            &mut task,
            "static_output_text_b64",
            "b3NzLXN0eWxlIGFwaSBjb250cmFjdAo=",
            "test",
            "scalar",
        );

        let task_type = executable_task_type(&task);
        let context =
            serde_json::from_str::<serde_json::Value>(&execution_context_json(&task, &task_type))
                .unwrap();

        assert_eq!(task_type, "WriteFile");
        assert_eq!(
            context["target_dir"],
            "/repo/build/reports/api-contract.txt"
        );
        assert_eq!(
            context["options"]["static_output_text_b64"],
            "b3NzLXN0eWxlIGFwaSBjb250cmFjdAo="
        );
    }

    #[tokio::test]
    async fn test_resolve_hydrates_from_build_plan_shadow_when_no_tasks_registered() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(BuildPlanShadowStore::new(temp.path().to_path_buf()));
        let history = Arc::new(ExecutionHistoryServiceImpl::new(
            temp.path().join("history"),
        ));
        let svc = TaskGraphServiceImpl::with_history_and_shadow(history, Arc::clone(&store));
        let build_id = "shadow-build";

        store
            .persist_plan(
                &super::super::build_plan_ir::CanonicalBuildPlan {
                    schema_version: super::super::build_plan_ir::BUILD_PLAN_SCHEMA_VERSION,
                    build_id: build_id.to_string(),
                    projects: Vec::new(),
                    tasks: vec![
                        super::super::build_plan_ir::CanonicalBuildPlanTask {
                            path: ":compileJava".to_string(),
                            project_path: ":".to_string(),
                            implementation_id: "JavaCompile".to_string(),
                            depends_on: vec![":generateSources".to_string()],
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
                            input_specs: Vec::new(),
                            output_specs: Vec::new(),
                            environment_inputs: Vec::new(),
                            system_property_inputs: Vec::new(),
                            diagnostics: Vec::new(),
                        },
                        super::super::build_plan_ir::CanonicalBuildPlanTask {
                            path: ":generateSources".to_string(),
                            project_path: ":".to_string(),
                            implementation_id: "GenerateSources".to_string(),
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
                        },
                    ],
                    dependencies: Vec::new(),
                    toolchains: Vec::new(),
                    metadata: Default::default(),
                },
                "test-shadow",
            )
            .unwrap();

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: build_id.to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.has_cycles);
        assert_eq!(resp.total_tasks, 2);
        assert_eq!(resp.execution_order.len(), 2);
        assert_eq!(resp.execution_order[0].task_path, ":generateSources");
        assert_eq!(resp.execution_order[1].task_path, ":compileJava");
        assert_eq!(
            resp.execution_order[1].task_type,
            "org.gradle.api.tasks.compile.JavaCompile"
        );
        assert_eq!(resp.plan_source, "build-plan-shadow");
    }

    #[test]
    fn test_java_compile_contract_lowers_to_native_with_context_options() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
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
                    value: "/repo/src/main/java/App.java".to_string(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "release".to_string(),
                    kind: "value".to_string(),
                    value: "VERSION_17".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "sourceCompatibility".to_string(),
                    kind: "value".to_string(),
                    value: "VERSION_1_8".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "classes".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/classes/java/main".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "JavaCompile");
        assert_eq!(context["source_files"][0], "/repo/src/main/java/App.java");
        assert_eq!(context["target_dir"], "/repo/build/classes/java/main");
        assert_eq!(context["options"]["release"], "17");
        assert!(context["options"].get("source_version").is_none());
    }

    #[test]
    fn test_java_compile_without_sources_lowers_to_lifecycle() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":compileTestJava".to_string(),
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
            input_specs: Vec::new(),
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "classes".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/classes/java/test".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "Lifecycle");
    }

    #[test]
    fn test_test_contract_lowers_to_native_test_exec_with_context_options() {
        let separator = if cfg!(windows) { ";" } else { ":" };
        let classpath = format!(
            "/repo/build/classes/java/test{separator}/repo/libs/junit-platform-console-standalone.jar"
        );
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":test".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.testing.Test".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "test".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "classpath".to_string(),
                    kind: "value".to_string(),
                    value: classpath.clone(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "test_classes_dirs".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/classes/java/test".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "max_heap_size".to_string(),
                    kind: "value".to_string(),
                    value: "1g".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "scan_classpath".to_string(),
                    kind: "value".to_string(),
                    value: "true".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "test_filter".to_string(),
                    kind: "value".to_string(),
                    value: "example.*Test".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "test_filter_includes".to_string(),
                    kind: "value".to_string(),
                    value: "example.*Test".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "test_filter_excludes".to_string(),
                    kind: "value".to_string(),
                    value: "example.Legacy*".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "include_tags".to_string(),
                    kind: "value".to_string(),
                    value: "fast,integration".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "exclude_tags".to_string(),
                    kind: "value".to_string(),
                    value: "slow".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "results".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/test-results/test".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "TestExec");
        assert_eq!(context["source_files"][0], "/repo/build/classes/java/test");
        assert_eq!(context["target_dir"], "/repo/build/test-results/test");
        assert_eq!(context["options"]["classpath"], classpath);
        assert_eq!(context["options"]["max_heap_mb"], "1024");
        assert_eq!(context["options"]["scan_classpath"], "true");
        assert_eq!(context["options"]["test_filter"], "example.*Test");
        assert_eq!(context["options"]["test_filter_includes"], "example.*Test");
        assert_eq!(
            context["options"]["test_filter_excludes"],
            "example.Legacy*"
        );
        assert_eq!(context["options"]["include_tags"], "fast,integration");
        assert_eq!(context["options"]["exclude_tags"], "slow");
    }

    #[test]
    fn test_test_contract_with_unsupported_filters_does_not_lower_to_native() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":test".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.testing.Test".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "test".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "classpath".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/libs/junit-platform-console-standalone.jar".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "test_classes_dirs".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/classes/java/test".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "test_unsupported_filters".to_string(),
                    kind: "value".to_string(),
                    value: "true".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "results".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/test-results/test".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(
            executable_task_type(&task),
            "org.gradle.api.tasks.testing.Test"
        );
    }

    #[test]
    fn test_test_with_missing_test_classes_and_no_classpath_lowers_to_lifecycle() {
        let temp = tempfile::tempdir().unwrap();
        let missing_test_classes = temp.path().join("build/classes/java/test");
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":test".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.testing.Test".to_string(),
            depends_on: vec![":testClasses".to_string()],
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "test".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "test_classes_dirs".to_string(),
                    kind: "value".to_string(),
                    value: missing_test_classes.to_string_lossy().into_owned(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "results".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/test-results/test".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "Lifecycle");
    }

    #[test]
    fn test_exec_contract_lowers_to_native_exec_with_context_options() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":generateFile".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.Exec".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "not-cacheable".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "external-process".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "executable".to_string(),
                    kind: "value".to_string(),
                    value: "/usr/bin/touch".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "args".to_string(),
                    kind: "value".to_string(),
                    value: "generated.txt".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "environment_json".to_string(),
                    kind: "value".to_string(),
                    value: "{\"NATIVE_EXEC_ENV\":\"from-task\"}".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "working_dir".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/exec".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "ignore_exit_value".to_string(),
                    kind: "value".to_string(),
                    value: "false".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "output".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/exec".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "Exec");
        assert_eq!(context["options"]["executable"], "/usr/bin/touch");
        assert_eq!(context["options"]["args"], "generated.txt");
        assert_eq!(
            context["options"]["environment_json"],
            "{\"NATIVE_EXEC_ENV\":\"from-task\"}"
        );
        assert_eq!(context["options"]["working_dir"], "/repo/build/exec");
        assert_eq!(context["options"]["ignore_exit_value"], "false");
    }

    #[test]
    fn test_java_exec_contract_lowers_to_native_java_exec_with_context_options() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":runTool".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.JavaExec".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "not-cacheable".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "external-process".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "java_home".to_string(),
                    kind: "value".to_string(),
                    value: "/jdk".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "classpath".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/classes/java/main".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "main_class".to_string(),
                    kind: "value".to_string(),
                    value: "example.Tool".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "args".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/resources/javaexec-result.txt expected-token".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "jvm_args".to_string(),
                    kind: "value".to_string(),
                    value: "-Dnative=true -Xmx128m".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "max_heap_size".to_string(),
                    kind: "value".to_string(),
                    value: "256m".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "system_properties".to_string(),
                    kind: "value".to_string(),
                    value: "native.prop=from-task".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "environment_json".to_string(),
                    kind: "value".to_string(),
                    value: "{\"NATIVE_JAVA_EXEC_ENV\":\"from-task\"}".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "working_dir".to_string(),
                    kind: "value".to_string(),
                    value: "/repo".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "ignore_exit_value".to_string(),
                    kind: "value".to_string(),
                    value: "false".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "output".to_string(),
                    kind: "file".to_string(),
                    path: "/repo/build/resources/javaexec-result.txt".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "JavaExec");
        assert_eq!(context["options"]["java_home"], "/jdk");
        assert_eq!(
            context["options"]["classpath"],
            "/repo/build/classes/java/main"
        );
        assert_eq!(context["options"]["main_class"], "example.Tool");
        assert_eq!(
            context["options"]["args"],
            "/repo/build/resources/javaexec-result.txt expected-token"
        );
        assert_eq!(context["options"]["jvm_args"], "-Dnative=true -Xmx128m");
        assert_eq!(context["options"]["max_heap_size"], "256m");
        assert_eq!(
            context["options"]["system_properties"],
            "native.prop=from-task"
        );
        assert_eq!(
            context["options"]["environment_json"],
            "{\"NATIVE_JAVA_EXEC_ENV\":\"from-task\"}"
        );
        assert_eq!(context["options"]["working_dir"], "/repo");
        assert_eq!(context["options"]["ignore_exit_value"], "false");
    }

    #[test]
    fn test_java_exec_without_main_class_does_not_lower_to_native() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":runTool".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.JavaExec".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "not-cacheable".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "external-process".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "classpath".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/classes/java/main".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: Vec::new(),
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "org.gradle.api.tasks.JavaExec");
    }

    #[test]
    fn test_java_exec_without_captured_or_inferable_classpath_does_not_lower_to_native() {
        let temp = tempfile::tempdir().unwrap();

        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":runTool".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.JavaExec".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "not-cacheable".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "external-process".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "main_class".to_string(),
                    kind: "value".to_string(),
                    value: "example.Tool".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "working_dir".to_string(),
                    kind: "value".to_string(),
                    value: temp.path().to_string_lossy().into_owned(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: Vec::new(),
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "org.gradle.api.tasks.JavaExec");
    }

    #[test]
    fn test_java_exec_without_captured_classpath_infers_standard_java_plugin_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let classes_dir = temp.path().join("build/classes/java/main");
        std::fs::create_dir_all(&classes_dir).unwrap();

        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":runTool".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.JavaExec".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "not-cacheable".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "external-process".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "main_class".to_string(),
                    kind: "value".to_string(),
                    value: "example.Tool".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "working_dir".to_string(),
                    kind: "value".to_string(),
                    value: temp.path().to_string_lossy().into_owned(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "output".to_string(),
                    kind: "file".to_string(),
                    path: temp
                        .path()
                        .join("build/resources/javaexec-result.txt")
                        .to_string_lossy()
                        .into_owned(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "JavaExec");
        assert_eq!(
            context["options"]["classpath"],
            classes_dir.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn test_start_scripts_contract_lowers_to_native_with_context_options() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":startScripts".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.jvm.application.tasks.CreateStartScripts".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "start-scripts".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "application_name".to_string(),
                    kind: "value".to_string(),
                    value: "demo-app".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "main_class".to_string(),
                    kind: "value".to_string(),
                    value: "example.Main".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "classpath".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/libs/demo-app.jar".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "output_dir".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/scripts".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "opts_environment_var".to_string(),
                    kind: "value".to_string(),
                    value: "DEMO_APP_OPTS".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "output".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/scripts".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "CreateStartScripts");
        assert_eq!(context["target_dir"], "/repo/build/scripts");
        assert_eq!(context["options"]["application_name"], "demo-app");
        assert_eq!(context["options"]["main_class"], "example.Main");
        assert_eq!(
            context["options"]["classpath"],
            "/repo/build/libs/demo-app.jar"
        );
        assert_eq!(context["options"]["output_dir"], "/repo/build/scripts");
        assert_eq!(context["options"]["opts_environment_var"], "DEMO_APP_OPTS");
    }

    #[test]
    fn test_start_scripts_without_main_class_lowers_to_fail_closed_native_executor() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":startScripts".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.jvm.application.tasks.CreateStartScripts".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "start-scripts".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "application_name".to_string(),
                    kind: "value".to_string(),
                    value: "demo-app".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "classpath".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/libs/demo-app.jar".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "output_dir".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/scripts".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: Vec::new(),
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "CreateStartScripts");
    }

    #[test]
    fn test_javadoc_contract_lowers_to_native_with_context_options() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":javadoc".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.javadoc.Javadoc".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "documentation".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "source0".to_string(),
                    kind: "source".to_string(),
                    value: "/repo/src/main/java/App.java".to_string(),
                    normalization: "relative".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "java_home".to_string(),
                    kind: "value".to_string(),
                    value: "/jdk".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "classpath".to_string(),
                    kind: "value".to_string(),
                    value: "/repo/build/classes/java/main".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "no_timestamp".to_string(),
                    kind: "value".to_string(),
                    value: "true".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "output".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/docs/javadoc".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "Javadoc");
        assert_eq!(context["source_files"][0], "/repo/src/main/java/App.java");
        assert_eq!(context["target_dir"], "/repo/build/docs/javadoc");
        assert_eq!(context["options"]["java_home"], "/jdk");
        assert_eq!(context["options"]["no_timestamp"], "true");
    }

    #[test]
    fn test_javadoc_without_sources_does_not_lower_to_native() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":javadoc".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.javadoc.Javadoc".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "documentation".to_string(),
            input_specs: Vec::new(),
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "output".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/docs/javadoc".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(
            executable_task_type(&task),
            "org.gradle.api.tasks.javadoc.Javadoc"
        );
    }

    #[test]
    fn test_copy_contract_lowers_permissions_to_native_options() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":copyAssets".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.Copy".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "file-transform".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "input0".to_string(),
                    kind: "path".to_string(),
                    value: "/repo/src/assets".to_string(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "file_permissions".to_string(),
                    kind: "value".to_string(),
                    value: "493".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "dir_permissions".to_string(),
                    kind: "value".to_string(),
                    value: "448".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "destination".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/assets".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "Copy");
        assert_eq!(context["options"]["file_permissions"], "493");
        assert_eq!(context["options"]["dir_permissions"], "448");
    }

    #[test]
    fn test_process_resources_without_sources_lowers_to_lifecycle() {
        let temp = tempfile::tempdir().unwrap();
        let missing_resources = temp.path().join("src/main/resources");
        let output_dir = temp.path().join("build/resources/main");
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":processResources".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.language.jvm.tasks.ProcessResources".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "file-transform".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "input0".to_string(),
                    kind: "path".to_string(),
                    value: missing_resources.to_string_lossy().into_owned(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "destination".to_string(),
                    kind: "directory".to_string(),
                    path: output_dir.to_string_lossy().into_owned(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "Lifecycle");
    }

    #[test]
    fn test_process_resources_with_sources_lowers_to_native_copy() {
        let temp = tempfile::tempdir().unwrap();
        let resources_dir = temp.path().join("src/main/resources");
        let output_dir = temp.path().join("build/resources/main");
        std::fs::create_dir_all(&resources_dir).unwrap();
        std::fs::write(resources_dir.join("app.properties"), "name=app\n").unwrap();

        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":processResources".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.language.jvm.tasks.ProcessResources".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "file-transform".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "input0".to_string(),
                    kind: "path".to_string(),
                    value: resources_dir.to_string_lossy().into_owned(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "destination".to_string(),
                    kind: "directory".to_string(),
                    path: output_dir.to_string_lossy().into_owned(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "Copy");
    }

    #[test]
    fn test_decorated_process_resources_lowers_to_native_copy() {
        let temp = tempfile::tempdir().unwrap();
        let resources_dir = temp.path().join("src/main/resources");
        let output_dir = temp.path().join("build/resources/main");
        std::fs::create_dir_all(&resources_dir).unwrap();
        std::fs::write(resources_dir.join("app.properties"), "name=app\n").unwrap();

        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":processResources".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.language.jvm.tasks.ProcessResources_Decorated"
                .to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "file-transform".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "input0".to_string(),
                    kind: "path".to_string(),
                    value: resources_dir.to_string_lossy().into_owned(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "destination".to_string(),
                    kind: "directory".to_string(),
                    path: output_dir.to_string_lossy().into_owned(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(logical_gradle_task_type("Jar_Decorated"), "Jar");
        assert_eq!(executable_task_type(&task), "Copy");
    }

    #[test]
    fn test_copy_with_unsupported_custom_actions_does_not_lower_to_native() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":copyCustom".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.Copy".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "file-transform".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "input0".to_string(),
                    kind: "path".to_string(),
                    value: "/repo/src/assets".to_string(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "copy_unsupported_custom_actions".to_string(),
                    kind: "value".to_string(),
                    value: "true".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "destination".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/assets".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "org.gradle.api.tasks.Copy");
    }

    #[test]
    fn test_copy_with_symlink_inputs_lowers_to_native_copy() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":copySymlink".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.Copy".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "file-transform".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "input0".to_string(),
                    kind: "path".to_string(),
                    value: "/repo/src/link.txt".to_string(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "copy_contains_symlinks".to_string(),
                    kind: "value".to_string(),
                    value: "true".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "destination".to_string(),
                    kind: "directory".to_string(),
                    path: "/repo/build/assets".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "Copy");
    }

    #[test]
    fn test_delete_contract_lowers_destroyables_to_native_context() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":clean".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.Delete".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "not-cacheable".to_string(),
            local_state: Vec::new(),
            destroyables: vec![
                "/repo/build".to_string(),
                "/repo/generated/stale.txt".to_string(),
            ],
            action_kind: "delete".to_string(),
            input_specs: Vec::new(),
            output_specs: Vec::new(),
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "Delete");
        assert_eq!(context["source_files"][0], "/repo/build");
        assert_eq!(context["source_files"][1], "/repo/generated/stale.txt");
        assert_eq!(context["target_dir"], "");
    }

    #[test]
    fn test_zip_archive_contract_lowers_to_native_zip_executor() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":distZip".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.bundling.Zip".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "archive".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "input0".to_string(),
                    kind: "path".to_string(),
                    value: "/repo/build/install/app".to_string(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "archive_file_name".to_string(),
                    kind: "value".to_string(),
                    value: "app.zip".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "main_class".to_string(),
                    kind: "value".to_string(),
                    value: "com.example.Main".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "manifest.Implementation-Title".to_string(),
                    kind: "value".to_string(),
                    value: "app".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "include_empty_dirs".to_string(),
                    kind: "value".to_string(),
                    value: "false".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "file_permissions".to_string(),
                    kind: "value".to_string(),
                    value: "493".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "dir_permissions".to_string(),
                    kind: "value".to_string(),
                    value: "448".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "archive".to_string(),
                    kind: "file".to_string(),
                    path: "/repo/build/distributions/app.zip".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "Zip");
        assert_eq!(context["source_files"][0], "/repo/build/install/app");
        assert_eq!(context["target_dir"], "/repo/build/distributions");
        assert_eq!(context["options"]["jarName"], "app.zip");
        assert_eq!(context["options"]["mainClass"], "com.example.Main");
        assert_eq!(context["options"]["manifest.Implementation-Title"], "app");
        assert_eq!(context["options"]["include_empty_dirs"], "false");
        assert_eq!(context["options"]["file_permissions"], "493");
        assert_eq!(context["options"]["dir_permissions"], "448");
    }

    #[test]
    fn test_archive_with_symlink_inputs_lowers_to_native_archive() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":distZip".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.bundling.Zip".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "archive".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "input0".to_string(),
                    kind: "path".to_string(),
                    value: "/repo/src/link.txt".to_string(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "copy_contains_symlinks".to_string(),
                    kind: "value".to_string(),
                    value: "true".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "archive".to_string(),
                    kind: "file".to_string(),
                    path: "/repo/build/distributions/app.zip".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_eq!(executable_task_type(&task), "Zip");
    }

    #[test]
    fn test_tar_archive_contract_lowers_to_native_tar_executor() {
        let task = super::super::build_plan_ir::CanonicalBuildPlanTask {
            path: ":distTar".to_string(),
            project_path: ":".to_string(),
            implementation_id: "org.gradle.api.tasks.bundling.Tar".to_string(),
            depends_on: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            worker_isolation: "in-process".to_string(),
            should_run_after: Vec::new(),
            must_run_after: Vec::new(),
            finalized_by: Vec::new(),
            cacheability: "declared-outputs".to_string(),
            local_state: Vec::new(),
            destroyables: Vec::new(),
            action_kind: "archive".to_string(),
            input_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "input0".to_string(),
                    kind: "path".to_string(),
                    value: "/repo/build/install/app".to_string(),
                    normalization: "absolute-path".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "archive_file_name".to_string(),
                    kind: "value".to_string(),
                    value: "app.tar".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "archive_compression".to_string(),
                    kind: "value".to_string(),
                    value: "gzip".to_string(),
                    normalization: "scalar".to_string(),
                    optional: true,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "include_empty_dirs".to_string(),
                    kind: "value".to_string(),
                    value: "false".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "file_permissions".to_string(),
                    kind: "value".to_string(),
                    value: "493".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
                super::super::build_plan_ir::CanonicalBuildPlanTaskInputSpec {
                    name: "dir_permissions".to_string(),
                    kind: "value".to_string(),
                    value: "448".to_string(),
                    normalization: "scalar".to_string(),
                    optional: false,
                },
            ],
            output_specs: vec![
                super::super::build_plan_ir::CanonicalBuildPlanTaskOutputSpec {
                    name: "archive".to_string(),
                    kind: "file".to_string(),
                    path: "/repo/build/distributions/app.tar".to_string(),
                },
            ],
            environment_inputs: Vec::new(),
            system_property_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let task_type = executable_task_type(&task);
        let context: serde_json::Value =
            serde_json::from_str(&execution_context_json(&task, &task_type)).unwrap();

        assert_eq!(task_type, "Tar");
        assert_eq!(context["source_files"][0], "/repo/build/install/app");
        assert_eq!(context["target_dir"], "/repo/build/distributions");
        assert_eq!(context["options"]["tarName"], "app.tar");
        assert_eq!(context["options"]["compression"], "gzip");
        assert_eq!(context["options"]["include_empty_dirs"], "false");
        assert_eq!(context["options"]["file_permissions"], "493");
        assert_eq!(context["options"]["dir_permissions"], "448");
    }

    #[tokio::test]
    async fn test_prefer_build_plan_shadow_replaces_registered_tasks() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(BuildPlanShadowStore::new(temp.path().to_path_buf()));
        let history = Arc::new(ExecutionHistoryServiceImpl::new(
            temp.path().join("history"),
        ));
        let svc = TaskGraphServiceImpl::with_history_and_shadow(history, Arc::clone(&store));
        let build_id = "prefer-shadow-build";

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: build_id.to_string(),
            task_path: ":staleFallbackTask".to_string(),
            depends_on: Vec::new(),
            should_execute: true,
            task_type: "Stale".to_string(),
            input_files: Vec::new(),
        }))
        .await
        .unwrap();

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

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: build_id.to_string(),
                prefer_build_plan_shadow: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_tasks, 1);
        assert_eq!(resp.execution_order[0].task_path, ":fromShadow");
        assert_eq!(resp.plan_source, "build-plan-shadow");
    }

    #[tokio::test]
    async fn test_prefer_build_plan_shadow_keeps_registered_tasks_when_shadow_missing() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(BuildPlanShadowStore::new(temp.path().to_path_buf()));
        let history = Arc::new(ExecutionHistoryServiceImpl::new(
            temp.path().join("history"),
        ));
        let svc = TaskGraphServiceImpl::with_history_and_shadow(history, store);

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "missing-shadow-build".to_string(),
            task_path: ":registered".to_string(),
            depends_on: Vec::new(),
            should_execute: true,
            task_type: "Registered".to_string(),
            input_files: Vec::new(),
        }))
        .await
        .unwrap();

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "missing-shadow-build".to_string(),
                prefer_build_plan_shadow: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_tasks, 1);
        assert_eq!(resp.execution_order[0].task_path, ":registered");
        assert_eq!(resp.plan_source, "registered-tasks");
    }

    #[tokio::test]
    async fn test_register_and_resolve_simple_chain() {
        let svc = make_svc();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":a".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":b".to_string(),
            depends_on: vec![":a".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":c".to_string(),
            depends_on: vec![":b".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "test".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.has_cycles);
        assert_eq!(resp.total_tasks, 3);
        assert_eq!(resp.execution_order.len(), 3);
        assert_eq!(resp.execution_order[0].task_path, ":a");
        assert_eq!(resp.execution_order[1].task_path, ":b");
        assert_eq!(resp.execution_order[2].task_path, ":c");
        assert_eq!(resp.execution_order[0].execution_order, 1);
        assert_eq!(resp.execution_order[1].execution_order, 2);
        assert_eq!(resp.execution_order[2].execution_order, 3);
        assert_eq!(resp.plan_source, "registered-tasks");
    }

    #[tokio::test]
    async fn test_clear_build_tasks_removes_only_requested_build() {
        let svc = make_svc();

        for (build_id, task_path) in [("build-a", ":a"), ("build-a", ":b"), ("build-b", ":other")] {
            svc.register_task(Request::new(RegisterTaskRequest {
                build_id: build_id.to_string(),
                task_path: task_path.to_string(),
                depends_on: Vec::new(),
                should_execute: true,
                task_type: "Task".to_string(),
                input_files: vec![format!("{}.txt", task_path)],
            }))
            .await
            .unwrap();
        }

        let cleared = svc
            .clear_build_tasks(Request::new(ClearBuildTasksRequest {
                build_id: "build-a".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(cleared.cleared_tasks, 2);

        let build_a = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "build-a".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();
        let build_b = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "build-b".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(build_a.total_tasks, 0);
        assert_eq!(build_b.total_tasks, 1);
        assert_eq!(build_b.execution_order[0].task_path, ":other");
    }

    #[tokio::test]
    async fn test_parallel_tasks() {
        let svc = make_svc();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":root".to_string(),
            depends_on: vec![":a".to_string(), ":b".to_string(), ":c".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        for t in &[":a", ":b", ":c"] {
            svc.register_task(Request::new(RegisterTaskRequest {
                build_id: "test".to_string(),
                task_path: t.to_string(),
                depends_on: vec![],
                should_execute: true,
                task_type: "Task".to_string(),
                input_files: vec![],
            }))
            .await
            .unwrap();
        }

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "test".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.ready_to_execute, 3);
        // The three independent tasks come first (in any order), then root
        let independent: Vec<_> = resp.execution_order[..3]
            .iter()
            .map(|n| n.task_path.as_str())
            .collect();
        assert_eq!(independent.len(), 3);
        assert!(independent.contains(&":a"));
        assert!(independent.contains(&":b"));
        assert!(independent.contains(&":c"));
    }

    #[tokio::test]
    async fn test_cycle_detection() {
        let svc = make_svc();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":a".to_string(),
            depends_on: vec![":b".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":b".to_string(),
            depends_on: vec![":a".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "test".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.has_cycles);
    }

    #[tokio::test]
    async fn test_progress_tracking() {
        let svc = make_svc();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":compile".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        svc.task_started(Request::new(TaskStartedRequest {
            build_id: "test".to_string(),
            task_path: ":compile".to_string(),
            start_time_ms: 100,
        }))
        .await
        .unwrap();

        let progress = svc
            .get_progress(Request::new(GetProgressRequest {
                build_id: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(progress.executing, 1);
        assert_eq!(progress.tasks[0].status, "EXECUTING");

        svc.task_finished(Request::new(TaskFinishedRequest {
            build_id: "test".to_string(),
            task_path: ":compile".to_string(),
            duration_ms: 500,
            success: true,
            outcome: "EXECUTED".to_string(),
        }))
        .await
        .unwrap();

        let progress = svc
            .get_progress(Request::new(GetProgressRequest {
                build_id: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(progress.completed, 1);
        assert_eq!(progress.executing, 0);
    }

    #[tokio::test]
    async fn test_task_graph_with_history_durations() {
        let history = Arc::new(ExecutionHistoryServiceImpl::new(std::path::PathBuf::new()));
        let svc = TaskGraphServiceImpl::with_history(Arc::clone(&history));

        // Pre-populate history with known durations
        history.store_task_duration(":fast", 100);
        history.store_task_duration(":slow", 5000);
        history.store_task_duration(":medium", 500);

        // Register tasks — they should pick up historical durations
        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":fast".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":slow".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":no_history".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "test".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Verify estimated durations are picked up from history
        let mut durations: Vec<_> = resp
            .execution_order
            .iter()
            .map(|n| (n.task_path.as_str(), n.estimated_duration_ms))
            .collect();
        durations.sort_by_key(|(_, d)| *d);

        assert_eq!(durations[0].1, 0); // :no_history
        assert_eq!(durations[1].1, 100); // :fast
        assert_eq!(durations[2].1, 5000); // :slow

        // Critical path should reflect the longest path
        assert_eq!(resp.critical_path_ms, 5000);

        // Now finish a task and verify duration is updated in history
        svc.task_finished(Request::new(TaskFinishedRequest {
            build_id: "test".to_string(),
            task_path: ":fast".to_string(),
            duration_ms: 150,
            success: true,
            outcome: "EXECUTED".to_string(),
        }))
        .await
        .unwrap();

        // History should be updated
        assert_eq!(history.get_task_duration(":fast"), 150);
    }

    #[tokio::test]
    async fn test_critical_path_with_dependencies() {
        let history = Arc::new(ExecutionHistoryServiceImpl::new(std::path::PathBuf::new()));
        let svc = TaskGraphServiceImpl::with_history(Arc::clone(&history));

        // A(100ms) -> B(200ms) -> C(50ms)
        // D(300ms) -> C
        history.store_task_duration(":a", 100);
        history.store_task_duration(":b", 200);
        history.store_task_duration(":c", 50);
        history.store_task_duration(":d", 300);

        for (path, deps) in [
            (":a", vec![] as Vec<&str>),
            (":b", vec![":a"]),
            (":c", vec![":b", ":d"]),
            (":d", vec![]),
        ] {
            svc.register_task(Request::new(RegisterTaskRequest {
                build_id: "test".to_string(),
                task_path: path.to_string(),
                depends_on: deps.into_iter().map(String::from).collect(),
                should_execute: true,
                task_type: "Task".to_string(),
                input_files: vec![],
            }))
            .await
            .unwrap();
        }

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "test".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Critical path: D(300) -> C(50) = 350, or A(100) -> B(200) -> C(50) = 350
        assert_eq!(resp.critical_path_ms, 350);
        assert!(!resp.has_cycles);
    }

    #[tokio::test]
    async fn test_register_duplicate_task() {
        let svc = make_svc();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":a".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        // Registering same task again should overwrite
        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "test".to_string(),
            task_path: ":a".to_string(),
            depends_on: vec![":b".to_string()],
            should_execute: false,
            task_type: "Other".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "test".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_tasks, 1);
    }

    #[tokio::test]
    async fn test_task_started_without_register() {
        let svc = make_svc();

        // Starting a task that was never registered should succeed
        let resp = svc
            .task_started(Request::new(TaskStartedRequest {
                build_id: String::new(),
                task_path: ":nonexistent".to_string(),
                start_time_ms: 100,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.acknowledged);
    }

    #[tokio::test]
    async fn test_task_finished_without_register() {
        let svc = make_svc();

        // Finishing a task that was never registered should succeed
        let resp = svc
            .task_finished(Request::new(TaskFinishedRequest {
                build_id: String::new(),
                task_path: ":nonexistent".to_string(),
                duration_ms: 100,
                success: true,
                outcome: "SUCCESS".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.acknowledged);
    }

    #[tokio::test]
    async fn test_progress_empty_graph() {
        let svc = make_svc();

        let resp = svc
            .get_progress(Request::new(GetProgressRequest {
                build_id: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.tasks.len(), 0);
        assert_eq!(resp.completed, 0);
    }

    #[tokio::test]
    async fn test_resolve_empty_graph() {
        let svc = make_svc();

        let resp = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "empty".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_tasks, 0);
        assert!(!resp.has_cycles);
        assert!(resp.execution_order.is_empty());
        assert_eq!(resp.plan_source, "empty");
    }

    /// Concurrent builds with the same task paths must not interfere.
    #[tokio::test]
    async fn test_concurrent_builds_isolated() {
        let svc = make_svc();

        // Build 1 registers :compileJava → :processResources
        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "build-1".to_string(),
            task_path: ":compileJava".to_string(),
            depends_on: vec![":processResources".to_string()],
            should_execute: true,
            task_type: "JavaCompile".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "build-1".to_string(),
            task_path: ":processResources".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Copy".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        // Build 2 registers :compileJava with no dependencies (different graph)
        svc.register_task(Request::new(RegisterTaskRequest {
            build_id: "build-2".to_string(),
            task_path: ":compileJava".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "JavaCompile".to_string(),
            input_files: vec![],
        }))
        .await
        .unwrap();

        // Build 1 plan should see 2 tasks with dependency
        let plan1 = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "build-1".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(plan1.total_tasks, 2);
        assert_eq!(plan1.ready_to_execute, 1);
        assert!(!plan1.has_cycles);

        // Build 2 plan should see 1 task, no dependencies, ready immediately
        let plan2 = svc
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: "build-2".to_string(),
                prefer_build_plan_shadow: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(plan2.total_tasks, 1);
        assert_eq!(plan2.ready_to_execute, 1);
        assert!(!plan2.has_cycles);

        // Finish :compileJava in build 2 — should not affect build 1
        svc.task_finished(Request::new(TaskFinishedRequest {
            build_id: "build-2".to_string(),
            task_path: ":compileJava".to_string(),
            duration_ms: 200,
            success: true,
            outcome: "EXECUTED".to_string(),
        }))
        .await
        .unwrap();

        // Build 1 progress should still show :compileJava as PENDING (not EXECUTED)
        let progress1 = svc
            .get_progress(Request::new(GetProgressRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(progress1.total, 2);
        let compile_status: Vec<_> = progress1
            .tasks
            .iter()
            .filter(|t| t.task_path == ":compileJava")
            .collect();
        assert_eq!(compile_status.len(), 1);
        assert_eq!(compile_status[0].status, "PENDING");

        // Build 2 progress should show :compileJava as completed
        let progress2 = svc
            .get_progress(Request::new(GetProgressRequest {
                build_id: "build-2".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(progress2.total, 1);
        assert_eq!(progress2.completed, 1);
    }
}
