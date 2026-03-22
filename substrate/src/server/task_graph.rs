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

/// Task graph node stored internally.
struct TaskNode {
    task_path: String,
    depends_on: Vec<String>,
    #[allow(dead_code)]
    should_execute: bool,
    #[allow(dead_code)]
    task_type: String,
    estimated_duration_ms: i64,
    status: String,
    start_time_ms: i64,
    duration_ms: i64,
}

/// Rust-native task graph service.
/// Manages dependency resolution and execution scheduling.
pub struct TaskGraphServiceImpl {
    tasks: DashMap<String, TaskNode>,
    request_counter: AtomicI64,
    /// Optional reference to execution history for duration estimates.
    history: Option<Arc<ExecutionHistoryServiceImpl>>,
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
        }
    }

    pub fn with_history(history: Arc<ExecutionHistoryServiceImpl>) -> Self {
        Self {
            tasks: DashMap::new(),
            request_counter: AtomicI64::new(0),
            history: Some(history),
        }
    }

    /// Look up estimated duration from execution history for a task path.
    fn lookup_historical_duration(&self, task_path: &str) -> i64 {
        match &self.history {
            Some(h) => h.get_task_duration(task_path),
            None => 0,
        }
    }

    /// Kahn's algorithm for topological sort with parallel scheduling.
    fn resolve_plan(&self) -> (Vec<ExecutionNode>, i64, bool) {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_tasks: HashSet<String> = HashSet::new();

        // Build adjacency info
        for entry in self.tasks.iter() {
            let path = entry.task_path.clone();
            all_tasks.insert(path.clone());
            in_degree.entry(path.clone()).or_insert(0);

            for dep in &entry.depends_on {
                if self.tasks.contains_key(dep.as_str()) {
                    *in_degree.entry(path.clone()).or_insert(0) += 1;
                    dependents.entry(dep.clone()).or_default().push(path.clone());
                }
            }
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
            order += 1;
            visited_count += 1;

            if let Some(entry) = self.tasks.get(&task) {
                let estimated = entry.estimated_duration_ms;
                execution_order.push(ExecutionNode {
                    task_path: entry.task_path.clone(),
                    dependencies: entry.depends_on.clone(),
                    execution_order: order,
                    estimated_duration_ms: estimated,
                });
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

        if visited_count != all_tasks.len() {
            has_cycles = true;
        }

        // Calculate critical path (longest path through DAG)
        let critical_path_ms = self.calculate_critical_path();

        (execution_order, critical_path_ms, has_cycles)
    }

    fn calculate_critical_path(&self) -> i64 {
        // Topological sort via Kahn's algorithm, then DP for longest path.
        // DashMap iteration order is non-deterministic, so we must sort first.
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();

        for entry in self.tasks.iter() {
            let path = entry.task_path.clone();
            in_degree.entry(path.clone()).or_insert(0);
            for dep in &entry.depends_on {
                if self.tasks.contains_key(dep.as_str()) {
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
            if let Some(entry) = self.tasks.get(task) {
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

        // Look up estimated duration from execution history
        let estimated = self.lookup_historical_duration(&req.task_path);

        self.tasks.insert(
            req.task_path.clone(),
            TaskNode {
                task_path: req.task_path,
                depends_on: req.depends_on,
                should_execute: req.should_execute,
                task_type: req.task_type,
                estimated_duration_ms: estimated,
                status: "PENDING".to_string(),
                start_time_ms: 0,
                duration_ms: 0,
            },
        );

        Ok(Response::new(RegisterTaskResponse { success: true }))
    }

    async fn resolve_execution_plan(
        &self,
        request: Request<ResolveExecutionPlanRequest>,
    ) -> Result<Response<ResolveExecutionPlanResponse>, Status> {
        let _req = request.into_inner();
        self.request_counter.fetch_add(1, Ordering::Relaxed);

        let (execution_order, critical_path_ms, has_cycles) = self.resolve_plan();

        let total = self.tasks.len() as i32;
        let ready = execution_order
            .iter()
            .filter(|n| n.dependencies.is_empty())
            .count() as i32;

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

        if let Some(mut task) = self.tasks.get_mut(&req.task_path) {
            task.status = "EXECUTING".to_string();
            task.start_time_ms = req.start_time_ms;
        }

        Ok(Response::new(TaskStartedResponse { acknowledged: true }))
    }

    async fn task_finished(
        &self,
        request: Request<TaskFinishedRequest>,
    ) -> Result<Response<TaskFinishedResponse>, Status> {
        let req = request.into_inner();

        if let Some(mut task) = self.tasks.get_mut(&req.task_path) {
            task.status = if req.success {
                req.outcome.clone()
            } else {
                "FAILED".to_string()
            };
            task.duration_ms = req.duration_ms;
            // Update estimated duration for future scheduling
            task.estimated_duration_ms = req.duration_ms;
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
        let _req = request.into_inner();

        let mut tasks = Vec::new();
        let mut completed = 0i32;
        let mut executing = 0i32;
        let total = self.tasks.len() as i32;

        for entry in self.tasks.iter() {
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
            task_path: ":a".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            task_path: ":b".to_string(),
            depends_on: vec![":a".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            task_path: ":c".to_string(),
            depends_on: vec![":b".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
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
            task_path: ":root".to_string(),
            depends_on: vec![":a".to_string(), ":b".to_string(), ":c".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
        }))
        .await
        .unwrap();

        for t in &[":a", ":b", ":c"] {
            svc.register_task(Request::new(RegisterTaskRequest {
                task_path: t.to_string(),
                depends_on: vec![],
                should_execute: true,
                task_type: "Task".to_string(),
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
            task_path: ":a".to_string(),
            depends_on: vec![":b".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            task_path: ":b".to_string(),
            depends_on: vec![":a".to_string()],
            should_execute: true,
            task_type: "Task".to_string(),
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
            task_path: ":compile".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
        }))
        .await
        .unwrap();

        svc.task_started(Request::new(TaskStartedRequest {
            task_path: ":compile".to_string(),
            start_time_ms: 100,
        }))
        .await
        .unwrap();

        let progress = svc
            .get_progress(Request::new(GetProgressRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(progress.executing, 1);
        assert_eq!(progress.tasks[0].status, "EXECUTING");

        svc.task_finished(Request::new(TaskFinishedRequest {
            task_path: ":compile".to_string(),
            duration_ms: 500,
            success: true,
            outcome: "EXECUTED".to_string(),
        }))
        .await
        .unwrap();

        let progress = svc
            .get_progress(Request::new(GetProgressRequest {}))
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
            task_path: ":fast".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            task_path: ":slow".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
        }))
        .await
        .unwrap();

        svc.register_task(Request::new(RegisterTaskRequest {
            task_path: ":no_history".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
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
                task_path: path.to_string(),
                depends_on: deps.into_iter().map(String::from).collect(),
                should_execute: true,
                task_type: "Task".to_string(),
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
            task_path: ":a".to_string(),
            depends_on: vec![],
            should_execute: true,
            task_type: "Task".to_string(),
        }))
        .await
        .unwrap();

        // Registering same task again should overwrite
        svc.register_task(Request::new(RegisterTaskRequest {
            task_path: ":a".to_string(),
            depends_on: vec![":b".to_string()],
            should_execute: false,
            task_type: "Other".to_string(),
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
            .get_progress(Request::new(GetProgressRequest {}))
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
}
