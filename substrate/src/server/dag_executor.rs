use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use super::event_dispatcher::EventDispatcher;
use super::scopes::BuildId;
use super::work::WorkerScheduler;

use crate::proto::{
    dag_executor_service_server::DagExecutorService, task_graph_service_server::TaskGraphService,
    AwaitBuildCompletionRequest, AwaitBuildCompletionResponse, BuildEventMessage,
    CancelBuildRequest, CancelBuildResponse, GetBuildStatusRequest, GetBuildStatusResponse,
    GetNextTaskRequest, GetNextTaskResponse, NotifyTaskFinishedRequest,
    NotifyTaskFinishedResponse, NotifyTaskStartedRequest, NotifyTaskStartedResponse,
    ResolveExecutionPlanRequest, StartBuildRequest, StartBuildResponse, TaskFinishedRequest,
    TaskStartedRequest, TaskStatusEntry,
};

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
}

/// Runtime state for an active build execution.
#[allow(dead_code)]
struct BuildExecution {
    build_id: BuildId,
    status: String,
    start_time_ms: i64,
    /// Tasks whose dependencies are all satisfied.
    ready_queue: std::sync::Mutex<VecDeque<String>>,
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
        self.executing.lock().unwrap().len() as i32
    }

    fn pending_count(&self) -> i32 {
        self.tasks
            .values()
            .filter(|t| t.status == "PENDING")
            .count() as i32
    }

    fn failed_count(&self) -> i32 {
        self.tasks
            .values()
            .filter(|t| t.status == "FAILED")
            .count() as i32
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
        matches!(
            self.status.as_str(),
            "COMPLETED" | "FAILED" | "CANCELLED"
        )
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
    /// Event dispatchers for automatic fan-out (console + metrics).
    dispatchers: Vec<Arc<dyn EventDispatcher>>,
    request_counter: AtomicI64,
    builds_started: AtomicI64,
}

impl Clone for DagExecutorServiceImpl {
    fn clone(&self) -> Self {
        Self {
            builds: Arc::clone(&self.builds),
            scheduler: Arc::clone(&self.scheduler),
            task_graph: Arc::clone(&self.task_graph),
            dispatchers: self.dispatchers.clone(),
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
            Vec::new(),
        )
    }
}

