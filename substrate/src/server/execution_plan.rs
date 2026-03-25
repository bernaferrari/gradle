use std::sync::Arc;

use dashmap::DashMap;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use tonic::{Request, Response, Status};

use crate::proto::{
    execution_plan_service_server::ExecutionPlanService, PredictOutcomeRequest,
    PredictOutcomeResponse, PredictedOutcome, RecordOutcomeRequest, RecordOutcomeResponse,
    ResolvePlanRequest, ResolvePlanResponse, WorkMetadata,
};
use crate::server::execution_history::ExecutionHistoryServiceImpl;
use crate::server::work::WorkerScheduler;

/// Tracks prediction accuracy for shadow mode reporting.
#[derive(Default)]
struct PredictionStats {
    total: DashMap<String, i64>,
    correct: DashMap<String, i64>,
}

impl PredictionStats {
    fn record(&self, category: &str, correct: bool) {
        *self.total.entry(category.to_string()).or_insert(0) += 1;
        if correct {
            *self.correct.entry(category.to_string()).or_insert(0) += 1;
        }
    }

    fn accuracy(&self, category: &str) -> f32 {
        let total = self.total.get(category).map(|r| *r).unwrap_or(0);
        let correct = self.correct.get(category).map(|r| *r).unwrap_or(0);
        if total == 0 {
            1.0
        } else {
            correct as f32 / total as f32
        }
    }
}

/// Tracks execution history keyed by work identity + input fingerprint.
#[derive(Default)]
struct ExecutionPlanHistory {
    entries: DashMap<String, ExecutionRecord>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ExecutionRecord {
    input_fingerprint: String,
    outcome: String,
    duration_ms: i64,
    /// Number of consecutive executions (for rebuild loop detection).
    consecutive_executions: i64,
    /// Total execution time across consecutive runs (for average estimation).
    total_consecutive_ms: i64,
}

impl ExecutionRecord {
    /// Estimated duration based on history. Returns 0 if no history.
    fn estimated_duration_ms(&self) -> i64 {
        if self.total_consecutive_ms > 0 && self.consecutive_executions > 0 {
            self.total_consecutive_ms / self.consecutive_executions
        } else {
            self.duration_ms
        }
    }
}

pub struct ExecutionPlanServiceImpl {
    _scheduler: Arc<WorkerScheduler>,
    history: ExecutionPlanHistory,
    stats: PredictionStats,
    /// Optional persistent history for cross-daemon-restart durability.
    persistent_history: Option<Arc<ExecutionHistoryServiceImpl>>,
}

impl Default for ExecutionPlanServiceImpl {
    fn default() -> Self {
        Self::new(std::sync::Arc::new(WorkerScheduler::new(16)))
    }
}

impl ExecutionPlanServiceImpl {
    pub fn new(scheduler: Arc<WorkerScheduler>) -> Self {
        Self {
            _scheduler: scheduler,
            history: ExecutionPlanHistory::default(),
            stats: PredictionStats::default(),
            persistent_history: None,
        }
    }

    pub fn with_persistent_history(
        scheduler: Arc<WorkerScheduler>,
        persistent_history: Arc<ExecutionHistoryServiceImpl>,
    ) -> Self {
        Self {
            _scheduler: scheduler,
            history: ExecutionPlanHistory::default(),
            stats: PredictionStats::default(),
            persistent_history: Some(persistent_history),
        }
    }

    /// Load persisted execution records from history service.
    /// Call this after construction to restore state across daemon restarts.
    pub fn load_persistent_history(&self) {
        let ph = match &self.persistent_history {
            Some(h) => h,
            None => return,
        };

        for entry in ph.entries.iter() {
            let key = entry.key();
            if key.starts_with("__exec_record__:") {
                if let Ok(record) = bincode::deserialize::<ExecutionRecord>(&entry.value().state) {
                    let work_identity = key
                        .strip_prefix("__exec_record__:")
                        .unwrap_or(key)
                        .to_string();
                    self.history.entries.insert(work_identity, record);
                }
            }
        }
        let count = self.history.entries.len();
        if count > 0 {
            tracing::info!("Loaded {} execution records from persistent history", count);
        }
    }

