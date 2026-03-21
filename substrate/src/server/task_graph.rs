use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    task_graph_service_server::TaskGraphService, ExecutionNode, GetProgressRequest,
    GetProgressResponse, RegisterTaskRequest, RegisterTaskResponse, ResolveExecutionPlanRequest,
    ResolveExecutionPlanResponse, TaskFinishedRequest, TaskFinishedResponse,
    TaskProgress, TaskStartedRequest, TaskStartedResponse,
};

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
#[derive(Default)]
pub struct TaskGraphServiceImpl {
    tasks: DashMap<String, TaskNode>,
    request_counter: AtomicI64,
}

impl TaskGraphServiceImpl {
    pub fn new() -> Self {
        Self {
            tasks: DashMap::new(),
            request_counter: AtomicI64::new(0),
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
        // Dynamic programming: longest path from source to each node
        let mut longest: HashMap<String, i64> = HashMap::new();

        for entry in self.tasks.iter() {
            let path = entry.task_path.clone();
            let mut max_dep = 0i64;
            for dep in &entry.depends_on {
                let dep_duration = longest.get(dep).copied().unwrap_or(0);
                max_dep = max_dep.max(dep_duration);
            }
            let current = entry.estimated_duration_ms + max_dep;
            longest.insert(path, current);
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

        self.tasks.insert(
            req.task_path.clone(),
            TaskNode {
                task_path: req.task_path,
                depends_on: req.depends_on,
                should_execute: req.should_execute,
                task_type: req.task_type,
                estimated_duration_ms: 0,
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
}
