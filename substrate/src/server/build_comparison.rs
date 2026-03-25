use dashmap::DashMap;
use tonic::{Request, Response, Status};

use super::scopes::BuildId;
#[cfg(test)]
use crate::proto::BuildDataSnapshot;
use crate::proto::{
    build_comparison_service_server::BuildComparisonService, ComparisonSummary,
    GetComparisonResultRequest, GetComparisonResultResponse, RecordBuildDataRequest,
    RecordBuildDataResponse, StartComparisonRequest, StartComparisonResponse, TaskComparison,
};

/// Stored build data for comparison.
struct StoredBuildData {
    build_id: BuildId,
    task_durations: std::collections::HashMap<String, i64>,
    task_outcomes: std::collections::HashMap<String, String>,
    task_order: Vec<String>,
    total_duration_ms: i64,
}

/// A comparison between two builds.
struct Comparison {
    comparison_id: BuildId,
    baseline_build_id: BuildId,
    candidate_build_id: BuildId,
}

/// Rust-native build comparison service.
/// Compares two build executions to identify differences in outputs,
/// task graph, and execution times.
pub struct BuildComparisonServiceImpl {
    build_data: DashMap<BuildId, StoredBuildData>,
    comparisons: DashMap<BuildId, Comparison>,
    next_comparison_id: std::sync::atomic::AtomicI64,
}

impl Default for BuildComparisonServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildComparisonServiceImpl {
    pub fn new() -> Self {
        Self {
            build_data: DashMap::new(),
            comparisons: DashMap::new(),
            next_comparison_id: std::sync::atomic::AtomicI64::new(1),
        }
    }
}

#[tonic::async_trait]
impl BuildComparisonService for BuildComparisonServiceImpl {
    async fn start_comparison(
        &self,
        request: Request<StartComparisonRequest>,
    ) -> Result<Response<StartComparisonResponse>, Status> {
        let req = request.into_inner();

        let baseline_key = BuildId::from(req.baseline_build_id.clone());
        let candidate_key = BuildId::from(req.candidate_build_id.clone());

        if !self.build_data.contains_key(&baseline_key) {
            return Err(Status::not_found(format!(
                "Baseline build {} not found",
                req.baseline_build_id
            )));
        }

        if !self.build_data.contains_key(&candidate_key) {
            return Err(Status::not_found(format!(
                "Candidate build {} not found",
                req.candidate_build_id
            )));
        }

        // Validate that stored build_id matches the request key
        let baseline_data = self.build_data.get(&baseline_key).unwrap();
        if baseline_data.build_id.0 != req.baseline_build_id {
            return Err(Status::failed_precondition(format!(
                "Baseline build_id mismatch: stored '{}' vs requested '{}'",
                baseline_data.build_id.0, req.baseline_build_id
            )));
        }
        let candidate_data = self.build_data.get(&candidate_key).unwrap();
        if candidate_data.build_id.0 != req.candidate_build_id {
            return Err(Status::failed_precondition(format!(
                "Candidate build_id mismatch: stored '{}' vs requested '{}'",
                candidate_data.build_id.0, req.candidate_build_id
            )));
        }

        let id = self
            .next_comparison_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let comparison_id = format!("cmp-{}", id);

        let comparison_key = BuildId::from(comparison_id.clone());
        self.comparisons.insert(
            comparison_key.clone(),
            Comparison {
                comparison_id: comparison_key,
                baseline_build_id: BuildId::from(req.baseline_build_id),
                candidate_build_id: BuildId::from(req.candidate_build_id),
            },
        );

        Ok(Response::new(StartComparisonResponse {
            comparison_id,
            started: true,
        }))
    }

    async fn record_build_data(
        &self,
        request: Request<RecordBuildDataRequest>,
    ) -> Result<Response<RecordBuildDataResponse>, Status> {
        let req = request.into_inner();

        let snapshot = req
            .snapshot
            .ok_or_else(|| Status::invalid_argument("BuildDataSnapshot is required"))?;

        if snapshot.build_id.is_empty() {
            return Err(Status::invalid_argument("build_id must not be empty"));
        }

        let total_duration_ms = snapshot.task_durations.values().sum();

        let task_durations: std::collections::HashMap<String, i64> =
            snapshot.task_durations.into_iter().collect();
        let task_outcomes: std::collections::HashMap<String, String> =
            snapshot.task_outcomes.into_iter().collect();

        let build_key = BuildId::from(snapshot.build_id.clone());
        self.build_data.insert(
            build_key.clone(),
            StoredBuildData {
                build_id: build_key,
                task_durations,
                task_outcomes,
                task_order: snapshot.task_order,
                total_duration_ms,
            },
        );

        Ok(Response::new(RecordBuildDataResponse { accepted: true }))
    }