    /// Persist an execution record to the history service.
    fn persist_record(&self, work_identity: &str, record: &ExecutionRecord) {
        if let Some(ph) = &self.persistent_history {
            let key = format!("__exec_record__:{}", work_identity);
            if let Ok(state) = bincode::serialize(record) {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                ph.entries.insert(
                    key.clone(),
                    super::execution_history::HistoryEntry {
                        key,
                        state,
                        timestamp_ms: ts,
                    },
                );
            }
        }
    }

    /// Record an outcome and update execution history for rebuild loop tracking.
    /// This is the synchronous core used by both the gRPC handler and tests.
    fn record_outcome_internal(
        &self,
        work_identity: &str,
        predicted_outcome: i32,
        actual_outcome: &str,
        prediction_correct: bool,
        duration_ms: i64,
    ) {
        self.stats.record("overall", prediction_correct);

        let predicted_name = match PredictedOutcome::try_from(predicted_outcome) {
            Ok(PredictedOutcome::PredictedExecute) => "execute",
            Ok(PredictedOutcome::PredictedUpToDate) => "up_to_date",
            Ok(PredictedOutcome::PredictedFromCache) => "from_cache",
            Ok(PredictedOutcome::PredictedShortCircuited) => "short_circuited",
            _ => "unknown",
        };
        self.stats.record(predicted_name, prediction_correct);

        // Update execution history for rebuild loop tracking
        let is_execute = actual_outcome == "EXECUTED"
            || actual_outcome == "EXECUTED_NON_INCREMENTALLY"
            || actual_outcome == "FAILED";

        let updated_record = if let Some((_, entry)) = self.history.entries.remove(work_identity) {
            let mut entry = entry;
            if is_execute {
                entry.consecutive_executions += 1;
                entry.total_consecutive_ms += duration_ms;
            } else {
                entry.consecutive_executions = 0;
                entry.total_consecutive_ms = 0;
            }
            entry.duration_ms = duration_ms;
            entry.outcome = actual_outcome.to_string();
            Some(entry)
        } else {
            // No existing record — create a new one
            Some(ExecutionRecord {
                input_fingerprint: String::new(), // fingerprint unknown at this point
                outcome: actual_outcome.to_string(),
                duration_ms,
                consecutive_executions: if is_execute { 1 } else { 0 },
                total_consecutive_ms: if is_execute { duration_ms } else { 0 },
            })
        };

        if let Some(record) = updated_record {
            self.history
                .entries
                .insert(work_identity.to_string(), record.clone());
            // Persist to history service for cross-daemon-restart durability
            self.persist_record(work_identity, &record);
        }

        tracing::debug!(
            work = %work_identity,
            predicted = predicted_name,
            actual = %actual_outcome,
            correct = prediction_correct,
            overall_accuracy = self.stats.accuracy("overall"),
            category_accuracy = self.stats.accuracy(predicted_name),
            "Recorded outcome for shadow comparison"
        );
    }

    /// Compute a composite fingerprint from work metadata.
    fn compute_fingerprint(work: &WorkMetadata) -> String {
        let mut hasher = Md5::new();

        // Include scalar input properties in sorted order
        let mut sorted_props: Vec<_> = work.input_properties.iter().collect();
        sorted_props.sort_by_key(|(k, _)| *k);
        for (key, value) in &sorted_props {
            hasher.update(key.as_bytes());
            hasher.update(b"=");
            hasher.update(value.as_bytes());
            hasher.update(b";");
        }

        // Include file fingerprints in sorted order
        let mut sorted_files: Vec<_> = work.input_file_fingerprints.iter().collect();
        sorted_files.sort_by_key(|(k, _)| *k);
        for (key, hash) in &sorted_files {
            hasher.update(key.as_bytes());
            hasher.update(b"=");
            hasher.update(hash.as_bytes());
            hasher.update(b";");
        }

        format!("{:x}", hasher.finalize())
    }

