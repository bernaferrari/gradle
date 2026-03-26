/// Build comparison differential testing: validates BuildComparisonService
/// correctness by recording realistic build data and verifying comparison results.
///
/// Tests cover:
/// - Identical builds produce zero regressions/improvements
/// - Single-task regression detection (>20% slower)
/// - Single-task improvement detection (>20% faster)
/// - Task-only-in-baseline/candidate detection
/// - Outcome change detection (SUCCESS -> FAILED)
/// - Multi-project builds with mixed results
/// - Duration ratio with zero-duration tasks
/// - Deterministic sort order (worst regression first)

use std::collections::HashMap;

use gradle_substrate_daemon::proto::build_comparison_service_server::BuildComparisonService;
use gradle_substrate_daemon::proto::{
    BuildDataSnapshot, GetComparisonResultRequest, RecordBuildDataRequest, StartComparisonRequest,
};
use gradle_substrate_daemon::server::build_comparison::BuildComparisonServiceImpl;
use tonic::Request;

/// Helper: build a BuildDataSnapshot from task tuples (path, outcome, duration_ms).
fn make_snapshot(build_id: &str, tasks: Vec<(&str, &str, i64)>) -> BuildDataSnapshot {
    let mut task_durations = HashMap::new();
    let mut task_outcomes = HashMap::new();
    let mut task_order = Vec::new();
    let total_ms: i64 = tasks.iter().map(|(_, _, d)| d).sum();

    for (path, outcome, duration) in &tasks {
        task_durations.insert(path.to_string(), *duration);
        task_outcomes.insert(path.to_string(), outcome.to_string());
        task_order.push(path.to_string());
    }

    BuildDataSnapshot {
        build_id: build_id.to_string(),
        start_time_ms: 0,
        end_time_ms: total_ms,
        task_durations,
        task_outcomes,
        task_order,
        root_dir: String::new(),
        input_properties: Vec::new(),
    }
}

