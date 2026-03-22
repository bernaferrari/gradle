use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};

use dashmap::DashMap;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

use crate::proto::{
    build_operations_service_server::BuildOperationsService, BuildEvent,
    BuildSummary, CompleteOperationRequest,
    CompleteOperationResponse, GetBuildSummaryRequest, GetBuildSummaryResponse,
    ReportProgressRequest, ReportProgressResponse, StartOperationRequest,
    StartOperationResponse, StreamEventsRequest,
};

/// Active build operation.
struct ActiveOperation {
    display_name: String,
    operation_type: String,
    #[allow(dead_code)]
    parent_id: String,
    start_time_ms: i64,
    #[allow(dead_code)]
    metadata: Vec<(String, String)>,
    progress: f32,
}

/// Completed operation record for summary.
struct CompletedOperation {
    #[allow(dead_code)]
    display_name: String,
    #[allow(dead_code)]
    operation_type: String,
    #[allow(dead_code)]
    duration_ms: i64,
    #[allow(dead_code)]
    success: bool,
}

/// Rust-native build operations service.
/// Streams build events and manages build lifecycle.
pub struct BuildOperationsServiceImpl {
    operations: DashMap<String, ActiveOperation>,
    completed: DashMap<String, CompletedOperation>,
    build_events_tx: tokio::sync::Mutex<mpsc::Sender<BuildEvent>>,
    build_events_rx: tokio::sync::Mutex<Option<mpsc::Receiver<BuildEvent>>>,
    build_start_ms: AtomicI64,
    total_tasks: AtomicI32,
    executed_tasks: AtomicI32,
    up_to_date_tasks: AtomicI32,
    from_cache_tasks: AtomicI32,
    failed_tasks: AtomicI32,
    total_operations: AtomicI32,
    total_operation_duration_ms: AtomicI64,
}