    /// Predict whether the work should execute based on history.
    fn predict(&self, work: &WorkMetadata) -> (PredictedOutcome, String, f32) {
        let fingerprint = Self::compute_fingerprint(work);

        // If there are rebuild reasons from Java, the work needs to execute
        if !work.rebuild_reasons.is_empty() {
            return (
                PredictedOutcome::PredictedExecute,
                format!(
                    "Java detected {} rebuild reason(s): {}",
                    work.rebuild_reasons.len(),
                    work.rebuild_reasons.join(", ")
                ),
                0.95,
            );
        }

        // Check execution history for matching fingerprint
        if let Some(entry) = self.history.entries.get(&work.work_identity) {
            if entry.input_fingerprint == fingerprint {
                let est_ms = entry.estimated_duration_ms();
                let mut reasoning = format!(
                    "Inputs unchanged (previous: {}ms avg: {}ms, outcome: {})",
                    entry.duration_ms, est_ms, entry.outcome
                );
                // Detect potential rebuild loops
                if entry.consecutive_executions > 3 {
                    reasoning = format!(
                        "Inputs unchanged (previous: {}ms avg: {}ms, outcome: {}, {} consecutive executions — possible rebuild loop)",
                        entry.duration_ms, est_ms, entry.outcome, entry.consecutive_executions
                    );
                }
                return (PredictedOutcome::PredictedUpToDate, reasoning, 0.99);
            } else {
                // Inputs changed — check if this is a rebuild loop
                let mut reasoning = format!(
                    "Inputs changed (fingerprint {} -> {})",
                    &entry.input_fingerprint[..8.min(entry.input_fingerprint.len())],
                    &fingerprint[..8.min(fingerprint.len())]
                );
                if entry.consecutive_executions > 3 {
                    reasoning = format!(
                        "{} — {} consecutive executions, possible rebuild loop",
                        reasoning, entry.consecutive_executions
                    );
                }
                return (PredictedOutcome::PredictedExecute, reasoning, 0.95);
            }
        }

        // No history: check if caching might help
        if work.caching_enabled && work.can_load_from_cache {
            return (
                PredictedOutcome::PredictedFromCache,
                "No execution history, but caching is enabled - may load from cache".to_string(),
                0.5,
            );
        }

        // No history, no cache: must execute
        (
            PredictedOutcome::PredictedExecute,
            "No previous execution record".to_string(),
            0.9,
        )
    }
}

#[tonic::async_trait]
impl ExecutionPlanService for ExecutionPlanServiceImpl {
    async fn predict_outcome(
        &self,
        request: Request<PredictOutcomeRequest>,
    ) -> Result<Response<PredictOutcomeResponse>, Status> {
        let req = request.into_inner();
        let work = req.work.unwrap();

        let (predicted, reasoning, confidence) = self.predict(&work);

        let predicted_name = match predicted {
            PredictedOutcome::PredictedExecute => "execute",
            PredictedOutcome::PredictedUpToDate => "up_to_date",
            PredictedOutcome::PredictedFromCache => "from_cache",
            PredictedOutcome::PredictedShortCircuited => "short_circuited",
            PredictedOutcome::PredictedUnknown => "unknown",
        };

        tracing::debug!(
            work = %work.display_name,
            ?predicted,
            confidence,
            category_accuracy = self.stats.accuracy(predicted_name),
            overall_accuracy = self.stats.accuracy("overall"),
            %reasoning,
            "Phase 5: Predicted outcome"
        );

        Ok(Response::new(PredictOutcomeResponse {
            predicted_outcome: predicted as i32,
            reasoning,
            confidence,
        }))
    }