/// Helper: record build data and start comparison, returning comparison_id.
async fn record_and_compare(
    svc: &BuildComparisonServiceImpl,
    baseline: BuildDataSnapshot,
    candidate: BuildDataSnapshot,
) -> String {
    let baseline_id = baseline.build_id.clone();
    let candidate_id = candidate.build_id.clone();

    svc.record_build_data(Request::new(RecordBuildDataRequest {
        snapshot: Some(baseline),
    }))
    .await
    .unwrap();

    svc.record_build_data(Request::new(RecordBuildDataRequest {
        snapshot: Some(candidate),
    }))
    .await
    .unwrap();

    let resp = svc
        .start_comparison(Request::new(StartComparisonRequest {
            baseline_build_id: baseline_id,
            candidate_build_id: candidate_id,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(resp.started);
    resp.comparison_id
}

#[tokio::test]
async fn identical_builds_produce_no_regressions() {
    let svc = BuildComparisonServiceImpl::new();

    let tasks = vec![
        (":compileJava", "SUCCESS", 1000),
        (":compileKotlin", "SUCCESS", 2000),
        (":test", "SUCCESS", 5000),
        (":jar", "SUCCESS", 500),
    ];

    let baseline = make_snapshot("baseline-identical", tasks.clone());
    let candidate = make_snapshot("candidate-identical", tasks);

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    let summary = result.summary.unwrap();
    assert_eq!(summary.tasks_with_regression, 0);
    assert_eq!(summary.tasks_with_improvement, 0);
    assert_eq!(summary.total_diff_ms, 0);
    assert_eq!(summary.tasks_only_in_baseline, 0);
    assert_eq!(summary.tasks_only_in_candidate, 0);
    assert_eq!(summary.tasks_with_changed_outcome, 0);
}

#[tokio::test]
async fn single_task_regression_detected() {
    let svc = BuildComparisonServiceImpl::new();

    let baseline = make_snapshot(
        "baseline-regression",
        vec![
            (":compileJava", "SUCCESS", 1000),
            (":test", "SUCCESS", 5000),
        ],
    );

    let candidate = make_snapshot(
        "candidate-regression",
        vec![
            (":compileJava", "SUCCESS", 1500), // 50% slower
            (":test", "SUCCESS", 5000),
        ],
    );

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    let summary = result.summary.unwrap();
    assert_eq!(summary.tasks_with_regression, 1);
    assert_eq!(summary.tasks_with_improvement, 0);
    assert_eq!(summary.total_diff_ms, 500);

    // compileJava should be the first in task_comparisons (worst regression first)
    let first = &result.task_comparisons[0];
    assert_eq!(first.task_path, ":compileJava");
    assert_eq!(first.duration_diff_ms, 500);
    assert!((first.duration_ratio - 1.5).abs() < 0.01);
}

#[tokio::test]
async fn single_task_improvement_detected() {
    let svc = BuildComparisonServiceImpl::new();

    let baseline = make_snapshot(
        "baseline-improvement",
        vec![(":compileKotlin", "SUCCESS", 10000)],
    );

    let candidate = make_snapshot(
        "candidate-improvement",
        vec![(":compileKotlin", "SUCCESS", 3000)], // 70% faster
    );

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    let summary = result.summary.unwrap();
    assert_eq!(summary.tasks_with_improvement, 1);
    assert_eq!(summary.tasks_with_regression, 0);
    assert_eq!(summary.total_diff_ms, -7000);
}

#[tokio::test]
async fn tasks_only_in_baseline_detected() {
    let svc = BuildComparisonServiceImpl::new();

    let baseline = make_snapshot(
        "baseline-only",
        vec![
            (":compileJava", "SUCCESS", 1000),
            (":lint", "SUCCESS", 500), // only in baseline
        ],
    );

    let candidate = make_snapshot(
        "candidate-only",
        vec![(":compileJava", "SUCCESS", 1000)],
    );

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    let summary = result.summary.unwrap();
    assert_eq!(summary.tasks_only_in_baseline, 1);
    assert_eq!(summary.tasks_only_in_candidate, 0);
}

#[tokio::test]
async fn tasks_only_in_candidate_detected() {
    let svc = BuildComparisonServiceImpl::new();

    let baseline = make_snapshot(
        "baseline-new",
        vec![(":compileJava", "SUCCESS", 1000)],
    );

    let candidate = make_snapshot(
        "candidate-new",
        vec![
            (":compileJava", "SUCCESS", 1000),
            (":spotlessCheck", "SUCCESS", 200), // new task
        ],
    );

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    let summary = result.summary.unwrap();
    assert_eq!(summary.tasks_only_in_candidate, 1);
    assert_eq!(summary.tasks_only_in_baseline, 0);
}

#[tokio::test]
async fn outcome_change_detected() {
    let svc = BuildComparisonServiceImpl::new();

    let baseline = make_snapshot(
        "baseline-outcome",
        vec![(":test", "SUCCESS", 5000)],
    );

    let candidate = make_snapshot(
        "candidate-outcome",
        vec![(":test", "FAILED", 3000)], // outcome changed + faster
    );

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    let summary = result.summary.unwrap();
    assert_eq!(summary.tasks_with_changed_outcome, 1);

    let task = result
        .task_comparisons
        .iter()
        .find(|t| t.task_path == ":test")
        .unwrap();
    assert!(task.outcome_changed);
    assert_eq!(task.baseline_outcome, "SUCCESS");
    assert_eq!(task.candidate_outcome, "FAILED");
}

#[tokio::test]
async fn multi_project_mixed_results() {
    let svc = BuildComparisonServiceImpl::new();

    let baseline = make_snapshot(
        "baseline-multi",
        vec![
            (":app:compileJava", "SUCCESS", 2000),
            (":app:compileKotlin", "SUCCESS", 3000),
            (":lib:compileJava", "SUCCESS", 1000),
            (":app:test", "SUCCESS", 10000),
            (":lib:test", "SUCCESS", 5000),
        ],
    );

    let candidate = make_snapshot(
        "candidate-multi",
        vec![
            (":app:compileJava", "SUCCESS", 1800),     // 10% faster (not enough for improvement)
            (":app:compileKotlin", "SUCCESS", 5000),   // 67% slower (regression)
            (":lib:compileJava", "SUCCESS", 1000),     // same
            (":app:test", "FAILED", 8000),             // outcome changed + faster
            (":lib:test", "SUCCESS", 2000),            // 60% faster (improvement)
            (":app:spotlessCheck", "SUCCESS", 500),    // new task
        ],
    );

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    let summary = result.summary.unwrap();
    assert_eq!(summary.tasks_with_regression, 1, "compileKotlin regressed");
    assert_eq!(summary.tasks_with_improvement, 1, "lib:test improved");
    assert_eq!(summary.tasks_with_changed_outcome, 2, "app:test + spotlessCheck outcome changed (new task has UNKNOWN vs SUCCESS)");
    assert_eq!(summary.tasks_only_in_candidate, 1, "spotlessCheck is new");
    assert_eq!(summary.tasks_only_in_baseline, 0);
}

#[tokio::test]
async fn zero_duration_tasks_handled() {
    let svc = BuildComparisonServiceImpl::new();

    let baseline = make_snapshot(
        "baseline-zero",
        vec![
            (":compileJava", "SUCCESS", 0),   // cached
            (":processResources", "SUCCESS", 0), // cached
        ],
    );

    let candidate = make_snapshot(
        "candidate-zero",
        vec![
            (":compileJava", "SUCCESS", 0),
            (":processResources", "SUCCESS", 500), // was cached, now executing
        ],
    );

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    // Zero-to-nonzero should be a regression
    let process_res = result
        .task_comparisons
        .iter()
        .find(|t| t.task_path == ":processResources")
        .unwrap();
    assert_eq!(process_res.duration_ratio, f64::INFINITY);
}

#[tokio::test]
async fn sort_order_worst_regression_first() {
    let svc = BuildComparisonServiceImpl::new();

    let baseline = make_snapshot(
        "baseline-sort",
        vec![
            (":a", "SUCCESS", 1000),
            (":b", "SUCCESS", 1000),
            (":c", "SUCCESS", 1000),
        ],
    );

    let candidate = make_snapshot(
        "candidate-sort",
        vec![
            (":a", "SUCCESS", 1100), // +100ms
            (":b", "SUCCESS", 1500), // +500ms (worst)
            (":c", "SUCCESS", 1200), // +200ms
        ],
    );

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    // Should be sorted worst regression first: :b (+500), :c (+200), :a (+100)
    assert_eq!(result.task_comparisons[0].task_path, ":b");
    assert_eq!(result.task_comparisons[1].task_path, ":c");
    assert_eq!(result.task_comparisons[2].task_path, ":a");
}

#[tokio::test]
async fn missing_build_data_returns_not_found() {
    let svc = BuildComparisonServiceImpl::new();

    let result = svc
        .start_comparison(Request::new(StartComparisonRequest {
            baseline_build_id: "nonexistent-baseline".to_string(),
            candidate_build_id: "nonexistent-candidate".to_string(),
        }))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn comparison_with_empty_tasks() {
    let svc = BuildComparisonServiceImpl::new();

    let baseline = make_snapshot("baseline-empty", vec![]);
    let candidate = make_snapshot("candidate-empty", vec![]);

    let cmp_id = record_and_compare(&svc, baseline, candidate).await;

    let result = svc
        .get_comparison_result(Request::new(GetComparisonResultRequest {
            comparison_id: cmp_id,
        }))
        .await
        .unwrap()
        .into_inner();

    let summary = result.summary.unwrap();
    assert_eq!(summary.tasks_with_regression, 0);
    assert_eq!(summary.tasks_with_improvement, 0);
    assert!(result.task_comparisons.is_empty());
}