impl Default for BuildOperationsServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildOperationsServiceImpl {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1000);
        Self {
            operations: DashMap::new(),
            completed: DashMap::new(),
            build_events_tx: tokio::sync::Mutex::new(tx),
            build_events_rx: tokio::sync::Mutex::new(Some(rx)),
            build_start_ms: AtomicI64::new(0),
            total_tasks: AtomicI32::new(0),
            executed_tasks: AtomicI32::new(0),
            up_to_date_tasks: AtomicI32::new(0),
            from_cache_tasks: AtomicI32::new(0),
            failed_tasks: AtomicI32::new(0),
            total_operations: AtomicI32::new(0),
            total_operation_duration_ms: AtomicI64::new(0),
        }
    }

    fn emit_event(&self, event: BuildEvent) {
        if let Ok(tx) = self.build_events_tx.try_lock() {
            if let Err(e) = tx.try_send(event) {
                tracing::warn!("Failed to emit build event: {}", e);
            }
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    #[allow(dead_code)]
    fn record_task_outcome(&self, outcome: &str) {
        self.total_tasks.fetch_add(1, Ordering::Relaxed);
        match outcome {
            "EXECUTED" | "EXECUTED_INCREMENTALLY" | "EXECUTED_NON_INCREMENTALLY" => {
                self.executed_tasks.fetch_add(1, Ordering::Relaxed);
            }
            "UP_TO_DATE" => {
                self.up_to_date_tasks.fetch_add(1, Ordering::Relaxed);
            }
            "FROM_CACHE" => {
                self.from_cache_tasks.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                if outcome.contains("FAIL") {
                    self.failed_tasks.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

#[tonic::async_trait]
impl BuildOperationsService for BuildOperationsServiceImpl {
    type StreamEventsStream = std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<BuildEvent, Status>> + Send>>;

    async fn start_operation(
        &self,
        request: Request<StartOperationRequest>,
    ) -> Result<Response<StartOperationResponse>, Status> {
        let req = request.into_inner();
        let display_name = req.display_name.clone();

        // Track build start on first operation
        self.build_start_ms.compare_exchange(
            0,
            Self::now_ms(),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ).ok();

        self.operations.insert(
            req.operation_id.clone(),
            ActiveOperation {
                display_name: req.display_name,
                operation_type: req.operation_type,
                parent_id: req.parent_id,
                start_time_ms: req.start_time_ms,
                metadata: req.metadata.into_iter().collect(),
                progress: 0.0,
            },
        );

        self.emit_event(BuildEvent {
            timestamp_ms: Self::now_ms(),
            event_type: "START".to_string(),
            operation_id: req.operation_id.clone(),
            display_name: display_name.clone(),
            message: format!("Started {}", display_name),
            progress: 0.0,
            success: true,
        });

        Ok(Response::new(StartOperationResponse { success: true }))
    }

    async fn complete_operation(
        &self,
        request: Request<CompleteOperationRequest>,
    ) -> Result<Response<CompleteOperationResponse>, Status> {
        let req = request.into_inner();

        let (display_name, op_type, start_ms) = self
            .operations
            .remove(&req.operation_id)
            .map(|(_key, op)| (op.display_name, op.operation_type, op.start_time_ms))
            .unwrap_or_default();

        // Record completed operation
        let duration = Self::now_ms() - start_ms;
        self.completed.insert(
            req.operation_id.clone(),
            CompletedOperation {
                display_name: display_name.clone(),
                operation_type: op_type,
                duration_ms: duration,
                success: req.success,
            },
        );

        self.total_operations.fetch_add(1, Ordering::Relaxed);
        self.total_operation_duration_ms.fetch_add(duration, Ordering::Relaxed);

        // Record task outcome if the operation type suggests a task
        if !req.operation_id.is_empty() {
            self.record_task_outcome(&req.outcome);
        }

        self.emit_event(BuildEvent {
            timestamp_ms: Self::now_ms(),
            event_type: "FINISH".to_string(),
            operation_id: req.operation_id,
            display_name,
            message: if req.success {
                "Completed".to_string()
            } else {
                format!("Failed: {}", req.outcome)
            },
            progress: 1.0,
            success: req.success,
        });

        Ok(Response::new(CompleteOperationResponse { success: true }))
    }

    async fn report_progress(
        &self,
        request: Request<ReportProgressRequest>,
    ) -> Result<Response<ReportProgressResponse>, Status> {
        let req = request.into_inner();

        if let Some(mut op) = self.operations.get_mut(&req.operation_id) {
            op.progress = req.progress;
        }

        self.emit_event(BuildEvent {
            timestamp_ms: Self::now_ms(),
            event_type: "PROGRESS".to_string(),
            operation_id: req.operation_id.clone(),
            display_name: String::new(),
            message: req.message,
            progress: req.progress,
            success: true,
        });

        Ok(Response::new(ReportProgressResponse { acknowledged: true }))
    }

    async fn stream_events(
        &self,
        request: Request<StreamEventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let _req = request.into_inner();

        let rx = self.build_events_rx.lock().await.take().ok_or_else(|| {
            Status::resource_exhausted("Event stream already consumed")
        })?;

        let stream = ReceiverStream::new(rx).map(Ok);
        Ok(Response::new(Box::pin(stream) as Self::StreamEventsStream))
    }

    async fn get_build_summary(
        &self,
        request: Request<GetBuildSummaryRequest>,
    ) -> Result<Response<GetBuildSummaryResponse>, Status> {
        let _req = request.into_inner();

        let now = Self::now_ms();
        let start = self.build_start_ms.load(Ordering::Relaxed);
        let duration = if start > 0 { now - start } else { 0 };

        let total = self.total_tasks.load(Ordering::Relaxed);
        let has_failures = self.failed_tasks.load(Ordering::Relaxed) > 0;

        Ok(Response::new(GetBuildSummaryResponse {
            summary: Some(BuildSummary {
                build_id: String::new(),
                total_duration_ms: duration,
                total_tasks: total,
                executed_tasks: self.executed_tasks.load(Ordering::Relaxed),
                up_to_date_tasks: self.up_to_date_tasks.load(Ordering::Relaxed),
                from_cache_tasks: self.from_cache_tasks.load(Ordering::Relaxed),
                failed_tasks: self.failed_tasks.load(Ordering::Relaxed),
                outcome: if has_failures { "FAILURE" } else { "SUCCESS" }.to_string(),
            }),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_start_and_complete() {
        let svc = BuildOperationsServiceImpl::new();

        svc.start_operation(Request::new(StartOperationRequest {
            operation_id: "op-1".to_string(),
            display_name: ":compileJava".to_string(),
            operation_type: "Task".to_string(),
            parent_id: String::new(),
            start_time_ms: 100,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

        assert!(svc.operations.contains_key("op-1"));

        svc.complete_operation(Request::new(CompleteOperationRequest {
            operation_id: "op-1".to_string(),
            duration_ms: 500,
            success: true,
            outcome: "EXECUTED".to_string(),
        }))
        .await
        .unwrap();

        assert!(!svc.operations.contains_key("op-1"));
    }

    #[tokio::test]
    async fn test_progress() {
        let svc = BuildOperationsServiceImpl::new();

        svc.start_operation(Request::new(StartOperationRequest {
            operation_id: "op-1".to_string(),
            display_name: "test".to_string(),
            operation_type: "Test".to_string(),
            parent_id: String::new(),
            start_time_ms: 0,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

        svc.report_progress(Request::new(ReportProgressRequest {
            operation_id: "op-1".to_string(),
            message: "Compiling...".to_string(),
            progress: 0.5,
            elapsed_ms: 250,
        }))
        .await
        .unwrap();

        let op = svc.operations.get("op-1").unwrap();
        assert!((op.progress - 0.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_build_summary_tracks_outcomes() {
        let svc = BuildOperationsServiceImpl::new();

        // Simulate task executions
        for (id, outcome) in [
            ("t1", "EXECUTED"),
            ("t2", "UP_TO_DATE"),
            ("t3", "FROM_CACHE"),
            ("t4", "EXECUTED_INCREMENTALLY"),
            ("t5", "FAILED"),
        ] {
            svc.start_operation(Request::new(StartOperationRequest {
                operation_id: id.to_string(),
                display_name: id.to_string(),
                operation_type: "Task".to_string(),
                parent_id: String::new(),
                start_time_ms: 0,
                metadata: Default::default(),
            }))
            .await
            .unwrap();

            svc.complete_operation(Request::new(CompleteOperationRequest {
                operation_id: id.to_string(),
                duration_ms: 100,
                success: outcome != "FAILED",
                outcome: outcome.to_string(),
            }))
            .await
            .unwrap();
        }

        let summary = svc
            .get_build_summary(Request::new(GetBuildSummaryRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert!(summary.summary.is_some());
        let s = summary.summary.unwrap();
        assert_eq!(s.total_tasks, 5);
        assert_eq!(s.executed_tasks, 2); // EXECUTED + EXECUTED_INCREMENTALLY
        assert_eq!(s.up_to_date_tasks, 1);
        assert_eq!(s.from_cache_tasks, 1);
        assert_eq!(s.failed_tasks, 1);
        assert_eq!(s.outcome, "FAILURE");
    }

    #[tokio::test]
    async fn test_build_summary_success_when_no_failures() {
        let svc = BuildOperationsServiceImpl::new();

        svc.start_operation(Request::new(StartOperationRequest {
            operation_id: "t1".to_string(),
            display_name: "t1".to_string(),
            operation_type: "Task".to_string(),
            parent_id: String::new(),
            start_time_ms: 0,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

        svc.complete_operation(Request::new(CompleteOperationRequest {
            operation_id: "t1".to_string(),
            duration_ms: 100,
            success: true,
            outcome: "EXECUTED".to_string(),
        }))
        .await
        .unwrap();

        let summary = svc
            .get_build_summary(Request::new(GetBuildSummaryRequest {}))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(summary.summary.unwrap().outcome, "SUCCESS");
    }

    #[tokio::test]
    async fn test_completed_operations_tracked() {
        let svc = BuildOperationsServiceImpl::new();

        svc.start_operation(Request::new(StartOperationRequest {
            operation_id: "op-1".to_string(),
            display_name: "op1".to_string(),
            operation_type: "T".to_string(),
            parent_id: String::new(),
            start_time_ms: 0,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

        svc.complete_operation(Request::new(CompleteOperationRequest {
            operation_id: "op-1".to_string(),
            duration_ms: 200,
            success: true,
            outcome: "EXECUTED".to_string(),
        }))
        .await
        .unwrap();

        assert_eq!(svc.total_operations.load(Ordering::Relaxed), 1);
        assert_eq!(svc.completed.len(), 1);
    }

    #[tokio::test]
    async fn test_complete_nonexistent_operation() {
        let svc = BuildOperationsServiceImpl::new();

        // Completing an operation that was never started should succeed
        let resp = svc
            .complete_operation(Request::new(CompleteOperationRequest {
                operation_id: "nonexistent-op".to_string(),
                duration_ms: 100,
                success: true,
                outcome: "SUCCESS".to_string(),
            }))
            .await
        .unwrap()
        .into_inner();

        assert!(resp.success);
    }

    #[tokio::test]
    async fn test_progress_nonexistent_operation() {
        let svc = BuildOperationsServiceImpl::new();

        // Reporting progress on nonexistent operation should succeed silently
        let resp = svc
            .report_progress(Request::new(ReportProgressRequest {
                operation_id: "nonexistent-op".to_string(),
                message: "doing stuff".to_string(),
                progress: 0.5,
                elapsed_ms: 100,
            }))
            .await
        .unwrap()
        .into_inner();

        assert!(resp.acknowledged);
    }

    #[tokio::test]
    async fn test_build_summary_initial_state() {
        let svc = BuildOperationsServiceImpl::new();

        let summary = svc
            .get_build_summary(Request::new(GetBuildSummaryRequest {}))
            .await
            .unwrap()
            .into_inner();

        let s = summary.summary.unwrap();
        assert_eq!(s.total_tasks, 0);
        assert_eq!(s.executed_tasks, 0);
        assert_eq!(s.outcome, "SUCCESS"); // no failures = success
    }

    #[tokio::test]
    async fn test_multiple_concurrent_operations() {
        let svc = BuildOperationsServiceImpl::new();

        // Start multiple operations
        for i in 0..5 {
            svc.start_operation(Request::new(StartOperationRequest {
                operation_id: format!("op-{}", i),
                display_name: format!("Task {}", i),
                operation_type: "Task".to_string(),
                parent_id: String::new(),
                start_time_ms: 0,
                metadata: Default::default(),
            }))
            .await
            .unwrap();
        }

        assert_eq!(svc.operations.len(), 5);

        // Complete them in different order
        for i in [2, 0, 4, 1, 3] {
            svc.complete_operation(Request::new(CompleteOperationRequest {
                operation_id: format!("op-{}", i),
                duration_ms: 100,
                success: true,
                outcome: "EXECUTED".to_string(),
            }))
            .await
            .unwrap();
        }

        assert_eq!(svc.operations.len(), 0);
        assert_eq!(svc.completed.len(), 5);
        assert_eq!(svc.total_operations.load(Ordering::Relaxed), 5);
    }

    #[tokio::test]
    async fn test_start_operation_with_parent_preserves_relationship() {
        let svc = BuildOperationsServiceImpl::new();

        // Start a parent operation
        svc.start_operation(Request::new(StartOperationRequest {
            operation_id: "parent-op".to_string(),
            display_name: ":build".to_string(),
            operation_type: "Lifecycle".to_string(),
            parent_id: String::new(),
            start_time_ms: 100,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

        // Start a child operation referencing the parent
        svc.start_operation(Request::new(StartOperationRequest {
            operation_id: "child-op".to_string(),
            display_name: ":compileJava".to_string(),
            operation_type: "Task".to_string(),
            parent_id: "parent-op".to_string(),
            start_time_ms: 200,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

        // Verify parent has no parent_id
        let parent = svc.operations.get("parent-op").unwrap();
        assert_eq!(parent.parent_id, "");
        assert_eq!(parent.display_name, ":build");

        // Verify child references the parent
        let child = svc.operations.get("child-op").unwrap();
        assert_eq!(child.parent_id, "parent-op");
        assert_eq!(child.display_name, ":compileJava");
        assert_eq!(child.operation_type, "Task");

        // Both are tracked independently in the active map
        assert_eq!(svc.operations.len(), 2);
    }

    #[tokio::test]
    async fn test_complete_operation_with_failure_records_failure() {
        let svc = BuildOperationsServiceImpl::new();

        svc.start_operation(Request::new(StartOperationRequest {
            operation_id: "failing-op".to_string(),
            display_name: ":compileBadCode".to_string(),
            operation_type: "Task".to_string(),
            parent_id: String::new(),
            start_time_ms: 0,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

        svc.complete_operation(Request::new(CompleteOperationRequest {
            operation_id: "failing-op".to_string(),
            duration_ms: 300,
            success: false,
            outcome: "FAILED".to_string(),
        }))
        .await
        .unwrap();

        // Verify the completed record marks it as failed
        let completed = svc.completed.get("failing-op").unwrap();
        assert!(!completed.success);
        assert_eq!(completed.display_name, ":compileBadCode");

        // Verify build summary reflects the failure
        let summary = svc
            .get_build_summary(Request::new(GetBuildSummaryRequest {}))
            .await
            .unwrap()
            .into_inner();
        let s = summary.summary.unwrap();
        assert_eq!(s.failed_tasks, 1);
        assert_eq!(s.outcome, "FAILURE");
    }

    #[tokio::test]
    async fn test_get_summary_for_nonexistent_operation_returns_empty() {
        let svc = BuildOperationsServiceImpl::new();

        // With no operations started at all, the summary should have defaults
        let summary = svc
            .get_build_summary(Request::new(GetBuildSummaryRequest {}))
            .await
            .unwrap()
            .into_inner();

        let s = summary.summary.unwrap();
        assert_eq!(s.build_id, "");
        assert_eq!(s.total_tasks, 0);
        assert_eq!(s.executed_tasks, 0);
        assert_eq!(s.up_to_date_tasks, 0);
        assert_eq!(s.from_cache_tasks, 0);
        assert_eq!(s.failed_tasks, 0);
        assert_eq!(s.outcome, "SUCCESS");
        assert_eq!(s.total_duration_ms, 0);

        // Also verify no completed operations exist for a made-up ID
        assert!(svc.completed.get("no-such-op").is_none());
        assert!(!svc.operations.contains_key("no-such-op"));
    }

    #[tokio::test]
    async fn test_same_name_different_ids_tracked_independently() {
        let svc = BuildOperationsServiceImpl::new();

        // Start two operations with the same display_name but different IDs
        svc.start_operation(Request::new(StartOperationRequest {
            operation_id: "task-run-1".to_string(),
            display_name: ":test".to_string(),
            operation_type: "Task".to_string(),
            parent_id: String::new(),
            start_time_ms: 100,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

        svc.start_operation(Request::new(StartOperationRequest {
            operation_id: "task-run-2".to_string(),
            display_name: ":test".to_string(),
            operation_type: "Task".to_string(),
            parent_id: String::new(),
            start_time_ms: 200,
            metadata: Default::default(),
        }))
        .await
        .unwrap();

        assert_eq!(svc.operations.len(), 2);

        // Complete the first with success, the second with failure
        svc.complete_operation(Request::new(CompleteOperationRequest {
            operation_id: "task-run-1".to_string(),
            duration_ms: 100,
            success: true,
            outcome: "EXECUTED".to_string(),
        }))
        .await
        .unwrap();

        svc.complete_operation(Request::new(CompleteOperationRequest {
            operation_id: "task-run-2".to_string(),
            duration_ms: 50,
            success: false,
            outcome: "FAILED".to_string(),
        }))
        .await
        .unwrap();

        // Both should be independently recorded in completed
        assert_eq!(svc.completed.len(), 2);

        let c1 = svc.completed.get("task-run-1").unwrap();
        assert!(c1.success);

        let c2 = svc.completed.get("task-run-2").unwrap();
        assert!(!c2.success);

        // Both should have the same display name but are separate entries
        assert_eq!(c1.display_name, c2.display_name);
    }
}
