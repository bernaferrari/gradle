use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    task_graph_service_server::TaskGraphService, ExecutionNode, GetProgressRequest,
    GetProgressResponse, RegisterTaskRequest, RegisterTaskResponse, ResolveExecutionPlanRequest,
    ResolveExecutionPlanResponse, TaskFinishedRequest, TaskFinishedResponse,
    TaskProgress, TaskStartedRequest, TaskStartedResponse,
};

use super::execution_history::ExecutionHistoryServiceImpl;
use super::scopes::BuildId;

/// Task graph node stored internally.
#[derive(Clone)]
struct TaskNode {
    task_path: String,
    depends_on: Vec<String>,
    should_execute: bool,
    task_type: String,
    estimated_duration_ms: i64,
    status: String,
    start_time_ms: i64,
    duration_ms: i64,
}

/// Rust-native task graph service.
/// Manages dependency resolution and execution scheduling.
/// Tasks are scoped by (BuildId, task_path) to prevent concurrent builds from mixing state.
pub struct TaskGraphServiceImpl {
    tasks: DashMap<(BuildId, String), TaskNode>,
    request_counter: AtomicI64,
    /// Optional reference to execution history for duration estimates.
    history: Option<Arc<ExecutionHistoryServiceImpl>>,
    /// Reverse index: file path -> list of (build_id, task_path) that depend on it.
    /// Used by file-watch integration to invalidate tasks when their inputs change.
    file_to_tasks: DashMap<String, Vec<(BuildId, String)>>,
}

impl Clone for TaskGraphServiceImpl {
    fn clone(&self) -> Self {
        Self {
            tasks: self.tasks.clone(),
            request_counter: AtomicI64::new(self.request_counter.load(Ordering::Relaxed)),
            history: self.history.clone(),
            file_to_tasks: self.file_to_tasks.clone(),
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
            tasks: DashMap::new(),
            request_counter: AtomicI64::new(0),
            history: None,
            file_to_tasks: DashMap::new(),
        }
    }

    pub fn with_history(history: Arc<ExecutionHistoryServiceImpl>) -> Self {
        Self {
            tasks: DashMap::new(),
            request_counter: AtomicI64::new(0),
            history: Some(history),
            file_to_tasks: DashMap::new(),
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
        let mut invalidated = std::collections::HashSet::new();
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

    /// Kahn's algorithm for topological sort with parallel scheduling.
    /// Tasks with `should_execute == false` are excluded from the execution order
    /// since they are already resolved (UP-TO-DATE, SKIPPED, etc.).
    /// Only resolves tasks belonging to the given build_id.
    fn resolve_plan(&self, build_id: &BuildId) -> (Vec<ExecutionNode>, i64, bool) {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_tasks: HashSet<String> = HashSet::new();
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
            *task_type_counts
                .entry(entry.task_type.clone())
                .or_default() += 1;

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
                    dependents.entry(dep.clone()).or_default().push(path.clone());
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

        let mut execution_order = Vec::new();
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
                    });
                }
            }

            // Reduce in-degree for dependents
            if let Some(deps) = dependents.get(&task) {
                for dep in deps {
                    let degree = in_degree.get_mut(dep).unwrap();
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(dep.clone());
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
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();

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
                    dependents.entry(dep.clone()).or_default().push(path.clone());
                }
            }
        }

        let mut queue: VecDeque<String> = VecDeque::new();
        for (task, &deg) in &in_degree {
            if deg == 0 {
                queue.push_back(task.clone());
            }
        }

        let mut topo_order = Vec::new();
        while let Some(task) = queue.pop_front() {
            topo_order.push(task.clone());
            if let Some(deps) = dependents.get(&task) {
                for dep in deps {
                    let degree = in_degree.get_mut(dep).unwrap();
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        // DP: longest path in topological order
        let mut longest: HashMap<String, i64> = HashMap::new();
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

    async fn resolve_execution_plan(
        &self,
        request: Request<ResolveExecutionPlanRequest>,
    ) -> Result<Response<ResolveExecutionPlanResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id.clone());
        self.request_counter.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(build_id = %req.build_id, "Resolving execution plan");

        let (execution_order, critical_path_ms, has_cycles) = self.resolve_plan(&build_id);

        let total = self
            .tasks
            .iter()
            .filter(|e| e.key().0 == build_id)
            .count() as i32;
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
                "Execution plan resolved with excluded tasks"
            );
        }

        Ok(Response::new(ResolveExecutionPlanResponse {
            execution_order,
            total_tasks: total,
            ready_to_execute: ready,
            critical_path_ms,
            has_cycles,
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

        let mut tasks = Vec::new();
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
                "SUCCEEDED" | "FAILED" | "SKIPPED" | "UP_TO_DATE" | "FROM_CACHE"
                | "EXECUTED" | "EXECUTED_INCREMENTALLY" | "EXECUTED_NON_INCREMENTALLY"
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
            }))
            .await
            .unwrap()
            .into_inner();

        // Verify estimated durations are picked up from history
        let mut durations: Vec<_> = resp.execution_order
            .iter()
            .map(|n| (n.task_path.as_str(), n.estimated_duration_ms))
            .collect();
        durations.sort_by_key(|(_, d)| *d);

        assert_eq!(durations[0].1, 0);     // :no_history
        assert_eq!(durations[1].1, 100);   // :fast
        assert_eq!(durations[2].1, 5000);  // :slow

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
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_tasks, 0);
        assert!(!resp.has_cycles);
        assert!(resp.execution_order.is_empty());
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
