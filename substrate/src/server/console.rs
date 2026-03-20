use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    console_service_server::ConsoleService, LogMessageRequest, LogMessageResponse,
    RequestInputRequest, RequestInputResponse, SetBuildDescriptionRequest,
    SetBuildDescriptionResponse, UpdateProgressRequest, UpdateProgressResponse,
};

/// A tracked progress operation.
struct ProgressEntry {
    operation_id: String,
    description: String,
    status: String,
    total_work: i64,
    completed_work: i64,
    start_time_ms: i64,
}

/// Rust-native console/rich output service.
/// Manages console output, progress rendering, and status lines.
pub struct ConsoleServiceImpl {
    progress_ops: DashMap<String, ProgressEntry>, // operation_id -> entry
    build_descriptions: DashMap<String, String>,  // build_id -> description
    log_counts: AtomicI64,
    progress_updates: AtomicI64,
}

impl ConsoleServiceImpl {
    pub fn new() -> Self {
        Self {
            progress_ops: DashMap::new(),
            build_descriptions: DashMap::new(),
            log_counts: AtomicI64::new(0),
            progress_updates: AtomicI64::new(0),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

#[tonic::async_trait]
impl ConsoleService for ConsoleServiceImpl {
    async fn log_message(
        &self,
        request: Request<LogMessageRequest>,
    ) -> Result<Response<LogMessageResponse>, Status> {
        let req = request.into_inner();
        self.log_counts.fetch_add(1, Ordering::Relaxed);

        // In production, this would write to the console with appropriate
        // formatting, colors, and ANSI codes. For now, we use tracing.
        match req.level.as_str() {
            "error" => tracing::error!(
                build_id = %req.build_id,
                category = %req.category,
                "{}",
                req.message
            ),
            "warn" => tracing::warn!(
                build_id = %req.build_id,
                category = %req.category,
                "{}",
                req.message
            ),
            "info" => tracing::info!(
                build_id = %req.build_id,
                category = %req.category,
                "{}",
                req.message
            ),
            "debug" => tracing::debug!(
                build_id = %req.build_id,
                category = %req.category,
                "{}",
                req.message
            ),
            "lifecycle" | _ => tracing::info!(
                build_id = %req.build_id,
                category = %req.category,
                "[{}] {}",
                req.level,
                req.message
            ),
        }

        Ok(Response::new(LogMessageResponse { accepted: true }))
    }

    async fn update_progress(
        &self,
        request: Request<UpdateProgressRequest>,
    ) -> Result<Response<UpdateProgressResponse>, Status> {
        let req = request.into_inner();
        self.progress_updates.fetch_add(1, Ordering::Relaxed);

        let now = Self::now_ms();

        for op in req.operations {
            let op_id = op.operation_id.clone();
            let op_desc = op.description.clone();
            let op_status = op.status.clone();
            let op_total = op.total_work;
            let op_completed = op.completed_work;
            let op_start = op.start_time_ms;
            let op_status_nonempty = !op.status.is_empty();

            self.progress_ops
                .entry(op_id.clone())
                .and_modify(|entry| {
                    entry.description = op_desc.clone();
                    if op_status_nonempty {
                        entry.status = op_status.clone();
                    }
                    entry.total_work = op_total;
                    entry.completed_work = op_completed;
                })
                .or_insert_with(|| ProgressEntry {
                    operation_id: op_id,
                    description: op_desc,
                    status: op_status,
                    total_work: op_total,
                    completed_work: op_completed,
                    start_time_ms: if op_start > 0 { op_start } else { now },
                });
        }

        Ok(Response::new(UpdateProgressResponse { accepted: true }))
    }

    async fn request_input(
        &self,
        request: Request<RequestInputRequest>,
    ) -> Result<Response<RequestInputResponse>, Status> {
        let _req = request.into_inner();

        // In production, this would read from stdin.
        // For daemon mode, input requests are typically not supported.
        Ok(Response::new(RequestInputResponse {
            value: String::new(),
        }))
    }

    async fn set_build_description(
        &self,
        request: Request<SetBuildDescriptionRequest>,
    ) -> Result<Response<SetBuildDescriptionResponse>, Status> {
        let req = request.into_inner();

        self.build_descriptions
            .insert(req.build_id, req.description);

        Ok(Response::new(SetBuildDescriptionResponse { accepted: true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ProgressOperation;

    #[tokio::test]
    async fn test_log_message() {
        let svc = ConsoleServiceImpl::new();

        let resp = svc
            .log_message(Request::new(LogMessageRequest {
                build_id: "build-1".to_string(),
                level: "lifecycle".to_string(),
                category: "org.gradle.api".to_string(),
                message: "Hello, Gradle!".to_string(),
                throwable: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.accepted);
    }

    #[tokio::test]
    async fn test_update_progress() {
        let svc = ConsoleServiceImpl::new();

        svc.update_progress(Request::new(UpdateProgressRequest {
            build_id: "build-2".to_string(),
            operations: vec![ProgressOperation {
                operation_id: "op-1".to_string(),
                description: "Compiling Java sources".to_string(),
                status: "running".to_string(),
                total_work: 100,
                completed_work: 25,
                start_time_ms: 1000,
                end_time_ms: 0,
                header: ":compileJava".to_string(),
            }],
        }))
        .await
        .unwrap();

        // Update progress
        svc.update_progress(Request::new(UpdateProgressRequest {
            build_id: "build-2".to_string(),
            operations: vec![ProgressOperation {
                operation_id: "op-1".to_string(),
                description: "Compiling Java sources".to_string(),
                status: "running".to_string(),
                total_work: 100,
                completed_work: 75,
                start_time_ms: 0,
                end_time_ms: 0,
                header: ":compileJava".to_string(),
            }],
        }))
        .await
        .unwrap();

        let entry = svc.progress_ops.get("op-1").unwrap();
        assert_eq!(entry.completed_work, 75);
    }

    #[tokio::test]
    async fn test_multiple_operations() {
        let svc = ConsoleServiceImpl::new();

        svc.update_progress(Request::new(UpdateProgressRequest {
            build_id: "build-3".to_string(),
            operations: vec![
                ProgressOperation {
                    operation_id: "op-a".to_string(),
                    description: "Compiling".to_string(),
                    status: "running".to_string(),
                    total_work: 10,
                    completed_work: 5,
                    start_time_ms: 0,
                    end_time_ms: 0,
                    header: String::new(),
                },
                ProgressOperation {
                    operation_id: "op-b".to_string(),
                    description: "Testing".to_string(),
                    status: "running".to_string(),
                    total_work: 20,
                    completed_work: 0,
                    start_time_ms: 0,
                    end_time_ms: 0,
                    header: String::new(),
                },
            ],
        }))
        .await
        .unwrap();

        assert_eq!(svc.progress_ops.len(), 2);
    }

    #[tokio::test]
    async fn test_build_description() {
        let svc = ConsoleServiceImpl::new();

        svc.set_build_description(Request::new(SetBuildDescriptionRequest {
            build_id: "build-4".to_string(),
            description: "Building my-app (42 tasks)".to_string(),
        }))
        .await
        .unwrap();

        let desc = svc.build_descriptions.get("build-4").unwrap();
        assert_eq!(*desc, "Building my-app (42 tasks)");
    }

    #[tokio::test]
    async fn test_request_input() {
        let svc = ConsoleServiceImpl::new();

        let resp = svc
            .request_input(Request::new(RequestInputRequest {
                build_id: "build-5".to_string(),
                prompt: "Continue? [y,n]".to_string(),
                default_value: "y".to_string(),
                input_id: "input-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        // In daemon mode, returns empty
        assert!(resp.value.is_empty());
    }
}
