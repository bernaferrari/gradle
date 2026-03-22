use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use super::scopes::BuildId;

use crate::proto::{
    test_execution_service_server::TestExecutionService, DetectFlakyTestsRequest,
    DetectFlakyTestsResponse, FlakyTestInfo as ProtoFlakyTestInfo, GetTestReportRequest,
    GetTestReportResponse, GetTestResultsByOutcomeRequest, GetTestResultsByOutcomeResponse,
    GetTestSummaryRequest, RegisterTestSuiteRequest, RegisterTestSuiteResponse,
    ReportTestResultRequest, ReportTestResultResponse, TestResultEntry, TestSuiteDescriptor,
    TestSuiteReport, TestSummaryResponse,
};

/// Registered test suite metadata.
struct TestSuite {
    descriptor: TestSuiteDescriptor,
    results: Vec<TestResultEntry>,
}

/// History of test outcomes across builds, keyed by test_id.
#[derive(Default)]
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
#[derive(Default)]
pub struct TestExecutionServiceImpl {
    suites: DashMap<String, TestSuite>,     // suite_id -> TestSuite
    build_suites: DashMap<BuildId, Vec<String>>, // build_id -> [suite_id]
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
    fn detect_flaky_tests(&self, build_id: &BuildId) -> Vec<InternalFlakyTestInfo> {
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
                        flaky.push(InternalFlakyTestInfo {
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

/// Information about a flaky test (internal representation).
struct InternalFlakyTestInfo {
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
        let build_id = BuildId::from(req.build_id.clone());

        self.suites.insert(
            suite_id.clone(),
            TestSuite {
                descriptor,
                results: Vec::new(),
            },
        );

        self.build_suites
            .entry(build_id)
            .or_default()
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
            .or_default()
            .push(outcome);

        Ok(Response::new(ReportTestResultResponse { accepted: true }))
    }

    async fn get_test_report(
        &self,
        request: Request<GetTestReportRequest>,
    ) -> Result<Response<GetTestReportResponse>, Status> {
        let req = request.into_inner();

        let build_id = BuildId::from(req.build_id.clone());
        let suite_ids = self
            .build_suites
            .get(&build_id)
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

        let build_id = BuildId::from(req.build_id.clone());
        let suite_ids = self
            .build_suites
            .get(&build_id)
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

        let build_id = BuildId::from(req.build_id.clone());
        let suite_ids = self
            .build_suites
            .get(&build_id)
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

    async fn detect_flaky_tests(
        &self,
        request: Request<DetectFlakyTestsRequest>,
    ) -> Result<Response<DetectFlakyTestsResponse>, Status> {
        let req = request.into_inner();
        let build_id = BuildId::from(req.build_id);
        let flaky = self.detect_flaky_tests(&build_id);

        Ok(Response::new(DetectFlakyTestsResponse {
            flaky_tests: flaky
                .into_iter()
                .map(|f| ProtoFlakyTestInfo {
                    test_id: f.test_id,
                    test_name: f.test_name,
                    test_class: f.test_class,
                    pass_count: f.pass_count,
                    fail_count: f.fail_count,
                    flake_rate: f.flake_rate,
                })
                .collect(),
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

        let flaky = svc.detect_flaky_tests(&BuildId::from("build-flaky".to_string()));

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

        let flaky = svc.detect_flaky_tests(&BuildId::from("build-stable".to_string()));
        assert_eq!(flaky.len(), 0); // test1 always passes, test2 only fails
    }

    #[tokio::test]
    async fn test_report_to_nonexistent_suite() {
        let svc = TestExecutionServiceImpl::new();

        // Reporting to a suite that doesn't exist should succeed (silently dropped)
        let result = svc
            .report_test_result(Request::new(ReportTestResultRequest {
                build_id: "build-no-suite".to_string(),
                result: Some(make_test_result("nonexistent", "test1", "A", "PASSED", 10)),
            }))
            .await;

        assert!(result.is_ok());
        assert!(result.unwrap().into_inner().accepted);

        // But the result should not appear in any summary
        let summary = svc
            .get_test_summary(Request::new(GetTestSummaryRequest {
                build_id: "build-no-suite".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(summary.tests, 0);
    }

    #[tokio::test]
    async fn test_skipped_and_aborted_counts() {
        let svc = TestExecutionServiceImpl::new();

        svc.register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "build-skip".to_string(),
            suite: Some(make_suite("s-skip", "SkipTest")),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-skip".to_string(),
            result: Some(make_test_result("s-skip", "test1", "A", "PASSED", 10)),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-skip".to_string(),
            result: Some(make_test_result("s-skip", "test2", "A", "SKIPPED", 0)),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-skip".to_string(),
            result: Some(make_test_result("s-skip", "test3", "A", "ABORTED", 5)),
        }))
        .await
        .unwrap();

        let summary = svc
            .get_test_summary(Request::new(GetTestSummaryRequest {
                build_id: "build-skip".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(summary.tests, 3);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.aborted, 1);
    }

    #[tokio::test]
    async fn test_results_reported_counter() {
        let svc = TestExecutionServiceImpl::new();

        svc.register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "build-ct".to_string(),
            suite: Some(make_suite("s-ct", "CT")),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-ct".to_string(),
            result: Some(make_test_result("s-ct", "test1", "A", "PASSED", 10)),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-ct".to_string(),
            result: Some(make_test_result("s-ct", "test2", "A", "FAILED", 20)),
        }))
        .await
        .unwrap();

        assert_eq!(svc.results_reported.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn test_get_test_results_by_outcome_empty() {
        let svc = TestExecutionServiceImpl::new();

        let resp = svc
            .get_test_results_by_outcome(Request::new(GetTestResultsByOutcomeRequest {
                build_id: "nonexistent".to_string(),
                outcome: "PASSED".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.count, 0);
        assert!(resp.results.is_empty());
    }

    #[tokio::test]
    async fn test_detect_flaky_tests_grpc() {
        let svc = TestExecutionServiceImpl::new();

        svc.register_test_suite(Request::new(RegisterTestSuiteRequest {
            build_id: "build-grpc-flaky".to_string(),
            suite: Some(make_suite("suite-g", "FlakyGTest")),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-grpc-flaky".to_string(),
            result: Some(make_test_result("suite-g", "testFlaky", "com.GTest", "PASSED", 10)),
        }))
        .await
        .unwrap();

        svc.report_test_result(Request::new(ReportTestResultRequest {
            build_id: "build-grpc-flaky".to_string(),
            result: Some(make_test_result("suite-g", "testFlaky", "com.GTest", "FAILED", 20)),
        }))
        .await
        .unwrap();

        let resp = TestExecutionService::detect_flaky_tests(
            &svc,
            Request::new(DetectFlakyTestsRequest {
                build_id: "build-grpc-flaky".to_string(),
            }),
        )
        .await
        .unwrap()
        .into_inner();

        assert_eq!(resp.flaky_tests.len(), 1);
        assert_eq!(resp.flaky_tests[0].test_name, "testFlaky");
        assert_eq!(resp.flaky_tests[0].pass_count, 1);
        assert_eq!(resp.flaky_tests[0].fail_count, 1);
        assert!((resp.flaky_tests[0].flake_rate - 0.5).abs() < f64::EPSILON);
    }
}
