use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use super::event_dispatcher::EventDispatcher;
use super::scopes::BuildId;

use crate::proto::{
    console_service_server::ConsoleService, LogMessageRequest, LogMessageResponse,
    RequestInputRequest, RequestInputResponse, SetBuildDescriptionRequest,
    SetBuildDescriptionResponse, UpdateProgressRequest, UpdateProgressResponse,
};

/// Maximum buffered log messages per build.
const MAX_LOG_BUFFER: usize = 1000;

/// A tracked progress operation.
#[derive(Clone)]
struct ProgressEntry {
    operation_id: String,
    description: String,
    status: String,
    total_work: i64,
    completed_work: i64,
    start_time_ms: i64,
    end_time_ms: i64,
}

impl ProgressEntry {
    /// Format this entry as a human-readable status line with ANSI colors.
    fn format_status_line(&self) -> String {
        let elapsed = if self.end_time_ms > 0 && self.start_time_ms > 0 {
            self.end_time_ms - self.start_time_ms
        } else if self.start_time_ms > 0 {
            Self::now_ms() - self.start_time_ms
        } else {
            0
        };

        let pct = if self.total_work > 0 {
            self.completed_work as f64 / self.total_work as f64 * 100.0
        } else {
            0.0
        };

        let status_color = ansi::color_for_status(&self.status);
        let reset = ansi::RESET;
        let bold = ansi::BOLD;

        format!(
            "{bold}{}{reset} [{}] {status_color}{}{reset} {}%{}",
            self.operation_id,
            self.description,
            self.status,
            pct as u64,
            if elapsed > 0 {
                format!(" ({}ms)", elapsed)
            } else {
                String::new()
            },
        )
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// A buffered log message for replay.
#[derive(Clone, Debug)]
pub struct BufferedLog {
    build_id: String,
    level: String,
    category: String,
    message: String,
    timestamp_ms: i64,
}

impl BufferedLog {
    /// Format this buffered log message with ANSI colors, including timestamp.
    pub fn formatted(&self) -> String {
        let color = ansi::color_for_level(&self.level);
        let reset = ansi::RESET;
        let bold = ansi::BOLD;
        let dim = ansi::DIM;
        let level_upper = self.level.to_uppercase();
        format!(
            "{dim}{}{reset} {bold}{color}[{level_upper}]{reset} [{}] {}",
            self.timestamp_ms, self.category, self.message
        )
    }

    pub fn build_id(&self) -> &str {
        &self.build_id
    }

    pub fn level(&self) -> &str {
        &self.level
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn timestamp_ms(&self) -> i64 {
        self.timestamp_ms
    }
}

/// ANSI color codes for console output.
mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const RED_BOLD: &str = "\x1b[1;31m";
    pub const GREEN_BOLD: &str = "\x1b[1;32m";
    pub const YELLOW_BOLD: &str = "\x1b[1;33m";

    /// Map a log level to an ANSI color string.
    pub fn color_for_level(level: &str) -> &'static str {
        match level {
            "error" => RED_BOLD,
            "warn" => YELLOW,
            "info" => GREEN,
            "debug" => DIM,
            "lifecycle" => CYAN,
            "quiet" => DIM,
            "progress" => BLUE,
            _ => "",
        }
    }

    /// Map a log level to a bold ANSI color string for headings/summaries.
    pub fn bold_color_for_level(level: &str) -> &'static str {
        match level {
            "error" => RED_BOLD,
            "warn" => YELLOW_BOLD,
            "info" => GREEN_BOLD,
            "debug" => DIM,
            "lifecycle" => CYAN,
            "quiet" => DIM,
            "progress" => BLUE,
            _ => "",
        }
    }

    /// Map a progress status string to an ANSI color.
    /// Uses `RED` for failures and `MAGENTA` for unknown states.
    pub fn color_for_status(status: &str) -> &'static str {
        match status {
            "running" => BLUE,
            "succeeded" | "complete" | "completed" | "up_to_date" => GREEN,
            "failed" => RED,
            "skipped" => DIM,
            _ => MAGENTA,
        }
    }
}

/// Rust-native console/rich output service.
/// Manages console output, progress rendering, status lines, and log buffering.
pub struct ConsoleServiceImpl {
    progress_ops: Arc<DashMap<String, ProgressEntry>>, // operation_id -> entry
    build_descriptions: Arc<DashMap<BuildId, String>>, // build_id -> description
    log_buffer: Arc<DashMap<BuildId, Vec<BufferedLog>>>, // build_id -> [logs]
    log_counts: Arc<AtomicI64>,
    progress_updates: Arc<AtomicI64>,
    logs_evicted: Arc<AtomicI64>,
}

