use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    test_execution_service_server::TestExecutionService, GetTestReportRequest,
    GetTestReportResponse, GetTestResultsByOutcomeRequest, GetTestResultsByOutcomeResponse,
    GetTestSummaryRequest, RegisterTestSuiteRequest, RegisterTestSuiteResponse,
    ReportTestResultRequest, ReportTestResultResponse, TestResultEntry,
    TestSuiteDescriptor, TestSuiteReport, TestSummaryResponse,
};

/// Registered test suite metadata.
struct TestSuite {
    descriptor: TestSuiteDescriptor,
    results: Vec<TestResultEntry>,
}

/// History of test outcomes across builds, keyed by test_id.
struct TestHistory {
    /// test_id -> list of outcomes (across builds and reruns)
    outcomes: DashMap<String, Vec<String>>,
    /// test_id -> count of passes
    pass_counts: DashMap<String, i64>,
    /// test_id -> count of failures
    fail_counts: DashMap<String, i64>,
}

/// Rust-native test execution service.
/// Tracks test discovery, execution, result aggregation, and flaky test detection.
pub struct TestExecutionServiceImpl {
    suites: DashMap<String, TestSuite>,     // suite_id -> TestSuite
    build_suites: DashMap<String, Vec<String>>, // build_id -> [suite_id]
    results_reported: AtomicI64,
    history: TestHistory,
}

impl TestExecutionServiceImpl {
    pub fn new() -> Self {
        Self {
            suites: DashMap::new(),
            build_suites: DashMap::new(),
            results_reported: AtomicI64::new(0),
            history: TestHistory {
                outcomes: DashMap::new(),
                pass_counts: DashMap::new(),
                fail_counts: DashMap::new(),
            },
        }
    }

    /// Detect flaky tests: tests that have both PASSED and FAILED outcomes.
    fn detect_flaky_tests(&self, build_id: &str) -> Vec<FlakyTestInfo> {
        let suite_ids = self
            .build_suites
            .get(build_id)
            .map(|s| s.clone())
            .unwrap_or_default();

        let mut flaky = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        for suite_id in &suite_ids {
            if let Some(suite) = self.suites.get(suite_id) {
                for r in &suite.results {
                    if seen.contains(&r.test_id) {
                        continue;
                    }
                    seen.insert(r.test_id.clone());

                    let passes = self
                        .history
                        .pass_counts
                        .get(&r.test_id)
                        .map(|c| *c)
                        .unwrap_or(0);
                    let failures = self
                        .history
                        .fail_counts
                        .get(&r.test_id)
                        .map(|c| *c)
                        .unwrap_or(0);

                    if passes > 0 && failures > 0 {
                        let total_runs = passes + failures;
                        let flake_rate = failures as f64 / total_runs as f64;
                        flaky.push(FlakyTestInfo {
                            test_id: r.test_id.clone(),
                            test_name: r.test_name.clone(),
                            test_class: r.test_class.clone(),
                            pass_count: passes,
                            fail_count: failures,
                            flake_rate,
                        });
                    }
                }
            }
        }

        // Sort by flake rate descending
        flaky.sort_by(|a, b| b.flake_rate.partial_cmp(&a.flake_rate).unwrap_or(std::cmp::Ordering::Equal));
        flaky
    }
}

/// Information about a flaky test.
struct FlakyTestInfo {
    test_id: String,
    test_name: String,
    test_class: String,
    pass_count: i64,
    fail_count: i64,
    flake_rate: f64,
}