    async fn get_comparison_result(
        &self,
        request: Request<GetComparisonResultRequest>,
    ) -> Result<Response<GetComparisonResultResponse>, Status> {
        let req = request.into_inner();

        let comparison_key = BuildId::from(req.comparison_id.clone());
        let comparison = self.comparisons.get(&comparison_key).ok_or_else(|| {
            Status::not_found(format!("Comparison {} not found", req.comparison_id))
        })?;

        let baseline = self.build_data.get(&comparison.baseline_build_id).unwrap();
        let candidate = self.build_data.get(&comparison.candidate_build_id).unwrap();

        let mut task_comparisons = Vec::new();
        let mut only_baseline = 0i32;
        let mut only_candidate = 0i32;
        let mut changed_outcome = 0i32;
        let mut regressions = 0i32;
        let mut improvements = 0i32;

        // Compare all tasks from both builds
        let all_tasks: std::collections::HashSet<String> = baseline
            .task_durations
            .keys()
            .chain(candidate.task_durations.keys())
            .cloned()
            .collect();

        for task_path in &all_tasks {
            let base_dur = baseline.task_durations.get(task_path).copied().unwrap_or(0);
            let cand_dur = candidate
                .task_durations
                .get(task_path)
                .copied()
                .unwrap_or(0);
            let base_outcome = baseline
                .task_outcomes
                .get(task_path)
                .cloned()
                .unwrap_or_else(|| "UNKNOWN".to_string());
            let cand_outcome = candidate
                .task_outcomes
                .get(task_path)
                .cloned()
                .unwrap_or_else(|| "UNKNOWN".to_string());

            let outcome_changed = base_outcome != cand_outcome;

            let baseline_only = !candidate.task_durations.contains_key(task_path);
            let candidate_only = !baseline.task_durations.contains_key(task_path);

            if baseline_only {
                only_baseline += 1;
            } else if candidate_only {
                only_candidate += 1;
            }

            if outcome_changed {
                changed_outcome += 1;
            }

            let duration_diff = cand_dur - base_dur;
            let duration_ratio = if base_dur > 0 {
                cand_dur as f64 / base_dur as f64
            } else if cand_dur > 0 {
                f64::INFINITY
            } else {
                1.0
            };

            // Consider regression if > 20% slower
            if duration_ratio > 1.2 && !candidate_only {
                regressions += 1;
            } else if duration_ratio < 0.8 && !baseline_only {
                improvements += 1;
            }

            task_comparisons.push(TaskComparison {
                task_path: task_path.clone(),
                baseline_outcome: base_outcome,
                candidate_outcome: cand_outcome,
                baseline_duration_ms: base_dur,
                candidate_duration_ms: cand_dur,
                duration_diff_ms: duration_diff,
                duration_ratio,
                outcome_changed,
            });
        }

        // Sort by duration_diff descending (worst regressions first),
        // with baseline task_order as a secondary tiebreaker for deterministic output.
        let baseline_order_index: std::collections::HashMap<&str, usize> = baseline
            .task_order
            .iter()
            .enumerate()
            .map(|(i, t)| (t.as_str(), i))
            .collect();

        task_comparisons.sort_by(|a, b| {
            let diff_cmp = b.duration_diff_ms.cmp(&a.duration_diff_ms);
            if diff_cmp != std::cmp::Ordering::Equal {
                return diff_cmp;
            }
            let ord_a = baseline_order_index
                .get(a.task_path.as_str())
                .copied()
                .unwrap_or(usize::MAX);
            let ord_b = baseline_order_index
                .get(b.task_path.as_str())
                .copied()
                .unwrap_or(usize::MAX);
            ord_a.cmp(&ord_b)
        });

        let total_diff = candidate.total_duration_ms - baseline.total_duration_ms;

        let summary = ComparisonSummary {
            comparison_id: comparison.comparison_id.0.clone(),
            baseline_build_id: comparison.baseline_build_id.0.clone(),
            candidate_build_id: comparison.candidate_build_id.0.clone(),
            baseline_total_ms: baseline.total_duration_ms,
            candidate_total_ms: candidate.total_duration_ms,
            total_diff_ms: total_diff,
            tasks_only_in_baseline: only_baseline,
            tasks_only_in_candidate: only_candidate,
            tasks_with_changed_outcome: changed_outcome,
            tasks_with_regression: regressions,
            tasks_with_improvement: improvements,
        };

        Ok(Response::new(GetComparisonResultResponse {
            summary: Some(summary),
            task_comparisons,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_snapshot(build_id: &str, tasks: Vec<(&str, &str, i64)>) -> BuildDataSnapshot {
        let mut task_durations = HashMap::new();
        let mut task_outcomes = HashMap::new();
        let mut task_order = Vec::new();

        for (path, outcome, duration) in tasks {
            task_durations.insert(path.to_string(), duration);
            task_outcomes.insert(path.to_string(), outcome.to_string());
            task_order.push(path.to_string());
        }

        BuildDataSnapshot {
            build_id: build_id.to_string(),
            start_time_ms: 0,
            end_time_ms: task_durations.values().sum(),
            task_durations,
            task_outcomes,
            task_order,
            root_dir: "/tmp/project".to_string(),
            input_properties: vec![],
        }
    }

    #[tokio::test]
    async fn test_comparison_basic() {
        let svc = BuildComparisonServiceImpl::new();

        // Record baseline
        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "baseline",
                vec![
                    (":compileJava", "SUCCESS", 1000),
                    (":test", "SUCCESS", 3000),
                    (":jar", "SUCCESS", 500),
                ],
            )),
        }))
        .await
        .unwrap();