    async fn resolve_plan(
        &self,
        request: Request<ResolvePlanRequest>,
    ) -> Result<Response<ResolvePlanResponse>, Status> {
        let req = request.into_inner();
        let work = req.work.unwrap();
        let fingerprint = Self::compute_fingerprint(&work);

        let (action, reasoning, cache_key_hint);

        if !work.rebuild_reasons.is_empty() {
            action = crate::proto::PlanAction::Execute as i32;
            reasoning = format!(
                "Java detected {} rebuild reason(s)",
                work.rebuild_reasons.len()
            );
            cache_key_hint = String::new();
        } else if let Some(entry) = self.history.entries.get(&work.work_identity) {
            if entry.input_fingerprint == fingerprint {
                action = crate::proto::PlanAction::SkipUpToDate as i32;
                let est_ms = entry.estimated_duration_ms();
                if entry.consecutive_executions > 3 {
                    reasoning = format!(
                        "Inputs unchanged (previous: {}ms avg: {}ms, {} consecutive executions — possible rebuild loop)",
                        entry.duration_ms, est_ms, entry.consecutive_executions
                    );
                } else {
                    reasoning = format!(
                        "Inputs unchanged (previous: {}ms avg: {}ms)",
                        entry.duration_ms, est_ms
                    );
                }
                cache_key_hint = String::new();
            } else {
                if entry.consecutive_executions > 3 {
                    // Rebuild loop detected — still execute but warn
                    action = crate::proto::PlanAction::Execute as i32;
                    cache_key_hint = String::new();
                    reasoning = format!(
                        "Inputs changed, {} consecutive executions — possible rebuild loop, forcing execute",
                        entry.consecutive_executions
                    );
                } else if work.caching_enabled && work.can_load_from_cache {
                    action = crate::proto::PlanAction::LoadFromCache as i32;
                    // Use the fingerprint as a cache key hint
                    cache_key_hint = fingerprint.clone();
                    let est_ms = entry.estimated_duration_ms();
                    reasoning = format!(
                        "Inputs changed, attempting cache lookup (key hint: {}, est. duration: {}ms)",
                        &fingerprint[..8.min(fingerprint.len())],
                        est_ms
                    );
                } else {
                    action = crate::proto::PlanAction::Execute as i32;
                    cache_key_hint = String::new();
                    let est_ms = entry.estimated_duration_ms();
                    reasoning = format!(
                        "Inputs changed, caching not available (est. duration: {}ms)",
                        est_ms
                    );
                }
            }
        } else if work.caching_enabled && work.can_load_from_cache {
            action = crate::proto::PlanAction::LoadFromCache as i32;
            cache_key_hint = fingerprint.clone();
            reasoning = "No execution history, attempting cache lookup".to_string();
        } else {
            action = crate::proto::PlanAction::Execute as i32;
            cache_key_hint = String::new();
            reasoning = "No execution history, must execute".to_string();
        }

        tracing::info!(
            work = %work.display_name,
            authoritative = req.authoritative,
            action,
            %reasoning,
            "Phase 6: Resolved execution plan"
        );

        Ok(Response::new(ResolvePlanResponse {
            action,
            reasoning,
            cache_key_hint,
        }))
    }