impl Clone for ConsoleServiceImpl {
    fn clone(&self) -> Self {
        Self {
            progress_ops: Arc::clone(&self.progress_ops),
            build_descriptions: Arc::clone(&self.build_descriptions),
            log_buffer: Arc::clone(&self.log_buffer),
            log_counts: Arc::clone(&self.log_counts),
            progress_updates: Arc::clone(&self.progress_updates),
            logs_evicted: Arc::clone(&self.logs_evicted),
        }
    }
}

impl Default for ConsoleServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleServiceImpl {
    pub fn new() -> Self {
        Self {
            progress_ops: Arc::new(DashMap::new()),
            build_descriptions: Arc::new(DashMap::new()),
            log_buffer: Arc::new(DashMap::new()),
            log_counts: Arc::new(AtomicI64::new(0)),
            progress_updates: Arc::new(AtomicI64::new(0)),
            logs_evicted: Arc::new(AtomicI64::new(0)),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// Format a log message with ANSI colors (for structured output).
    pub fn format_log_message(level: &str, category: &str, message: &str) -> String {
        let color = ansi::color_for_level(level);
        let reset = ansi::RESET;
        let level_upper = level.to_uppercase();
        format!("{color}[{level_upper}]{reset} [{category}] {message}")
    }

    /// Format a log message with bold ANSI level tag (for headings/summaries).
    pub fn format_log_message_bold(level: &str, category: &str, message: &str) -> String {
        let color = ansi::bold_color_for_level(level);
        let reset = ansi::RESET;
        let bold = ansi::BOLD;
        let level_upper = level.to_uppercase();
        format!("{bold}{color}[{level_upper}]{reset} [{category}] {message}")
    }

    /// Get all buffered log messages for a build.
    pub fn get_log_buffer(&self, build_id: &BuildId) -> Vec<BufferedLog> {
        self.log_buffer
            .get(build_id)
            .map(|buf| buf.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all buffered log messages for a build, formatted with ANSI colors and timestamps.
    pub fn get_formatted_log_buffer(&self, build_id: &BuildId) -> Vec<String> {
        self.log_buffer
            .get(build_id)
            .map(|buf| buf.iter().map(|log| log.formatted()).collect())
            .unwrap_or_default()
    }

    /// Flush (clear) the log buffer for a build, returning the evicted count.
    pub fn flush_log_buffer(&self, build_id: &BuildId) -> usize {
        if let Some((_, buf)) = self.log_buffer.remove(build_id) {
            buf.len()
        } else {
            0
        }
    }

    /// Get progress percentage for an operation.
    pub fn get_progress_percent(&self, operation_id: &str) -> Option<f64> {
        self.progress_ops.get(operation_id).map(|entry| {
            if entry.total_work > 0 {
                entry.completed_work as f64 / entry.total_work as f64 * 100.0
            } else {
                0.0
            }
        })
    }

    /// Buffer a log message and evict if at capacity.
    pub fn buffer_log(&self, build_id: &BuildId, level: &str, category: &str, message: &str) {
        let build_id_str = build_id.to_string();
        if let Some(mut buf) = self.log_buffer.get_mut(build_id) {
            if buf.len() >= MAX_LOG_BUFFER {
                let evict = buf.len() / 2;
                buf.drain(..evict);
                self.logs_evicted.fetch_add(evict as i64, Ordering::Relaxed);
            }
            buf.push(BufferedLog {
                build_id: build_id_str,
                level: level.to_string(),
                category: category.to_string(),
                message: message.to_string(),
                timestamp_ms: Self::now_ms(),
            });
        } else {
            self.log_buffer
                .entry(build_id.clone())
                .or_default()
                .push(BufferedLog {
                    build_id: build_id.to_string(),
                    level: level.to_string(),
                    category: category.to_string(),
                    message: message.to_string(),
                    timestamp_ms: Self::now_ms(),
                });
        }
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

        let build_id = BuildId::from(req.build_id.clone());

        // Buffer the log message
        self.buffer_log(&build_id, &req.level, &req.category, &req.message);

        // Format and log via tracing; use bold formatting for errors to make them stand out
        let formatted = match req.level.as_str() {
            "error" => Self::format_log_message_bold(&req.level, &req.category, &req.message),
            _ => Self::format_log_message(&req.level, &req.category, &req.message),
        };

        let evicted = self.logs_evicted.load(Ordering::Relaxed);
        let total = self.log_counts.load(Ordering::Relaxed);

        match req.level.as_str() {
            "error" => tracing::error!(
                build_id = %req.build_id,
                category = %req.category,
                log_count = total,
                logs_evicted = evicted,
                "{}",
                formatted
            ),
            "warn" => tracing::warn!(
                build_id = %req.build_id,
                category = %req.category,
                log_count = total,
                logs_evicted = evicted,
                "{}",
                formatted
            ),
            "quiet" => tracing::info!(
                build_id = %req.build_id,
                category = %req.category,
                log_count = total,
                logs_evicted = evicted,
                "{}",
                formatted
            ),
            "lifecycle" => tracing::info!(
                build_id = %req.build_id,
                category = %req.category,
                log_count = total,
                logs_evicted = evicted,
                "{}",
                formatted
            ),
            "progress" => tracing::info!(
                build_id = %req.build_id,
                category = %req.category,
                log_count = total,
                logs_evicted = evicted,
                "{}",
                formatted
            ),
            "debug" => tracing::debug!(
                build_id = %req.build_id,
                category = %req.category,
                log_count = total,
                logs_evicted = evicted,
                "{}",
                formatted
            ),
            _ => tracing::info!(
                build_id = %req.build_id,
                category = %req.category,
                log_count = total,
                logs_evicted = evicted,
                "{}",
                formatted
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
            let op_end = op.end_time_ms;
            let op_status_nonempty = !op.status.is_empty();

            let is_new = !self.progress_ops.contains_key(&op_id);

            self.progress_ops
                .entry(op_id.clone())
                .and_modify(|entry| {
                    entry.description = op_desc.clone();
                    if op_status_nonempty {
                        entry.status = op_status.clone();
                    }
                    entry.total_work = op_total;
                    entry.completed_work = op_completed;
                    if op_start > 0 {
                        entry.start_time_ms = op_start;
                    }
                    if op_end > 0 {
                        entry.end_time_ms = op_end;
                    }
                })
                .or_insert_with(|| ProgressEntry {
                    operation_id: op_id.clone(),
                    description: op_desc,
                    status: op_status,
                    total_work: op_total,
                    completed_work: op_completed,
                    start_time_ms: if op_start > 0 { op_start } else { now },
                    end_time_ms: op_end,
                });

            // Log the formatted status line for this operation
            if let Some(entry) = self.progress_ops.get(&op_id) {
                let status_line = entry.format_status_line();
                if is_new {
                    tracing::info!(
                        build_id = %req.build_id,
                        operation_id = %entry.operation_id,
                        start_time_ms = entry.start_time_ms,
                        "Progress started: {}", status_line
                    );
                } else {
                    tracing::debug!(
                        build_id = %req.build_id,
                        operation_id = %entry.operation_id,
                        start_time_ms = entry.start_time_ms,
                        "Progress updated: {}", status_line
                    );
                }
            }
        }

        Ok(Response::new(UpdateProgressResponse { accepted: true }))
    }

    async fn request_input(
        &self,
        request: Request<RequestInputRequest>,
    ) -> Result<Response<RequestInputResponse>, Status> {
        let req = request.into_inner();

        tracing::warn!(
            build_id = %req.build_id,
            input_id = %req.input_id,
            prompt = %req.prompt,
            default_value = %req.default_value,
            "Input requested in daemon mode -- returning empty value"
        );

        // In daemon mode, input requests are typically not supported.
        Ok(Response::new(RequestInputResponse {
            value: String::new(),
        }))
    }

    async fn set_build_description(
        &self,
        request: Request<SetBuildDescriptionRequest>,
    ) -> Result<Response<SetBuildDescriptionResponse>, Status> {
        let req = request.into_inner();

        let build_id = BuildId::from(req.build_id.clone());

        self.build_descriptions
            .insert(build_id, req.description.clone());

        let total_ops = self.progress_ops.len();
        tracing::info!(
            build_id = %req.build_id,
            description = %req.description,
            active_operations = total_ops,
            "Build description set"
        );

        Ok(Response::new(SetBuildDescriptionResponse {
            accepted: true,
        }))
    }
}

impl EventDispatcher for ConsoleServiceImpl {
    fn dispatch_event(&self, event: &crate::proto::BuildEventMessage) {
        let build_id = BuildId::from(event.build_id.clone());

        match event.event_type.as_str() {
            "task_start" => {
                self.buffer_log(
                    &build_id,
                    "lifecycle",
                    "task",
                    &format!("> {} ...", event.display_name),
                );
            }
            "task_finish" => {
                let outcome = event
                    .properties
                    .get("outcome")
                    .map(|s| s.as_str())
                    .unwrap_or("UNKNOWN");
                let duration = event
                    .properties
                    .get("duration_ms")
                    .map(|s| format!("({}ms)", s))
                    .unwrap_or_default();
                let level = if outcome == "FAILED" {
                    "error"
                } else {
                    "lifecycle"
                };
                self.buffer_log(
                    &build_id,
                    level,
                    "task",
                    &format!("{} {} {}", outcome, event.display_name, duration),
                );
            }
            "build_start" => {
                self.buffer_log(
                    &build_id,
                    "lifecycle",
                    "build",
                    &format!("Build started: {}", event.display_name),
                );
            }
            "build_finish" => {
                let outcome = event
                    .properties
                    .get("outcome")
                    .map(|s| s.as_str())
                    .unwrap_or("SUCCESS");
                let duration = event
                    .properties
                    .get("duration_ms")
                    .map(|s| format!("{}ms", s))
                    .unwrap_or_else(|| "unknown time".to_string());
                let level = if outcome == "FAILED" {
                    "error"
                } else {
                    "lifecycle"
                };
                self.buffer_log(
                    &build_id,
                    level,
                    "build",
                    &format!("Build {} in {}", outcome, duration),
                );
            }
            _ => {}
        }
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
    async fn test_log_buffering() {
        let svc = ConsoleServiceImpl::new();

        for i in 0..5 {
            svc.log_message(Request::new(LogMessageRequest {
                build_id: "build-buf".to_string(),
                level: "info".to_string(),
                category: "test".to_string(),
                message: format!("Message {}", i),
                throwable: String::new(),
            }))
            .await
            .unwrap();
        }

        let buffer = svc.get_log_buffer(&BuildId("build-buf".to_string()));
        assert_eq!(buffer.len(), 5);
        assert_eq!(buffer[0].message, "Message 0");
        assert_eq!(buffer[4].message, "Message 4");
    }

    #[tokio::test]
    async fn test_log_buffer_isolation() {
        let svc = ConsoleServiceImpl::new();

        svc.log_message(Request::new(LogMessageRequest {
            build_id: "build-a".to_string(),
            level: "info".to_string(),
            category: "test".to_string(),
            message: "A".to_string(),
            throwable: String::new(),
        }))
        .await
        .unwrap();

        svc.log_message(Request::new(LogMessageRequest {
            build_id: "build-b".to_string(),
            level: "error".to_string(),
            category: "test".to_string(),
            message: "B".to_string(),
            throwable: String::new(),
        }))
        .await
        .unwrap();

        let buf_a = svc.get_log_buffer(&BuildId("build-a".to_string()));
        let buf_b = svc.get_log_buffer(&BuildId("build-b".to_string()));

        assert_eq!(buf_a.len(), 1);
        assert_eq!(buf_b.len(), 1);
        assert_eq!(buf_a[0].message, "A");
        assert_eq!(buf_b[0].message, "B");
        assert_eq!(buf_b[0].level, "error");
    }

    #[tokio::test]
    async fn test_format_log_message() {
        let formatted =
            ConsoleServiceImpl::format_log_message("error", "org.gradle", "Build failed");
        assert!(formatted.contains("[ERROR]"));
        assert!(formatted.contains("[org.gradle]"));

        let formatted = ConsoleServiceImpl::format_log_message("info", "test", "OK");
        assert!(formatted.contains("[INFO]"));

        let formatted = ConsoleServiceImpl::format_log_message("lifecycle", "core", "Task started");
        assert!(formatted.contains("[LIFECYCLE]"));
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
    async fn test_progress_percent() {
        let svc = ConsoleServiceImpl::new();

        svc.update_progress(Request::new(UpdateProgressRequest {
            build_id: "build-3".to_string(),
            operations: vec![ProgressOperation {
                operation_id: "op-pct".to_string(),
                description: "Test".to_string(),
                status: "running".to_string(),
                total_work: 200,
                completed_work: 50,
                start_time_ms: 0,
                end_time_ms: 0,
                header: String::new(),
            }],
        }))
        .await
        .unwrap();

        let pct = svc.get_progress_percent("op-pct").unwrap();
        assert!((pct - 25.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_progress_percent_no_total() {
        let svc = ConsoleServiceImpl::new();

        svc.update_progress(Request::new(UpdateProgressRequest {
            build_id: "build-4".to_string(),
            operations: vec![ProgressOperation {
                operation_id: "op-nototal".to_string(),
                description: "Test".to_string(),
                status: "running".to_string(),
                total_work: 0,
                completed_work: 50,
                start_time_ms: 0,
                end_time_ms: 0,
                header: String::new(),
            }],
        }))
        .await
        .unwrap();

        let pct = svc.get_progress_percent("op-nototal").unwrap();
        assert!((pct - 0.0).abs() < f64::EPSILON);
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

        let desc = svc
            .build_descriptions
            .get(&BuildId("build-4".to_string()))
            .unwrap();
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