        // Record candidate (faster)
        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "candidate",
                vec![
                    (":compileJava", "SUCCESS", 600),
                    (":test", "SUCCESS", 2000),
                    (":jar", "SUCCESS", 300),
                ],
            )),
        }))
        .await
        .unwrap();

        // Start comparison
        let cmp = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "baseline".to_string(),
                candidate_build_id: "candidate".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(cmp.started);

        let result = svc
            .get_comparison_result(Request::new(GetComparisonResultRequest {
                comparison_id: cmp.comparison_id,
            }))
            .await
            .unwrap()
            .into_inner();

        let summary = result.summary.unwrap();
        assert_eq!(summary.baseline_total_ms, 4500);
        assert_eq!(summary.candidate_total_ms, 2900);
        assert_eq!(summary.total_diff_ms, -1600);
        assert!(summary.tasks_with_improvement > 0);
        assert_eq!(summary.tasks_with_changed_outcome, 0);
    }

    #[tokio::test]
    async fn test_comparison_outcome_change() {
        let svc = BuildComparisonServiceImpl::new();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot("base1", vec![(":test", "SUCCESS", 2000)])),
        }))
        .await
        .unwrap();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot("cand1", vec![(":test", "FAILED", 100)])),
        }))
        .await
        .unwrap();

        let cmp = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "base1".to_string(),
                candidate_build_id: "cand1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let result = svc
            .get_comparison_result(Request::new(GetComparisonResultRequest {
                comparison_id: cmp.comparison_id,
            }))
            .await
            .unwrap()
            .into_inner();

        let summary = result.summary.unwrap();
        assert_eq!(summary.tasks_with_changed_outcome, 1);
        assert_eq!(result.task_comparisons[0].outcome_changed, true);
    }

    #[tokio::test]
    async fn test_comparison_missing_build() {
        let svc = BuildComparisonServiceImpl::new();

        let result = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "nonexistent".to_string(),
                candidate_build_id: "also-nonexistent".to_string(),
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_comparison_regression_detection() {
        let svc = BuildComparisonServiceImpl::new();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "base-reg",
                vec![(":compileJava", "SUCCESS", 1000)],
            )),
        }))
        .await
        .unwrap();

        // Candidate is > 20% slower → regression
        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "cand-reg",
                vec![(":compileJava", "SUCCESS", 2000)],
            )),
        }))
        .await
        .unwrap();

        let cmp = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "base-reg".to_string(),
                candidate_build_id: "cand-reg".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let result = svc
            .get_comparison_result(Request::new(GetComparisonResultRequest {
                comparison_id: cmp.comparison_id,
            }))
            .await
            .unwrap()
            .into_inner();

        let summary = result.summary.unwrap();
        assert_eq!(summary.tasks_with_regression, 1);
        assert_eq!(summary.tasks_with_improvement, 0);
    }

    #[tokio::test]
    async fn test_comparison_identical_builds() {
        let svc = BuildComparisonServiceImpl::new();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "base-same",
                vec![(":a", "SUCCESS", 100), (":b", "SUCCESS", 200)],
            )),
        }))
        .await
        .unwrap();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "cand-same",
                vec![(":a", "SUCCESS", 100), (":b", "SUCCESS", 200)],
            )),
        }))
        .await
        .unwrap();

        let cmp = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "base-same".to_string(),
                candidate_build_id: "cand-same".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let result = svc
            .get_comparison_result(Request::new(GetComparisonResultRequest {
                comparison_id: cmp.comparison_id,
            }))
            .await
            .unwrap()
            .into_inner();

        let summary = result.summary.unwrap();
        assert_eq!(summary.total_diff_ms, 0);
        assert_eq!(summary.tasks_with_changed_outcome, 0);
        assert_eq!(summary.tasks_with_regression, 0);
        assert_eq!(summary.tasks_with_improvement, 0);
    }

    #[tokio::test]
    async fn test_get_nonexistent_comparison() {
        let svc = BuildComparisonServiceImpl::new();

        let result = svc
            .get_comparison_result(Request::new(GetComparisonResultRequest {
                comparison_id: "nonexistent".to_string(),
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_task_comparisons_sorted_by_diff() {
        let svc = BuildComparisonServiceImpl::new();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "base-sort",
                vec![(":slow", "SUCCESS", 100), (":fast", "SUCCESS", 1000)],
            )),
        }))
        .await
        .unwrap();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "cand-sort",
                vec![(":slow", "SUCCESS", 500), (":fast", "SUCCESS", 100)],
            )),
        }))
        .await
        .unwrap();

        let cmp = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "base-sort".to_string(),
                candidate_build_id: "cand-sort".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let result = svc
            .get_comparison_result(Request::new(GetComparisonResultRequest {
                comparison_id: cmp.comparison_id,
            }))
            .await
            .unwrap()
            .into_inner();

        // Sorted by duration_diff descending (worst first)
        assert_eq!(result.task_comparisons[0].task_path, ":slow");
        assert_eq!(result.task_comparisons[0].duration_diff_ms, 400);
        assert_eq!(result.task_comparisons[1].task_path, ":fast");
        assert_eq!(result.task_comparisons[1].duration_diff_ms, -900);
    }

    #[tokio::test]
    async fn test_comparison_different_tasks() {
        let svc = BuildComparisonServiceImpl::new();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "base2",
                vec![(":a", "SUCCESS", 100), (":b", "SUCCESS", 200)],
            )),
        }))
        .await
        .unwrap();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "cand2",
                vec![(":b", "SUCCESS", 200), (":c", "SUCCESS", 300)],
            )),
        }))
        .await
        .unwrap();

        let cmp = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "base2".to_string(),
                candidate_build_id: "cand2".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let result = svc
            .get_comparison_result(Request::new(GetComparisonResultRequest {
                comparison_id: cmp.comparison_id,
            }))
            .await
            .unwrap()
            .into_inner();

        let summary = result.summary.unwrap();
        assert_eq!(summary.tasks_only_in_baseline, 1); // :a
        assert_eq!(summary.tasks_only_in_candidate, 1); // :c
    }

    // Edge case 1: start_comparison with nonexistent baseline returns NOT_FOUND
    #[tokio::test]
    async fn test_start_comparison_nonexistent_baseline_returns_not_found() {
        let svc = BuildComparisonServiceImpl::new();

        // Candidate exists but baseline does not
        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "existing-candidate",
                vec![(":compileJava", "SUCCESS", 500)],
            )),
        }))
        .await
        .unwrap();

        let result = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "ghost-baseline".to_string(),
                candidate_build_id: "existing-candidate".to_string(),
            }))
            .await;

        let err = result.expect_err("should fail for nonexistent baseline");
        assert_eq!(err.code(), tonic::Code::NotFound);
        assert!(
            err.message().contains("ghost-baseline"),
            "error message should mention the missing build id"
        );
    }

    // Edge case 2: start_comparison with nonexistent candidate returns NOT_FOUND
    #[tokio::test]
    async fn test_start_comparison_nonexistent_candidate_returns_not_found() {
        let svc = BuildComparisonServiceImpl::new();

        // Baseline exists but candidate does not
        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "existing-baseline",
                vec![(":compileJava", "SUCCESS", 500)],
            )),
        }))
        .await
        .unwrap();

        let result = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "existing-baseline".to_string(),
                candidate_build_id: "ghost-candidate".to_string(),
            }))
            .await;

        let err = result.expect_err("should fail for nonexistent candidate");
        assert_eq!(err.code(), tonic::Code::NotFound);
        assert!(
            err.message().contains("ghost-candidate"),
            "error message should mention the missing build id"
        );
    }

    // Edge case 3: record_build_data with empty task list succeeds and stores zero total duration
    #[tokio::test]
    async fn test_record_build_data_empty_task_list() {
        let svc = BuildComparisonServiceImpl::new();

        let snapshot = BuildDataSnapshot {
            build_id: "empty-build".to_string(),
            start_time_ms: 1000,
            end_time_ms: 1000,
            task_durations: HashMap::new(),
            task_outcomes: HashMap::new(),
            task_order: vec![],
            root_dir: "/tmp/project".to_string(),
            input_properties: vec![],
        };

        let resp = svc
            .record_build_data(Request::new(RecordBuildDataRequest {
                snapshot: Some(snapshot),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.accepted);

        // Verify the build was stored and can be used in a comparison
        // Record a second build with one task so we can compare
        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "with-tasks",
                vec![(":compileJava", "SUCCESS", 300)],
            )),
        }))
        .await
        .unwrap();

        let cmp = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "empty-build".to_string(),
                candidate_build_id: "with-tasks".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(cmp.started);

        let result = svc
            .get_comparison_result(Request::new(GetComparisonResultRequest {
                comparison_id: cmp.comparison_id,
            }))
            .await
            .unwrap()
            .into_inner();

        let summary = result.summary.unwrap();
        assert_eq!(summary.baseline_total_ms, 0);
        assert_eq!(summary.candidate_total_ms, 300);
        assert_eq!(summary.total_diff_ms, 300);
        assert_eq!(summary.tasks_only_in_baseline, 0);
        assert_eq!(summary.tasks_only_in_candidate, 1);
        assert_eq!(summary.tasks_with_changed_outcome, 1); // UNKNOWN vs SUCCESS
    }

    // Edge case 4: start_comparison with same build as both baseline and candidate
    #[tokio::test]
    async fn test_start_comparison_same_build_as_baseline_and_candidate() {
        let svc = BuildComparisonServiceImpl::new();

        svc.record_build_data(Request::new(RecordBuildDataRequest {
            snapshot: Some(make_snapshot(
                "self-compare",
                vec![
                    (":compileJava", "SUCCESS", 1000),
                    (":test", "FAILED", 5000),
                    (":jar", "SUCCESS", 200),
                ],
            )),
        }))
        .await
        .unwrap();

        let cmp = svc
            .start_comparison(Request::new(StartComparisonRequest {
                baseline_build_id: "self-compare".to_string(),
                candidate_build_id: "self-compare".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(cmp.started);
        assert!(cmp.comparison_id.starts_with("cmp-"));

        let result = svc
            .get_comparison_result(Request::new(GetComparisonResultRequest {
                comparison_id: cmp.comparison_id,
            }))
            .await
            .unwrap()
            .into_inner();

        let summary = result.summary.unwrap();
        assert_eq!(summary.baseline_build_id, "self-compare");
        assert_eq!(summary.candidate_build_id, "self-compare");
        assert_eq!(summary.baseline_total_ms, 6200);
        assert_eq!(summary.candidate_total_ms, 6200);
        assert_eq!(summary.total_diff_ms, 0);
        assert_eq!(summary.tasks_with_changed_outcome, 0);
        assert_eq!(summary.tasks_with_regression, 0);
        assert_eq!(summary.tasks_with_improvement, 0);
        assert_eq!(summary.tasks_only_in_baseline, 0);
        assert_eq!(summary.tasks_only_in_candidate, 0);

        // All task comparisons should have diff=0 and outcome_changed=false
        for tc in &result.task_comparisons {
            assert_eq!(tc.duration_diff_ms, 0);
            assert!(!tc.outcome_changed);
            assert_eq!(tc.baseline_outcome, tc.candidate_outcome);
            assert_eq!(tc.baseline_duration_ms, tc.candidate_duration_ms);
        }
        assert_eq!(result.task_comparisons.len(), 3);
    }
}