#[tonic::async_trait]
impl TestExecutionService for TestExecutionServiceImpl {
    async fn register_test_suite(
        &self,
        request: Request<RegisterTestSuiteRequest>,
    ) -> Result<Response<RegisterTestSuiteResponse>, Status> {
        let req = request.into_inner();

        let descriptor = req
            .suite
            .ok_or_else(|| Status::invalid_argument("TestSuiteDescriptor is required"))?;

        let suite_id = descriptor.suite_id.clone();
        let build_id = req.build_id.clone();

        self.suites.insert(
            suite_id.clone(),
            TestSuite {
                descriptor,
                results: Vec::new(),
            },
        );

        self.build_suites
            .entry(build_id)
            .or_insert_with(Vec::new)
            .push(suite_id);

        Ok(Response::new(RegisterTestSuiteResponse { accepted: true }))
    }

    async fn report_test_result(
        &self,
        request: Request<ReportTestResultRequest>,
    ) -> Result<Response<ReportTestResultResponse>, Status> {
        let req = request.into_inner();

        let result = req
            .result
            .ok_or_else(|| Status::invalid_argument("TestResultEntry is required"))?;

        let suite_id = result.suite_id.clone();
        let test_name = result.test_name.clone();
        let outcome = result.outcome.clone();
        let test_id = result.test_id.clone();

        if let Some(mut suite) = self.suites.get_mut(&suite_id) {
            suite.results.push(result);
        }

        tracing::debug!(
            suite_id = %suite_id,
            test = %test_name,
            outcome = %outcome,
            "Test result reported"
        );

        self.results_reported.fetch_add(1, Ordering::Relaxed);

        // Track outcome history for flaky test detection
        match outcome.as_str() {
            "PASSED" => {
                *self.history.pass_counts.entry(test_id.clone()).or_insert(0) += 1;
            }
            "FAILED" => {
                *self.history.fail_counts.entry(test_id.clone()).or_insert(0) += 1;
            }
            _ => {}
        }
        self.history
            .outcomes
            .entry(test_id)
            .or_insert_with(Vec::new)
            .push(outcome);

        Ok(Response::new(ReportTestResultResponse { accepted: true }))
    }

    async fn get_test_report(
        &self,
        request: Request<GetTestReportRequest>,
    ) -> Result<Response<GetTestReportResponse>, Status> {
        let req = request.into_inner();

        let suite_ids = self
            .build_suites
            .get(&req.build_id)
            .map(|s| s.clone())
            .unwrap_or_default();

        let mut total_tests = 0i32;
        let mut total_passed = 0i32;
        let mut total_failed = 0i32;
        let mut total_skipped = 0i32;
        let mut total_duration_ms = 0i64;
        let mut suite_reports = Vec::new();

        for suite_id in &suite_ids {
            if let Some(suite) = self.suites.get(suite_id) {
                let mut passed = 0i32;
                let mut failed = 0i32;
                let mut skipped = 0i32;
                let mut suite_duration = 0i64;

                for r in &suite.results {
                    match r.outcome.as_str() {
                        "PASSED" => passed += 1,
                        "FAILED" => failed += 1,
                        "SKIPPED" => skipped += 1,
                        _ => {}
                    }
                    suite_duration += r.duration_ms;
                }

                let count = suite.results.len() as i32;
                total_tests += count;
                total_passed += passed;
                total_failed += failed;
                total_skipped += skipped;
                total_duration_ms += suite_duration;

                suite_reports.push(TestSuiteReport {
                    suite: Some(suite.descriptor.clone()),
                    results: suite.results.clone(),
                    passed,
                    failed,
                    skipped,
                    total_duration_ms: suite_duration,
                });
            }
        }

        Ok(Response::new(GetTestReportResponse {
            suites: suite_reports,
            total_tests,
            total_passed,
            total_failed,
            total_skipped,
            total_duration_ms,
        }))
    }

    async fn get_test_results_by_outcome(
        &self,
        request: Request<GetTestResultsByOutcomeRequest>,
    ) -> Result<Response<GetTestResultsByOutcomeResponse>, Status> {
        let req = request.into_inner();

        let suite_ids = self
            .build_suites
            .get(&req.build_id)
            .map(|s| s.clone())
            .unwrap_or_default();

        let mut results = Vec::new();

        for suite_id in &suite_ids {
            if let Some(suite) = self.suites.get(suite_id) {
                for r in &suite.results {
                    if r.outcome == req.outcome {
                        results.push(r.clone());
                    }
                }
            }
        }

        let count = results.len() as i32;

        Ok(Response::new(GetTestResultsByOutcomeResponse { results, count }))
    }

