use std::sync::atomic::{AtomicI32, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    build_result_service_server::BuildResultService, BuildOutcome, GetBuildResultRequest,
    GetBuildResultResponse, GetTaskSummaryRequest, GetTaskSummaryResponse, ReportBuildFailureRequest,
    ReportBuildFailureResponse, ReportTaskResultRequest, ReportTaskResultResponse, TaskResult,
    TaskSummaryEntry,
};

/// A tracked build's aggregated state.
struct BuildState {
    failed: bool,
    #[allow(dead_code)]
    failure_type: String,
    #[allow(dead_code)]
    failure_message: String,
    #[allow(dead_code)]
    failed_task_paths: Vec<String>,
}

/// Rust-native build result reporting service.
/// Aggregates task results and build outcomes for structured reporting.
#[derive(Default)]
pub struct BuildResultServiceImpl {
    task_results: DashMap<String, Vec<TaskResult>>, // build_id -> [TaskResult]
    build_states: DashMap<String, BuildState>,
    results_reported: AtomicI32,
}

impl BuildResultServiceImpl {
    pub fn new() -> Self {
        Self {
            task_results: DashMap::new(),
            build_states: DashMap::new(),
            results_reported: AtomicI32::new(0),
        }
    }

    fn count_outcomes(results: &[TaskResult]) -> (i32, i32, i32, i32, i32) {
        let mut executed = 0i32;
        let mut from_cache = 0i32;
        let mut up_to_date = 0i32;
        let mut failed = 0i32;
        let mut skipped = 0i32;

        for r in results {
            match r.outcome.as_str() {
                "SUCCESS" => executed += 1,
                "FROM_CACHE" => from_cache += 1,
                "UP_TO_DATE" => up_to_date += 1,
                "FAILED" => failed += 1,
                "SKIPPED" => skipped += 1,
                _ => executed += 1,
            }
        }

        (executed, from_cache, up_to_date, failed, skipped)
    }
}

#[tonic::async_trait]
impl BuildResultService for BuildResultServiceImpl {
    async fn report_task_result(
        &self,
        request: Request<ReportTaskResultRequest>,
    ) -> Result<Response<ReportTaskResultResponse>, Status> {
        let req = request.into_inner();

        let result = req
            .result
            .ok_or_else(|| Status::invalid_argument("TaskResult is required"))?;

        let task_path = result.task_path.clone();
        let outcome = result.outcome.clone();

        self.task_results
            .entry(req.build_id.clone())
            .or_default()
            .push(result);

        self.results_reported.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(
            build_id = %req.build_id,
            task = %task_path,
            outcome = %outcome,
            "Task result reported"
        );

        Ok(Response::new(ReportTaskResultResponse { accepted: true }))
    }

    async fn report_build_failure(
        &self,
        request: Request<ReportBuildFailureRequest>,
    ) -> Result<Response<ReportBuildFailureResponse>, Status> {
        let req = request.into_inner();

        self.build_states.insert(
            req.build_id.clone(),
            BuildState {
                failed: true,
                failure_type: req.failure_type,
                failure_message: req.failure_message,
                failed_task_paths: req.failed_task_paths,
            },
        );

        tracing::warn!(
            build_id = %req.build_id,
            "Build failure reported"
        );

        Ok(Response::new(ReportBuildFailureResponse { accepted: true }))
    }

