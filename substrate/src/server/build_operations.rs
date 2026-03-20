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
    parent_id: String,
    start_time_ms: i64,
    metadata: Vec<(String, String)>,
    progress: f32,
}

/// Rust-native build operations service.
/// Streams build events and manages build lifecycle.
pub struct BuildOperationsServiceImpl {
    operations: DashMap<String, ActiveOperation>,
    build_events_tx: tokio::sync::Mutex<mpsc::Sender<BuildEvent>>,
    build_events_rx: tokio::sync::Mutex<Option<mpsc::Receiver<BuildEvent>>>,
    build_start_ms: AtomicI64,
    total_tasks: AtomicI32,
    executed_tasks: AtomicI32,
    up_to_date_tasks: AtomicI32,
    from_cache_tasks: AtomicI32,
    failed_tasks: AtomicI32,
}

impl BuildOperationsServiceImpl {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1000);
        Self {
            operations: DashMap::new(),
            build_events_tx: tokio::sync::Mutex::new(tx),
            build_events_rx: tokio::sync::Mutex::new(Some(rx)),
            build_start_ms: AtomicI64::new(0),
            total_tasks: AtomicI32::new(0),
            executed_tasks: AtomicI32::new(0),
            up_to_date_tasks: AtomicI32::new(0),
            from_cache_tasks: AtomicI32::new(0),
            failed_tasks: AtomicI32::new(0),
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

        let display_name = self
            .operations
            .remove(&req.operation_id)
            .map(|(_key, op)| op.display_name)
            .unwrap_or_default();

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
}