    async fn get_test_summary(
        &self,
        request: Request<GetTestSummaryRequest>,
    ) -> Result<Response<TestSummaryResponse>, Status> {
        let req = request.into_inner();

        let suite_ids = self
            .build_suites
            .get(&req.build_id)
            .map(|s| s.clone())
            .unwrap_or_default();

        let mut tests = 0i32;
        let mut passed = 0i32;
        let mut failed = 0i32;
        let mut skipped = 0i32;
        let mut aborted = 0i32;
        let mut total_duration_ms = 0i64;
        let mut failed_test_names = Vec::new();

        for suite_id in &suite_ids {
            if let Some(suite) = self.suites.get(suite_id) {
                for r in &suite.results {
                    tests += 1;
                    total_duration_ms += r.duration_ms;
                    match r.outcome.as_str() {
                        "PASSED" => passed += 1,
                        "FAILED" => {
                            failed += 1;
                            failed_test_names.push(format!(
                                "{} > {}",
                                r.test_class, r.test_name
                            ));
                        }
                        "SKIPPED" => skipped += 1,
                        "ABORTED" => aborted += 1,
                        _ => {}
                    }
                }
            }
        }

        let pass_rate = if tests > 0 {
            passed as f64 / tests as f64
        } else {
            1.0
        };

        Ok(Response::new(TestSummaryResponse {
            suites: suite_ids.len() as i32,
            tests,
            passed,
            failed,
            skipped,
            aborted,
            total_duration_ms,
            failed_test_names,
            pass_rate,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_suite(suite_id: &str, name: &str) -> TestSuiteDescriptor {
        TestSuiteDescriptor {
            suite_id: suite_id.to_string(),
            suite_name: name.to_string(),
            suite_type: "junit5".to_string(),
            test_count: 0,
            module_path: ":app".to_string(),
        }
    }

    fn make_test_result(suite_id: &str, name: &str, class: &str, outcome: &str, ms: i64) -> TestResultEntry {
        TestResultEntry {
            test_id: format!("{}#{}", class, name),
            suite_id: suite_id.to_string(),
            test_name: name.to_string(),
            test_class: class.to_string(),
            outcome: outcome.to_string(),
            start_time_ms: 0,
            end_time_ms: ms,
            duration_ms: ms,
            failure_message: String::new(),
            failure_type: String::new(),
            failure_stack_trace: vec![],
            rerun: false,
            attempt: 1,
        }
    }

    #[tokio::test]
    async fn test_register_and_report() {
        let svc = TestExecutionServiceImpl::new();

        svc.register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "build-1".to_string(),
            suite: Some(make_suite("suite-1", "MyTest")),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-1".to_string(),
            result: Some(make_test_result("suite-1", "testA", "com.example.MyTest", "PASSED", 100)),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-1".to_string(),
            result: Some(make_test_result("suite-1", "testB", "com.example.MyTest", "PASSED", 200)),
        }))
        .await
        .unwrap();

        let summary = svc
            .get_test_summary(Request::new(GetTestSummaryRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(summary.tests, 2);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.failed, 0);
        assert!((summary.pass_rate - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_failed_tests() {
        let svc = TestExecutionServiceImpl::new();

        svc.register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "build-2".to_string(),
            suite: Some(make_suite("suite-2", "FailingTest")),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-2".to_string(),
            result: Some(make_test_result("suite-2", "testPass", "com.FailingTest", "PASSED", 50)),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-2".to_string(),
            result: Some(make_test_result("suite-2", "testFail", "com.FailingTest", "FAILED", 100)),
        }))
        .await
        .unwrap();

        let summary = svc
            .get_test_summary(Request::new(GetTestSummaryRequest {
                build_id: "build-2".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(summary.failed, 1);
        assert_eq!(summary.failed_test_names.len(), 1);
        assert!(summary.failed_test_names[0].contains("testFail"));

        let by_outcome = svc
            .get_test_results_by_outcome(Request::new(GetTestResultsByOutcomeRequest {
                build_id: "build-2".to_string(),
                outcome: "FAILED".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(by_outcome.count, 1);
    }

    #[tokio::test]
    async fn test_multiple_suites() {
        let svc = TestExecutionServiceImpl::new();

        svc.register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "build-3".to_string(),
            suite: Some(make_suite("s1", "SuiteA")),
        }))
        .await
        .unwrap();

        svc.register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "build-3".to_string(),
            suite: Some(make_suite("s2", "SuiteB")),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-3".to_string(),
            result: Some(make_test_result("s1", "test1", "A", "PASSED", 10)),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-3".to_string(),
            result: Some(make_test_result("s2", "test2", "B", "SKIPPED", 0)),
        }))
        .await
        .unwrap();

        let report = svc
            .get_test_report(Request::new(GetTestReportRequest {
                build_id: "build-3".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(report.suites.len(), 2);
        assert_eq!(report.total_tests, 2);
        assert_eq!(report.total_passed, 1);
        assert_eq!(report.total_skipped, 1);
    }

    #[tokio::test]
    async fn test_empty_build() {
        let svc = TestExecutionServiceImpl::new();

        let summary = svc
            .get_test_summary(Request::new(GetTestSummaryRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(summary.tests, 0);
        assert_eq!(summary.pass_rate, 1.0);
    }

    #[tokio::test]
    async fn test_flaky_test_detection() {
        let svc = TestExecutionServiceImpl::new();

        svc.register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "build-flaky".to_string(),
            suite: Some(make_suite("suite-flaky", "FlakyTest")),
        }))
        .await
        .unwrap();

        // Test passes first time
        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-flaky".to_string(),
            result: Some(make_test_result(
                "suite-flaky",
                "unstableTest",
                "com.FlakyTest",
                "PASSED",
                50,
            )),
        }))
        .await
        .unwrap();

        // Same test fails on rerun (simulates flaky behavior)
        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-flaky".to_string(),
            result: Some(make_test_result(
                "suite-flaky",
                "unstableTest",
                "com.FlakyTest",
                "FAILED",
                75,
            )),
        }))
        .await
        .unwrap();

        // Stable test that always passes
        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-flaky".to_string(),
            result: Some(make_test_result(
                "suite-flaky",
                "stableTest",
                "com.FlakyTest",
                "PASSED",
                25,
            )),
        }))
        .await
        .unwrap();

        let flaky = svc.detect_flaky_tests("build-flaky");

        // Only unstableTest should be flaky
        assert_eq!(flaky.len(), 1);
        assert_eq!(flaky[0].test_name, "unstableTest");
        assert_eq!(flaky[0].pass_count, 1);
        assert_eq!(flaky[0].fail_count, 1);
        assert!((flaky[0].flake_rate - 0.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_no_flaky_when_stable() {
        let svc = TestExecutionServiceImpl::new();

        svc.register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "build-stable".to_string(),
            suite: Some(make_suite("suite-stable", "StableTest")),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-stable".to_string(),
            result: Some(make_test_result("suite-stable", "test1", "A", "PASSED", 10)),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-stable".to_string(),
            result: Some(make_test_result("suite-stable", "test1", "A", "PASSED", 10)),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-stable".to_string(),
            result: Some(make_test_result("suite-stable", "test2", "A", "FAILED", 20)),
        }))
        .await
        .unwrap();

        let flaky = svc.detect_flaky_tests("build-stable");
        assert_eq!(flaky.len(), 0); // test1 always passes, test2 only fails
    }
}