    async fn get_build_result(
        &self,
        request: Request<GetBuildResultRequest>,
    ) -> Result<Response<GetBuildResultResponse>, Status> {
        let req = request.into_inner();

        let results = self
            .task_results
            .get(&req.build_id)
            .map(|r| r.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let (executed, from_cache, up_to_date, failed, skipped) =
            Self::count_outcomes(&results);

        let build_state = self.build_states.get(&req.build_id);

        let overall_result = if let Some(state) = &build_state {
            if state.failed {
                "FAILED"
            } else {
                "SUCCESS"
            }
        } else if failed > 0 {
            "FAILED"
        } else {
            "SUCCESS"
        };

        let total_duration_ms: i64 = results.iter().map(|r| r.duration_ms).sum();

        let outcome = BuildOutcome {
            build_id: req.build_id,
            overall_result: overall_result.to_string(),
            total_duration_ms,
            tasks_total: results.len() as i32,
            tasks_executed: executed,
            tasks_from_cache: from_cache,
            tasks_up_to_date: up_to_date,
            tasks_failed: failed,
            tasks_skipped: skipped,
        };

        Ok(Response::new(GetBuildResultResponse {
            outcome: Some(outcome),
            task_results: results,
        }))
    }

    async fn get_task_summary(
        &self,
        request: Request<GetTaskSummaryRequest>,
    ) -> Result<Response<GetTaskSummaryResponse>, Status> {
        let req = request.into_inner();

        let results = self
            .task_results
            .get(&req.build_id)
            .map(|r| r.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let total_duration_ms: i64 = results.iter().map(|r| r.duration_ms).sum();

        let tasks: Vec<TaskSummaryEntry> = results
            .into_iter()
            .map(|r| TaskSummaryEntry {
                task_path: r.task_path.clone(),
                outcome: r.outcome.clone(),
                duration_ms: r.duration_ms,
            })
            .collect();

        Ok(Response::new(GetTaskSummaryResponse {
            tasks,
            total_duration_ms,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task_result(path: &str, outcome: &str, duration_ms: i64) -> TaskResult {
        TaskResult {
            task_path: path.to_string(),
            outcome: outcome.to_string(),
            duration_ms,
            did_work: outcome == "SUCCESS",
            cache_key: String::new(),
            start_time_ms: 0,
            end_time_ms: duration_ms,
            failure_message: String::new(),
            execution_reason: 0,
        }
    }

    #[tokio::test]
    async fn test_report_and_get_results() {
        let svc = BuildResultServiceImpl::new();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "build-1".to_string(),
            result: Some(make_task_result(":compileJava", "SUCCESS", 1500)),
        }))
        .await
        .unwrap();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "build-1".to_string(),
            result: Some(make_task_result(":test", "SUCCESS", 3000)),
        }))
        .await
        .unwrap();

        let result = svc
            .get_build_result(Request::new(GetBuildResultRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let outcome = result.outcome.unwrap();
        assert_eq!(outcome.overall_result, "SUCCESS");
        assert_eq!(outcome.tasks_total, 2);
        assert_eq!(outcome.tasks_executed, 2);
        assert_eq!(outcome.total_duration_ms, 4500);
    }

    #[tokio::test]
    async fn test_build_failure() {
        let svc = BuildResultServiceImpl::new();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "build-2".to_string(),
            result: Some(make_task_result(":compileJava", "SUCCESS", 1000)),
        }))
        .await
        .unwrap();

        svc.report_build_failure(Request::new(ReportBuildFailureRequest {
            build_id: "build-2".to_string(),
            failure_type: "build_failed".to_string(),
            failure_message: ":test failed".to_string(),
            failed_task_paths: vec![":test".to_string()],
        }))
        .await
        .unwrap();

