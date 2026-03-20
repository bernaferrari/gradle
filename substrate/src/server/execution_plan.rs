use std::sync::Arc;

use dashmap::DashMap;
use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    execution_plan_service_server::ExecutionPlanService, PredictOutcomeRequest,
    PredictOutcomeResponse, PredictedOutcome, RecordOutcomeRequest, RecordOutcomeResponse,
    ResolvePlanRequest, ResolvePlanResponse, WorkMetadata,
};
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

struct ExecutionRecord {
    input_fingerprint: String,
    outcome: String,
    duration_ms: i64,
}

pub struct ExecutionPlanServiceImpl {
    scheduler: Arc<WorkerScheduler>,
    history: ExecutionPlanHistory,
    stats: PredictionStats,
}

impl ExecutionPlanServiceImpl {
    pub fn new(scheduler: Arc<WorkerScheduler>) -> Self {
        Self {
            scheduler,
            history: ExecutionPlanHistory::default(),
            stats: PredictionStats::default(),
        }
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
                return (
                    PredictedOutcome::PredictedUpToDate,
                    format!(
                        "Inputs unchanged (previous: {}ms, outcome: {})",
                        entry.duration_ms, entry.outcome
                    ),
                    0.99,
                );
            } else {
                return (
                    PredictedOutcome::PredictedExecute,
                    format!(
                        "Inputs changed (fingerprint {} -> {})",
                        entry.input_fingerprint, fingerprint
                    ),
                    0.95,
                );
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

        tracing::debug!(
            work = %work.display_name,
            ?predicted,
            confidence,
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
                reasoning = format!(
                    "Inputs unchanged (previous execution: {}ms)",
                    entry.duration_ms
                );
                cache_key_hint = String::new();
            } else {
                if work.caching_enabled && work.can_load_from_cache {
                    action = crate::proto::PlanAction::LoadFromCache as i32;
                    // Use the fingerprint as a cache key hint
                    cache_key_hint = fingerprint.clone();
                    reasoning = format!(
                        "Inputs changed, attempting cache lookup (key hint: {})",
                        &fingerprint[..8.min(fingerprint.len())]
                    );
                } else {
                    action = crate::proto::PlanAction::Execute as i32;
                    cache_key_hint = String::new();
                    reasoning = "Inputs changed, caching not available".to_string();
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

        self.stats.record("overall", req.prediction_correct);

        let predicted_name = match PredictedOutcome::try_from(req.predicted_outcome) {
            Ok(PredictedOutcome::PredictedExecute) => "execute",
            Ok(PredictedOutcome::PredictedUpToDate) => "up_to_date",
            Ok(PredictedOutcome::PredictedFromCache) => "from_cache",
            Ok(PredictedOutcome::PredictedShortCircuited) => "short_circuited",
            _ => "unknown",
        };
        self.stats.record(predicted_name, req.prediction_correct);

        tracing::debug!(
            work = %req.work_identity,
            predicted = predicted_name,
            actual = %req.actual_outcome,
            correct = req.prediction_correct,
            "Recorded outcome for shadow comparison"
        );

        Ok(Response::new(RecordOutcomeResponse {
            acknowledged: true,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::proto::WorkMetadata;

    fn make_work(identity: &str, props: Vec<(&str, &str)>, file_fps: Vec<(&str, &str)>) -> WorkMetadata {
        let input_properties: HashMap<String, String> = props.into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let input_file_fingerprints: HashMap<String, String> = file_fps.into_iter()
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
        work.rebuild_reasons.push("Input file 'A.java' has changed".to_string());

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
}