impl DagExecutorServiceImpl {
    pub fn new(
        scheduler: Arc<WorkerScheduler>,
        task_graph: Arc<super::task_graph::TaskGraphServiceImpl>,
        dispatchers: Vec<Arc<dyn EventDispatcher>>,
    ) -> Self {
        Self {
            builds: Arc::new(DashMap::new()),
            scheduler,
            task_graph,
            dispatchers,
            request_counter: AtomicI64::new(0),
            builds_started: AtomicI64::new(0),
        }
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

    /// Try to mark dependents as ready after a task finishes.
    /// Returns list of newly ready task paths.
    fn try_unblock_dependents(execution: &BuildExecution, finished_task: &str) -> Vec<String> {
        let mut newly_ready = Vec::new();
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
                                matches!(
                                    d.status.as_str(),
                                    "SUCCEEDED" | "FAILED" | "SKIPPED"
                                )
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
        if !newly_ready.is_empty() {
            let mut queue = execution.ready_queue.lock().unwrap();
            for task in &newly_ready {
                queue.push_back(task.clone());
            }
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
        let mut visited = HashSet::new();
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
                        queue.retain(|t| t != &task_path);
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
        execution
            .tasks
            .values()
            .map(|t| TaskStatusEntry {
                task_path: t.task_path.clone(),
                status: t.status.clone(),
                duration_ms: t.duration_ms,
            })
            .collect()
    }
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

        // Resolve execution plan from TaskGraphService
        let plan_response = self
            .task_graph
            .resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                build_id: req.build_id.clone(),
            }))
            .await
            .map_err(|e| Status::internal(format!("Failed to resolve execution plan: {}", e)))?
            .into_inner();

        if plan_response.has_cycles {
            return Ok(Response::new(StartBuildResponse {
                accepted: false,
                error_message: "Task graph contains cycles".to_string(),
                total_tasks: 0,
                critical_path_ms: 0,
            }));
        }

        let task_filter: Option<HashSet<String>> = if req.task_filter.is_empty() {
            None
        } else {
            Some(req.task_filter.into_iter().collect())
        };

        // Build task slots and dependents map
        let mut tasks = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        let mut ready_queue = VecDeque::new();

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
                dependents.entry(dep.clone()).or_default().push(node.task_path.clone());
            }

            tasks.insert(
                node.task_path.clone(),
                TaskSlot {
                    task_path: node.task_path.clone(),
                    task_type: String::new(), // populated from task_graph if needed
                    status: "PENDING".to_string(),
                    start_time_ms: 0,
                    duration_ms: 0,
                    dependencies: deps,
                },
            );

            // Root tasks (no dependencies) are immediately ready
            if node.dependencies.is_empty() {
                ready_queue.push_back(node.task_path.clone());
            }
        }

        let total_tasks = tasks.len() as i32;

        // If no tasks pass the filter
        if total_tasks == 0 {
            return Ok(Response::new(StartBuildResponse {
                accepted: false,
                error_message: "No tasks to execute".to_string(),
                total_tasks: 0,
                critical_path_ms: 0,
            }));
        }

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        let execution = BuildExecution {
            build_id: build_id.clone(),
            status: "EXECUTING".to_string(),
            start_time_ms: Self::now_ms(),
            ready_queue: std::sync::Mutex::new(ready_queue),
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
            "Build execution started"
        );

        Ok(Response::new(StartBuildResponse {
            accepted: true,
            error_message: String::new(),
            total_tasks,
            critical_path_ms: plan_response.critical_path_ms,
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
                if let Some(task_path) = queue.pop_front() {
                    // Mark as executing
                    if let Ok(mut executing) = execution.executing.lock() {
                        executing.insert(task_path.clone());
                    }
                    if let Some(slot) = execution.tasks.get(&task_path) {
                        return Ok(Response::new(GetNextTaskResponse {
                            task_path,
                            task_type: slot.task_type.clone(),
                            estimated_duration_ms: 0,
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

            Ok(Response::new(NotifyTaskStartedResponse { acknowledged: true }))
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

            let mut newly_ready = Vec::new();

            if req.success {
                newly_ready = Self::try_unblock_dependents(&execution, &req.task_path);
            } else {
                execution.failure_message = req.failure_message.clone();
                Self::skip_transitive_dependents(&mut execution, &req.task_path);
            }

            let was_executing = execution.status == "EXECUTING";
            Self::check_build_completion(&mut execution);

            let build_just_finished = was_executing && execution.is_terminal();
            let build_outcome_str = if execution.status == "COMPLETED" {
                "SUCCESS".to_string()
            } else {
                "FAILED".to_string()
            };
            let failure_msg = execution.failure_message.clone();

            (true, newly_ready, build_just_finished, build_outcome_str, failure_msg)
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
                let succeeded = execution.total_tasks
                    - execution.failed_count()
                    - execution.skipped_count();
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
            let succeeded = execution.total_tasks
                - execution.failed_count()
                - execution.skipped_count();
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
    use crate::proto::RegisterTaskRequest;

    fn make_svc() -> DagExecutorServiceImpl {
        let task_graph = Arc::new(super::super::task_graph::TaskGraphServiceImpl::new());
        let scheduler = Arc::new(WorkerScheduler::new(4));
        DagExecutorServiceImpl::new(scheduler, task_graph, Vec::new())
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
        roots.sort();
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
        for (path, deps) in &[(":a", vec![":b".to_string()]), (":b", vec![":a".to_string()])] {
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
}