        let result = svc
            .get_build_result(Request::new(GetBuildResultRequest {
                build_id: "build-2".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(result.outcome.unwrap().overall_result, "FAILED");
    }

    #[tokio::test]
    async fn test_mixed_outcomes() {
        let svc = BuildResultServiceImpl::new();

        let build_id = "build-3".to_string();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(make_task_result(":compileJava", "UP_TO_DATE", 0)),
        }))
        .await
        .unwrap();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(make_task_result(":processResources", "FROM_CACHE", 5)),
        }))
        .await
        .unwrap();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(make_task_result(":classes", "SUCCESS", 100)),
        }))
        .await
        .unwrap();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: build_id.clone(),
            result: Some(make_task_result(":test", "FAILED", 5000)),
        }))
        .await
        .unwrap();

        let result = svc
            .get_build_result(Request::new(GetBuildResultRequest {
                build_id: build_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        let outcome = result.outcome.unwrap();
        assert_eq!(outcome.tasks_total, 4);
        assert_eq!(outcome.tasks_up_to_date, 1);
        assert_eq!(outcome.tasks_from_cache, 1);
        assert_eq!(outcome.tasks_executed, 1);
        assert_eq!(outcome.tasks_failed, 1);
        assert_eq!(outcome.overall_result, "FAILED");
    }

    #[tokio::test]
    async fn test_task_summary() {
        let svc = BuildResultServiceImpl::new();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "build-4".to_string(),
            result: Some(make_task_result(":a", "SUCCESS", 100)),
        }))
        .await
        .unwrap();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "build-4".to_string(),
            result: Some(make_task_result(":b", "SUCCESS", 200)),
        }))
        .await
        .unwrap();

        let summary = svc
            .get_task_summary(Request::new(GetTaskSummaryRequest {
                build_id: "build-4".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(summary.tasks.len(), 2);
        assert_eq!(summary.total_duration_ms, 300);
    }

    #[tokio::test]
    async fn test_skipped_outcome() {
        let svc = BuildResultServiceImpl::new();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "build-skip".to_string(),
            result: Some(make_task_result(":compileJava", "SKIPPED", 0)),
        }))
        .await
        .unwrap();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "build-skip".to_string(),
            result: Some(make_task_result(":test", "SUCCESS", 1000)),
        }))
        .await
        .unwrap();

        let result = svc
            .get_build_result(Request::new(GetBuildResultRequest {
                build_id: "build-skip".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let outcome = result.outcome.unwrap();
        assert_eq!(outcome.tasks_total, 2);
        assert_eq!(outcome.tasks_skipped, 1);
        assert_eq!(outcome.tasks_executed, 1);
    }

    #[tokio::test]
    async fn test_empty_task_result_rejected() {
        let svc = BuildResultServiceImpl::new();

        let result = svc
            .report_task_result(Request::new(ReportTaskResultRequest {
                build_id: "build-empty".to_string(),
                result: None,
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_task_summary_unknown_build() {
        let svc = BuildResultServiceImpl::new();

        let summary = svc
            .get_task_summary(Request::new(GetTaskSummaryRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(summary.tasks.is_empty());
        assert_eq!(summary.total_duration_ms, 0);
    }

    #[tokio::test]
    async fn test_results_reported_counter() {
        let svc = BuildResultServiceImpl::new();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "build-count".to_string(),
            result: Some(make_task_result(":a", "SUCCESS", 100)),
        }))
        .await
        .unwrap();

        svc.report_task_result(Request::new(ReportTaskResultRequest {
            build_id: "build-count".to_string(),
            result: Some(make_task_result(":b", "SUCCESS", 200)),
        }))
        .await
        .unwrap();

        assert_eq!(svc.results_reported.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn test_unknown_build() {
        let svc = BuildResultServiceImpl::new();

        let result = svc
            .get_build_result(Request::new(GetBuildResultRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let outcome = result.outcome.unwrap();
        assert_eq!(outcome.overall_result, "SUCCESS"); // no failures = success
        assert_eq!(outcome.tasks_total, 0);
    }

    #[tokio::test]
    async fn test_report_build_failure_with_custom_message() {
        let svc = BuildResultServiceImpl::new();

        let custom_msg = "NullPointerException in :app:mergeDebugResources: \
                          Unable to resolve symbol 'R'";
        let resp = svc
            .report_build_failure(Request::new(ReportBuildFailureRequest {
                build_id: "build-custom-fail".to_string(),
                failure_type: "configuration_error".to_string(),
                failure_message: custom_msg.to_string(),
                failed_task_paths: vec![
                    ":app:compileDebugKotlin".to_string(),
                    ":app:mergeDebugResources".to_string(),
                ],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.accepted);

        // Verify the build result reflects the failure
        let result = svc
            .get_build_result(Request::new(GetBuildResultRequest {
                build_id: "build-custom-fail".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let outcome = result.outcome.unwrap();
        assert_eq!(outcome.overall_result, "FAILED");

        // Also verify the internal build state stored the custom message correctly
        let state = svc
            .build_states
            .get("build-custom-fail")
            .expect("build state should exist");
        assert!(state.failed);
        assert_eq!(state.failure_type, "configuration_error");
        assert_eq!(state.failure_message, custom_msg);
        assert_eq!(state.failed_task_paths.len(), 2);
        assert!(state.failed_task_paths.contains(&":app:compileDebugKotlin".to_string()));
        assert!(state.failed_task_paths.contains(&":app:mergeDebugResources".to_string()));
    }

    #[tokio::test]
    async fn test_task_summary_for_build_with_no_tasks() {
        let svc = BuildResultServiceImpl::new();

        // Query task summary for a build that was never reported to
        let summary = svc
            .get_task_summary(Request::new(GetTaskSummaryRequest {
                build_id: "build-no-tasks".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(summary.tasks.is_empty());
        assert_eq!(summary.tasks.len(), 0);
        assert_eq!(summary.total_duration_ms, 0);
    }

    #[tokio::test]
    async fn test_report_many_tasks_aggregation() {
        let svc = BuildResultServiceImpl::new();
        let build_id = "build-large".to_string();
        let num_tasks = 60;

        let mut expected_executed = 0i32;
        let mut expected_from_cache = 0i32;
        let mut expected_up_to_date = 0i32;
        let mut expected_failed = 0i32;
        let mut expected_skipped = 0i32;
        let mut expected_total_duration_ms = 0i64;

        for i in 0..num_tasks {
            let (outcome, duration_ms) = match i % 5 {
                0 => ("SUCCESS", 100i64),
                1 => ("FROM_CACHE", 10i64),
                2 => ("UP_TO_DATE", 0i64),
                3 => ("FAILED", 2000i64),
                4 => ("SKIPPED", 0i64),
                _ => unreachable!(),
            };

            match outcome {
                "SUCCESS" => expected_executed += 1,
                "FROM_CACHE" => expected_from_cache += 1,
                "UP_TO_DATE" => expected_up_to_date += 1,
                "FAILED" => expected_failed += 1,
                "SKIPPED" => expected_skipped += 1,
                _ => {}
            }
            expected_total_duration_ms += duration_ms;

            let task_path = format!(":module{}:task{}", i / 6, i);
            svc.report_task_result(Request::new(ReportTaskResultRequest {
                build_id: build_id.clone(),
                result: Some(make_task_result(&task_path, outcome, duration_ms)),
            }))
            .await
            .unwrap();
        }

        assert_eq!(
            svc.results_reported.load(Ordering::Relaxed),
            num_tasks as i32
        );

        // Verify aggregated build result
        let result = svc
            .get_build_result(Request::new(GetBuildResultRequest {
                build_id: build_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        let outcome = result.outcome.unwrap();
        assert_eq!(outcome.tasks_total, num_tasks as i32);
        assert_eq!(outcome.tasks_executed, expected_executed);
        assert_eq!(outcome.tasks_from_cache, expected_from_cache);
        assert_eq!(outcome.tasks_up_to_date, expected_up_to_date);
        assert_eq!(outcome.tasks_failed, expected_failed);
        assert_eq!(outcome.tasks_skipped, expected_skipped);
        assert_eq!(outcome.overall_result, "FAILED"); // 12 failed tasks
        assert_eq!(outcome.total_duration_ms, expected_total_duration_ms);

        // Verify task summary also returns all entries
        let summary = svc
            .get_task_summary(Request::new(GetTaskSummaryRequest {
                build_id: build_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(summary.tasks.len(), num_tasks as usize);
        assert_eq!(summary.total_duration_ms, expected_total_duration_ms);

        // Verify task result list in build result also has all entries
        assert_eq!(result.task_results.len(), num_tasks as usize);
    }

    #[tokio::test]
    async fn test_get_build_result_nonexistent_returns_success_default() {
        let svc = BuildResultServiceImpl::new();

        let build_id = "build-ghost-xyz-999";
        let result = svc
            .get_build_result(Request::new(GetBuildResultRequest {
                build_id: build_id.to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        // A nonexistent build should return a response (not an error)
        assert!(result.outcome.is_some());
        let outcome = result.outcome.unwrap();
        assert_eq!(outcome.overall_result, "SUCCESS");
        assert_eq!(outcome.tasks_total, 0);
        assert_eq!(outcome.tasks_executed, 0);
        assert_eq!(outcome.tasks_from_cache, 0);
        assert_eq!(outcome.tasks_up_to_date, 0);
        assert_eq!(outcome.tasks_failed, 0);
        assert_eq!(outcome.tasks_skipped, 0);
        assert_eq!(outcome.total_duration_ms, 0);
        assert_eq!(outcome.build_id, build_id);

        // No task results should be returned
        assert!(result.task_results.is_empty());
    }
}
