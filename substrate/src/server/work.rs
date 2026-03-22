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

    #[tokio::test]
    async fn test_evaluate_different_tasks_independent() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        // Record execution for task A
        let resp_a = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":taskA".to_string(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        svc.record_execution(Request::new(WorkRecordRequest {
            task_path: ":taskA".to_string(),
            duration_ms: 100,
            success: true,
            input_hash: resp_a.input_hash,
        }))
        .await
        .unwrap();

        // Task B should still need execution (different task path)
        let resp_b = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":taskB".to_string(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp_b.should_execute);
        assert!(resp_b.reason.contains("no previous"));
    }

    #[tokio::test]
    async fn test_record_overwrites_previous() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        // First evaluation + record
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
            input_hash: resp1.input_hash.clone(),
        }))
        .await
        .unwrap();

        // Record again with same hash but different duration
        svc.record_execution(Request::new(WorkRecordRequest {
            task_path: ":compileJava".to_string(),
            duration_ms: 999,
            success: true,
            input_hash: resp1.input_hash,
        }))
        .await
        .unwrap();

        // Should still be up-to-date but show the new duration
        let resp2 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":compileJava".to_string(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp2.should_execute);
        assert!(resp2.reason.contains("999ms"));
    }

    #[tokio::test]
    async fn test_evaluate_input_hash_deterministic() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        let mut props = std::collections::HashMap::new();
        props.insert("b".to_string(), "2".to_string());
        props.insert("a".to_string(), "1".to_string());

        let resp1 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":test".to_string(),
                input_properties: props.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        // Same properties in different insertion order should produce same hash
        let mut props2 = std::collections::HashMap::new();
        props2.insert("a".to_string(), "1".to_string());
        props2.insert("b".to_string(), "2".to_string());

        let resp2 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":test".to_string(),
                input_properties: props2,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp1.input_hash, resp2.input_hash);
    }

    #[test]
    fn test_scheduler_running_count() {
        let scheduler = WorkerScheduler::new(3);

        assert_eq!(scheduler.running_count(), 0);

        scheduler.start_work(":t1".to_string(), 0);
        scheduler.start_work(":t2".to_string(), 0);
        assert_eq!(scheduler.running_count(), 2);

        scheduler.complete_work(":t1");
        assert_eq!(scheduler.running_count(), 1);

        // Completing non-existent work is safe
        scheduler.complete_work(":nonexistent");
        assert_eq!(scheduler.running_count(), 1);
    }

    // --- Additional edge-case tests (4 new) ---

    /// Evaluate for a task with no history returns default (no previous execution).
    /// Same task_path, non-empty input_properties, never recorded → should_execute=true.
    #[tokio::test]
    async fn test_evaluate_no_history_with_properties_returns_default() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        let mut props = std::collections::HashMap::new();
        props.insert("sourceSet".to_string(), "main".to_string());
        props.insert("compiler".to_string(), "javac".to_string());

        let resp = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":app:compileKotlin".to_string(),
                input_properties: props,
            }))
            .await
            .unwrap()
            .into_inner();

        // No prior record exists for this task, so it must be scheduled.
        assert!(resp.should_execute);
        assert!(
            resp.reason.contains("no previous execution record"),
            "expected 'no previous' in reason, got: {}",
            resp.reason
        );
        // input_hash should be non-empty and deterministic
        assert!(!resp.input_hash.is_empty());
    }

    /// Evaluate after recording execution returns updated result.
    /// Record a failed execution, then evaluate with the same inputs → UP_TO_DATE even on failure.
    #[tokio::test]
    async fn test_evaluate_after_recording_failed_execution() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        // First evaluation → must execute
        let eval1 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":app:test".to_string(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(eval1.should_execute);

        // Record a *failed* execution
        let rec = svc
            .record_execution(Request::new(WorkRecordRequest {
                task_path: ":app:test".to_string(),
                duration_ms: 3000,
                success: false,
                input_hash: eval1.input_hash.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(rec.acknowledged);

        // Re-evaluate with identical inputs → UP_TO_DATE (failure is still cached)
        let eval2 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":app:test".to_string(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!eval2.should_execute, "failed task should still be considered UP_TO_DATE on unchanged inputs");
        assert!(eval2.reason.contains("UP_TO_DATE"));
        assert!(
            eval2.reason.contains("failed"),
            "reason should mention previous failure, got: {}",
            eval2.reason
        );
        assert!(
            eval2.reason.contains("3000ms"),
            "reason should include the recorded duration, got: {}",
            eval2.reason
        );
    }

    /// Record execution for a task never evaluated.
    /// Directly record without calling evaluate first, then evaluate → UP_TO_DATE.
    #[tokio::test]
    async fn test_record_then_evaluate_task_never_evaluated() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        // Record a successful execution *without* ever calling evaluate first
        let rec = svc
            .record_execution(Request::new(WorkRecordRequest {
                task_path: ":lib:processResources".to_string(),
                duration_ms: 42,
                success: true,
                input_hash: "abc123".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(rec.acknowledged);

        // Compute the hash for known input properties so we can record a matching entry.
        // The service computes MD5(sorted property values joined by "|").
        let mut props_for_hash = std::collections::HashMap::new();
        props_for_hash.insert("only".to_string(), "val".to_string());
        let mut sorted: Vec<&String> = props_for_hash.values().collect();
        sorted.sort();
        let input_str = sorted.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("|");
        let mut hasher = Md5::new();
        hasher.update(input_str.as_bytes());
        let expected_hash = format!("{:x}", hasher.finalize());

        // Record with the correct hash
        svc.record_execution(Request::new(WorkRecordRequest {
            task_path: ":lib:processResources".to_string(),
            duration_ms: 42,
            success: true,
            input_hash: expected_hash.clone(),
        }))
        .await
        .unwrap();

        // Evaluate with matching properties → UP_TO_DATE
        let eval = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: ":lib:processResources".to_string(),
                input_properties: props_for_hash,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!eval.should_execute);
        assert!(eval.reason.contains("UP_TO_DATE"));
        assert!(eval.reason.contains("succeeded"));
        assert!(eval.reason.contains("42ms"));
    }

    /// Multiple record + evaluate cycles show updated history.
    /// Simulate three successive builds: first (no history), second (same inputs → skip),
    /// third (inputs changed → execute again), fourth (same as third → skip).
    #[tokio::test]
    async fn test_multiple_record_evaluate_cycles_show_updated_history() {
        let scheduler = std::sync::Arc::new(WorkerScheduler::new(4));
        let svc = WorkServiceImpl::new(scheduler);

        let task = ":app:build".to_string();

        // --- Cycle 1: no history → execute ---
        let eval1 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: task.clone(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(eval1.should_execute, "cycle 1: no history should require execution");

        svc.record_execution(Request::new(WorkRecordRequest {
            task_path: task.clone(),
            duration_ms: 1200,
            success: true,
            input_hash: eval1.input_hash.clone(),
        }))
        .await
        .unwrap();

        // --- Cycle 2: same inputs → UP_TO_DATE ---
        let eval2 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: task.clone(),
                input_properties: Default::default(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!eval2.should_execute, "cycle 2: unchanged inputs should be UP_TO_DATE");
        assert!(eval2.reason.contains("1200ms"));

        // --- Cycle 3: inputs changed → execute ---
        let mut new_props = std::collections::HashMap::new();
        new_props.insert("flag".to_string(), "enabled".to_string());

        let eval3 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: task.clone(),
                input_properties: new_props,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(eval3.should_execute, "cycle 3: changed inputs should require execution");
        assert!(eval3.reason.contains("inputs changed"));

        svc.record_execution(Request::new(WorkRecordRequest {
            task_path: task.clone(),
            duration_ms: 800,
            success: true,
            input_hash: eval3.input_hash.clone(),
        }))
        .await
        .unwrap();

        // --- Cycle 4: same as cycle 3 → UP_TO_DATE with new duration ---
        let mut props_cycle4 = std::collections::HashMap::new();
        props_cycle4.insert("flag".to_string(), "enabled".to_string());

        let eval4 = svc
            .evaluate(Request::new(WorkEvaluateRequest {
                task_path: task.clone(),
                input_properties: props_cycle4,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!eval4.should_execute, "cycle 4: unchanged inputs should be UP_TO_DATE");
        assert!(
            eval4.reason.contains("800ms"),
            "cycle 4: should reflect the most recent recorded duration (800ms), got: {}",
            eval4.reason
        );
        assert!(
            eval4.reason.contains("succeeded"),
            "cycle 4: should reflect the most recent success status, got: {}",
            eval4.reason
        );
    }
}