    async fn record_outcome(
        &self,
        request: Request<RecordOutcomeRequest>,
    ) -> Result<Response<RecordOutcomeResponse>, Status> {
        let req = request.into_inner();

        self.record_outcome_internal(
            &req.work_identity,
            req.predicted_outcome,
            &req.actual_outcome,
            req.prediction_correct,
            req.duration_ms,
        );

        Ok(Response::new(RecordOutcomeResponse { acknowledged: true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::WorkMetadata;
    use std::collections::HashMap;

    fn make_work(
        identity: &str,
        props: Vec<(&str, &str)>,
        file_fps: Vec<(&str, &str)>,
    ) -> WorkMetadata {
        let input_properties: HashMap<String, String> = props
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let input_file_fingerprints: HashMap<String, String> = file_fps
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        WorkMetadata {
            work_identity: identity.to_string(),
            display_name: identity.to_string(),
            implementation_class: "com.example.FakeTask".to_string(),
            input_properties,
            input_file_fingerprints,
            caching_enabled: true,
            can_load_from_cache: true,
            has_previous_execution_state: false,
            rebuild_reasons: Vec::new(),
        }
    }

    fn make_service() -> ExecutionPlanServiceImpl {
        let scheduler = Arc::new(WorkerScheduler::new(4));
        ExecutionPlanServiceImpl::new(scheduler)
    }

    #[tokio::test]
    async fn test_predict_no_history() {
        let svc = make_service();
        let work = make_work(":compileJava", vec![("source", "src/main/java")], vec![]);

        let (outcome, reason, confidence) = svc.predict(&work);
        assert_eq!(outcome, PredictedOutcome::PredictedFromCache);
        assert!(reason.contains("caching is enabled"));
        assert!(confidence < 1.0);
    }

    #[tokio::test]
    async fn test_predict_up_to_date() {
        let svc = make_service();
        let work = make_work(":compileJava", vec![("source", "src/main/java")], vec![]);

        // Simulate previous execution with same inputs
        let fp = ExecutionPlanServiceImpl::compute_fingerprint(&work);
        svc.history.entries.insert(
            ":compileJava".to_string(),
            ExecutionRecord {
                input_fingerprint: fp,
                outcome: "EXECUTED_NON_INCREMENTALLY".to_string(),
                duration_ms: 1200,
                consecutive_executions: 1,
                total_consecutive_ms: 1200,
            },
        );

        let (outcome, reason, confidence) = svc.predict(&work);
        assert_eq!(outcome, PredictedOutcome::PredictedUpToDate);
        assert!(reason.contains("Inputs unchanged"));
        assert!(confidence > 0.9);
    }

    #[tokio::test]
    async fn test_predict_inputs_changed() {
        let svc = make_service();
        let work_v1 = make_work(":compileJava", vec![("source", "v1")], vec![]);
        let work_v2 = make_work(":compileJava", vec![("source", "v2")], vec![]);

        // Record v1 execution
        let fp1 = ExecutionPlanServiceImpl::compute_fingerprint(&work_v1);
        svc.history.entries.insert(
            ":compileJava".to_string(),
            ExecutionRecord {
                input_fingerprint: fp1,
                outcome: "EXECUTED_NON_INCREMENTALLY".to_string(),
                duration_ms: 500,
                consecutive_executions: 1,
                total_consecutive_ms: 500,
            },
        );

        // Predict v2
        let (outcome, reason, _) = svc.predict(&work_v2);
        assert_eq!(outcome, PredictedOutcome::PredictedExecute);
        assert!(reason.contains("Inputs changed"));
    }

    #[tokio::test]
    async fn test_predict_with_rebuild_reasons() {
        let svc = make_service();
        let mut work = make_work(":compileJava", vec![], vec![]);
        work.rebuild_reasons
            .push("Input file 'A.java' has changed".to_string());

        let (outcome, reason, _) = svc.predict(&work);
        assert_eq!(outcome, PredictedOutcome::PredictedExecute);
        assert!(reason.contains("rebuild reason"));
    }

    #[tokio::test]
    async fn test_fingerprint_stability() {
        // Same inputs in different insertion order should produce the same fingerprint
        let work1 = make_work(
            ":test",
            vec![("a", "1"), ("b", "2"), ("c", "3")],
            vec![("f1", "h1"), ("f2", "h2")],
        );
        let work2 = make_work(
            ":test",
            vec![("c", "3"), ("a", "1"), ("b", "2")],
            vec![("f2", "h2"), ("f1", "h1")],
        );

        assert_eq!(
            ExecutionPlanServiceImpl::compute_fingerprint(&work1),
            ExecutionPlanServiceImpl::compute_fingerprint(&work2)
        );
    }

    #[tokio::test]
    async fn test_prediction_stats() {
        let svc = make_service();
        svc.stats.record("overall", true);
        svc.stats.record("overall", true);
        svc.stats.record("overall", false);
        svc.stats.record("execute", true);
        svc.stats.record("execute", false);

        assert!((svc.stats.accuracy("overall") - 0.6667).abs() < 0.01);
        assert!((svc.stats.accuracy("execute") - 0.5).abs() < 0.01);
        assert_eq!(svc.stats.accuracy("nonexistent"), 1.0);
    }

    #[tokio::test]
    async fn test_resolve_plan_execute() {
        let svc = make_service();
        let mut work = make_work(":compileJava", vec![], vec![]);
        work.rebuild_reasons.push("Output removed".to_string());

        let resp = svc
            .resolve_plan(Request::new(ResolvePlanRequest {
                work: Some(work),
                authoritative: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.action, crate::proto::PlanAction::Execute as i32);
    }

    #[tokio::test]
    async fn test_resolve_plan_skip_up_to_date() {
        let svc = make_service();
        let work = make_work(":compileJava", vec![("x", "y")], vec![]);
        let fp = ExecutionPlanServiceImpl::compute_fingerprint(&work);
        svc.history.entries.insert(
            ":compileJava".to_string(),
            ExecutionRecord {
                input_fingerprint: fp,
                outcome: "EXECUTED".to_string(),
                duration_ms: 100,
                consecutive_executions: 1,
                total_consecutive_ms: 100,
            },
        );

        let resp = svc
            .resolve_plan(Request::new(ResolvePlanRequest {
                work: Some(work),
                authoritative: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.action, crate::proto::PlanAction::SkipUpToDate as i32);
    }

    #[tokio::test]
    async fn test_predict_rebuild_loop_detection() {
        let svc = make_service();
        let work = make_work(":compileJava", vec![("source", "src/main/java")], vec![]);
        let fp = ExecutionPlanServiceImpl::compute_fingerprint(&work);

        // Simulate 5 consecutive executions (rebuild loop)
        svc.history.entries.insert(
            ":compileJava".to_string(),
            ExecutionRecord {
                input_fingerprint: fp,
                outcome: "EXECUTED_NON_INCREMENTALLY".to_string(),
                duration_ms: 200,
                consecutive_executions: 5,
                total_consecutive_ms: 1000,
            },
        );

        let (outcome, reason, _) = svc.predict(&work);
        assert_eq!(outcome, PredictedOutcome::PredictedUpToDate);
        assert!(reason.contains("possible rebuild loop"));
        assert!(reason.contains("5 consecutive executions"));
    }

    #[tokio::test]
    async fn test_resolve_plan_rebuild_loop_forces_execute() {
        let svc = make_service();
        let work_v1 = make_work(":compileJava", vec![("source", "v1")], vec![]);
        let work_v2 = make_work(":compileJava", vec![("source", "v2")], vec![]);
        let fp1 = ExecutionPlanServiceImpl::compute_fingerprint(&work_v1);

        // Simulate 5 consecutive executions, now inputs changed again
        svc.history.entries.insert(
            ":compileJava".to_string(),
            ExecutionRecord {
                input_fingerprint: fp1,
                outcome: "EXECUTED".to_string(),
                duration_ms: 150,
                consecutive_executions: 5,
                total_consecutive_ms: 750,
            },
        );

        let resp = svc
            .resolve_plan(Request::new(ResolvePlanRequest {
                work: Some(work_v2),
                authoritative: true,
            }))
            .await
            .unwrap()
            .into_inner();

        // Even with caching enabled, rebuild loop forces execute
        assert_eq!(resp.action, crate::proto::PlanAction::Execute as i32);
        assert!(resp.reasoning.contains("possible rebuild loop"));
    }

    #[tokio::test]
    async fn test_resolve_plan_rebuild_loop_on_up_to_date() {
        let svc = make_service();
        let work = make_work(":compileJava", vec![("x", "y")], vec![]);
        let fp = ExecutionPlanServiceImpl::compute_fingerprint(&work);

        svc.history.entries.insert(
            ":compileJava".to_string(),
            ExecutionRecord {
                input_fingerprint: fp,
                outcome: "EXECUTED".to_string(),
                duration_ms: 100,
                consecutive_executions: 4,
                total_consecutive_ms: 400,
            },
        );

        let resp = svc
            .resolve_plan(Request::new(ResolvePlanRequest {
                work: Some(work),
                authoritative: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Still SKIP_UP_TO_DATE but with rebuild loop warning in reasoning
        assert_eq!(resp.action, crate::proto::PlanAction::SkipUpToDate as i32);
        assert!(resp.reasoning.contains("possible rebuild loop"));
        assert!(resp.reasoning.contains("avg:"));
    }

    #[test]
    fn test_record_outcome_tracks_consecutive() {
        let svc = ExecutionPlanServiceImpl::default();
        let fp = ExecutionPlanServiceImpl::compute_fingerprint(&make_work(
            ":compileJava",
            vec![("x", "y")],
            vec![],
        ));

        // Seed initial history
        svc.history.entries.insert(
            ":compileJava".to_string(),
            ExecutionRecord {
                input_fingerprint: fp.clone(),
                outcome: "EXECUTED".to_string(),
                duration_ms: 100,
                consecutive_executions: 1,
                total_consecutive_ms: 100,
            },
        );

        // Record an execution outcome
        svc.record_outcome_internal(
            ":compileJava",
            PredictedOutcome::PredictedExecute as i32,
            "EXECUTED",
            true,
            200,
        );

        // Verify via get clone to avoid holding lock
        let entry = svc.history.entries.get(":compileJava").map(|r| {
            (
                r.consecutive_executions,
                r.total_consecutive_ms,
                r.duration_ms,
            )
        });
        assert_eq!(entry, Some((2, 300, 200)));

        // Record UP_TO_DATE — should reset consecutive counter
        svc.record_outcome_internal(
            ":compileJava",
            PredictedOutcome::PredictedUpToDate as i32,
            "UP_TO_DATE",
            true,
            0,
        );

        let entry = svc.history.entries.get(":compileJava").map(|r| {
            (
                r.consecutive_executions,
                r.total_consecutive_ms,
                r.duration_ms,
            )
        });
        assert_eq!(entry, Some((0, 0, 0)));
    }

    #[tokio::test]
    async fn test_estimated_duration_average() {
        let svc = make_service();
        let work = make_work(":compileJava", vec![("source", "src")], vec![]);
        let fp = ExecutionPlanServiceImpl::compute_fingerprint(&work);

        // 3 executions totaling 600ms
        svc.history.entries.insert(
            ":compileJava".to_string(),
            ExecutionRecord {
                input_fingerprint: fp,
                outcome: "EXECUTED".to_string(),
                duration_ms: 250, // most recent
                consecutive_executions: 3,
                total_consecutive_ms: 600,
            },
        );

        let (outcome, reason, _) = svc.predict(&work);
        assert_eq!(outcome, PredictedOutcome::PredictedUpToDate);
        // Should show average = 200ms
        assert!(reason.contains("avg: 200ms"));
        assert!(reason.contains("previous: 250ms"));
    }

    #[test]
    fn test_persistent_history_roundtrip() {
        let history = Arc::new(ExecutionHistoryServiceImpl::new(std::path::PathBuf::new()));
        let svc = ExecutionPlanServiceImpl::with_persistent_history(
            Arc::new(WorkerScheduler::new(4)),
            Arc::clone(&history),
        );

        // No history initially
        assert!(svc.history.entries.is_empty());

        // Record an outcome
        svc.record_outcome_internal(
            ":compileJava",
            PredictedOutcome::PredictedExecute as i32,
            "EXECUTED",
            true,
            500,
        );

        // Should be in memory
        assert_eq!(svc.history.entries.len(), 1);
        let record = svc.history.entries.get(":compileJava").map(|r| {
            (
                r.consecutive_executions,
                r.total_consecutive_ms,
                r.duration_ms,
                r.outcome.clone(),
            )
        });
        assert_eq!(record, Some((1, 500, 500, "EXECUTED".to_string())));

        // Should also be persisted in history service
        // Note: work_identity is ":compileJava", so key is "__exec_record__::compileJava"
        let persisted = history
            .entries
            .get("__exec_record__::compileJava")
            .map(|e| e.state.clone());
        assert!(persisted.is_some());
        assert!(!persisted.unwrap().is_empty());

        // Create a NEW service instance and load from history
        let svc2 = ExecutionPlanServiceImpl::with_persistent_history(
            Arc::new(WorkerScheduler::new(4)),
            Arc::clone(&history),
        );
        svc2.load_persistent_history();

        // Should have restored the record
        assert_eq!(svc2.history.entries.len(), 1);
        let restored = svc2
            .history
            .entries
            .get(":compileJava")
            .map(|r| (r.consecutive_executions, r.duration_ms));
        assert_eq!(restored, Some((1, 500)));

        // Record another execution — should increment consecutive
        svc2.record_outcome_internal(
            ":compileJava",
            PredictedOutcome::PredictedExecute as i32,
            "EXECUTED",
            true,
            600,
        );

        let record = svc2.history.entries.get(":compileJava").map(|r| {
            (
                r.consecutive_executions,
                r.total_consecutive_ms,
                r.duration_ms,
            )
        });
        assert_eq!(record, Some((2, 1100, 600)));
    }
}
