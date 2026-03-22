use dashmap::DashMap;
use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    work_service_server::WorkService, WorkEvaluateRequest, WorkEvaluateResponse, WorkRecordRequest,
    WorkRecordResponse,
};

/// Tracks execution history for work items to enable up-to-date detection.
#[derive(Default)]
pub struct WorkHistory {
    entries: DashMap<String, WorkHistoryEntry>,
}

struct WorkHistoryEntry {
    input_hash: String,
    success: bool,
    duration_ms: i64,
}

/// Worker scheduling state.
pub struct WorkerScheduler {
    history: WorkHistory,
    running: DashMap<String, i64>,
    max_concurrent: usize,
}

impl WorkerScheduler {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            history: WorkHistory::default(),
            running: DashMap::new(),
            max_concurrent,
        }
    }

    pub fn running_count(&self) -> usize {
        self.running.len()
    }

    pub fn can_accept_work(&self) -> bool {
        self.running.len() < self.max_concurrent
    }

    pub fn start_work(&self, task_path: String, start_time_ms: i64) -> bool {
        if !self.can_accept_work() {
            return false;
        }
        self.running.insert(task_path, start_time_ms);
        true
    }

    pub fn complete_work(&self, task_path: &str) {
        self.running.remove(task_path);
    }
}

pub struct WorkServiceImpl {
    scheduler: std::sync::Arc<WorkerScheduler>,
}

impl Default for WorkServiceImpl {
    fn default() -> Self {
        Self::new(std::sync::Arc::new(WorkerScheduler::new(16)))
    }
}

impl WorkServiceImpl {
    pub fn new(scheduler: std::sync::Arc<WorkerScheduler>) -> Self {
        Self { scheduler }
    }
}

#[tonic::async_trait]
impl WorkService for WorkServiceImpl {
    async fn evaluate(
        &self,
        request: Request<WorkEvaluateRequest>,
    ) -> Result<Response<WorkEvaluateResponse>, Status> {
        let req = request.into_inner();

        // Compute an input hash from the input properties
        let mut input_parts: Vec<&String> = req.input_properties.values().collect();
        input_parts.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let input_str = input_parts.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("|");
        let mut hasher = Md5::new();
        hasher.update(input_str.as_bytes());
        let input_hash = format!("{:x}", hasher.finalize());

        let should_execute;
        let reason;

        if let Some(entry) = self.scheduler.history.entries.get(&req.task_path) {
            if entry.input_hash == input_hash {
                should_execute = false;
                reason = format!(
                    "UP_TO_DATE: inputs unchanged (previous: {}ms, {})",
                    entry.duration_ms,
                    if entry.success { "succeeded" } else { "failed" }
                );
            } else {
                should_execute = true;
                reason = format!("EXECUTE: inputs changed (hash {} -> {})", entry.input_hash, input_hash);
            }
        } else {
            should_execute = true;
            reason = "EXECUTE: no previous execution record".to_string();
        }

        tracing::debug!(task = %req.task_path, should_execute, %reason, "Work evaluation");

        Ok(Response::new(WorkEvaluateResponse {
            should_execute,
            reason,
            input_hash,
        }))
    }

    async fn record_execution(
        &self,
        request: Request<WorkRecordRequest>,
    ) -> Result<Response<WorkRecordResponse>, Status> {
        let req = request.into_inner();

        self.scheduler.history.entries.insert(
            req.task_path.clone(),
            WorkHistoryEntry {
                input_hash: req.input_hash,
                success: req.success,
                duration_ms: req.duration_ms,
            },
        );

        self.scheduler.complete_work(&req.task_path);

        tracing::debug!(
            task = %req.task_path,
            duration_ms = req.duration_ms,
            success = req.success,
            "Recorded execution"
        );

        Ok(Response::new(WorkRecordResponse {
            acknowledged: true,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_evaluate_no_history() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        let resp = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":compileJava".to_string(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.should_execute);
        assert!(resp.reason.contains("no previous"));
    }

    #[tokio::test]
    async fn test_evaluate_up_to_date() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler.clone());

        // Compute the same hash that the evaluate function will compute
        let mut hasher = Md5::new();
        hasher.update(b"|"); // sorted single key "|" joined with "|"
        let input_hash = format!("{:x}", hasher.finalize());

        scheduler.history.entries.insert(
            ":compileJava".to_string(),
            WorkHistoryEntry {
                input_hash,
                success: true,
                duration_ms: 500,
            },
        );

        let mut props = std::collections::HashMap::new();
        props.insert("key".to_string(), "|".to_string());

        let resp = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":compileJava".to_string(),
                input_properties: props,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.should_execute);
        assert!(resp.reason.contains("UP_TO_DATE"));
    }

    #[test]
    fn test_scheduler_concurrency() {
        let scheduler = WorkerScheduler::new(2);

        assert!(scheduler.can_accept_work());
        assert!(scheduler.start_work(":task1".to_string(), 0));
        assert!(scheduler.start_work(":task2".to_string(), 0));
        assert!(!scheduler.can_accept_work());
        assert!(!scheduler.start_work(":task3".to_string(), 0));

        scheduler.complete_work(":task1");
        assert!(scheduler.can_accept_work());
        assert_eq!(scheduler.running_count(), 1);
    }

    #[tokio::test]
    async fn test_record_execution_then_evaluate() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        // Evaluate with no history → must execute
        let resp1 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":compileJava".to_string(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp1.should_execute);
        let input_hash = resp1.input_hash;

        // Record execution
        let rec = svc
            .record_execution(Request::new(WorkRecordRequest {
                task_path: ":compileJava".to_string(),
                duration_ms: 500,
                success: true,
                input_hash: input_hash.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(rec.acknowledged);

        // Evaluate again with same inputs → up-to-date
        let resp2 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":compileJava".to_string(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp2.should_execute);
        assert!(resp2.reason.contains("UP_TO_DATE"));
    }

    #[tokio::test]
    async fn test_evaluate_inputs_changed() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        // Record execution with empty props
        let resp1 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":compileJava".to_string(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        svc.record_execution(Request::new(WorkRecordRequest {
            task_path: ":compileJava".to_string(),
            duration_ms: 100,
            success: true,
            input_hash: resp1.input_hash,
        }))
        .await
        .unwrap();

        // Now evaluate with different inputs → must execute
        let mut props = std::collections::HashMap::new();
        props.insert("source".to_string(), "v2".to_string());

        let resp2 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":compileJava".to_string(),
                input_properties: props,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp2.should_execute);
        assert!(resp2.reason.contains("inputs changed"));
    }
}
