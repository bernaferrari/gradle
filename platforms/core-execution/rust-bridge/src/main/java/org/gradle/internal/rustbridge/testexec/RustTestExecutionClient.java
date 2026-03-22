package org.gradle.internal.rustbridge.testexec;

import gradle.substrate.v1.DetectFlakyTestsRequest;
import gradle.substrate.v1.DetectFlakyTestsResponse;
import gradle.substrate.v1.GetTestReportRequest;
import gradle.substrate.v1.GetTestReportResponse;
import gradle.substrate.v1.GetTestResultsByOutcomeRequest;
import gradle.substrate.v1.GetTestResultsByOutcomeResponse;
import gradle.substrate.v1.GetTestSummaryRequest;
import gradle.substrate.v1.GetTestSummaryResponse;
import gradle.substrate.v1.RegisterTestSuiteRequest;
import gradle.substrate.v1.RegisterTestSuiteResponse;
import gradle.substrate.v1.ReportTestResultRequest;
import gradle.substrate.v1.ReportTestResultResponse;
import gradle.substrate.v1.TestExecutionServiceGrpc;
import gradle.substrate.v1.TestResultEntry;
import gradle.substrate.v1.TestSuiteDescriptor;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust test execution service.
 * Registers test suites and records test results via gRPC.
 */
public class RustTestExecutionClient {

    private static final Logger LOGGER = Logging.getLogger(RustTestExecutionClient.class);

    private final SubstrateClient client;

    public RustTestExecutionClient(SubstrateClient client) {
        this.client = client;
    }

    public boolean registerTestSuite(String buildId, String suiteId, String suiteName,
                                      String suiteType, int testCount, String modulePath) {
        if (client.isNoop()) {
            return false;
        }

        try {
            TestSuiteDescriptor suite = TestSuiteDescriptor.newBuilder()
                .setSuiteId(suiteId)
                .setSuiteName(suiteName)
                .setSuiteType(suiteType)
                .setTestCount(testCount)
                .setModulePath(modulePath != null ? modulePath : "")
                .build();

            RegisterTestSuiteResponse response = client.getTestExecutionStub()
                .registerTestSuite(RegisterTestSuiteRequest.newBuilder()
                    .setBuildId(buildId)
                    .setSuite(suite)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] register test suite failed for {}", suiteId, e);
            return false;
        }
    }

    public boolean reportTestResult(String buildId, String testId, String suiteId,
                                     String testName, String testClass, String outcome,
                                     long startTimeMs, long endTimeMs, long durationMs,
                                     String failureMessage, String failureType,
                                     List<String> failureStackTrace) {
        if (client.isNoop()) {
            return false;
        }

        try {
            TestResultEntry.Builder resultBuilder = TestResultEntry.newBuilder()
                .setTestId(testId)
                .setSuiteId(suiteId)
                .setTestName(testName)
                .setTestClass(testClass)
                .setOutcome(outcome)
                .setStartTimeMs(startTimeMs)
                .setEndTimeMs(endTimeMs)
                .setDurationMs(durationMs)
                .setFailureMessage(failureMessage != null ? failureMessage : "")
                .setFailureType(failureType != null ? failureType : "")
                .addAllFailureStackTrace(failureStackTrace);

            ReportTestResultResponse response = client.getTestExecutionStub()
                .reportTestResult(ReportTestResultRequest.newBuilder()
                    .setBuildId(buildId)
                    .setResult(resultBuilder.build())
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] report test result failed for {}", testId, e);
            return false;
        }
    }

    public GetTestReportResponse getTestReport(String buildId) {
        if (client.isNoop()) {
            return GetTestReportResponse.getDefaultInstance();
        }

        try {
            return client.getTestExecutionStub()
                .getTestReport(GetTestReportRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] get test report failed", e);
            return GetTestReportResponse.getDefaultInstance();
        }
    }

    public GetTestResultsByOutcomeResponse getTestResultsByOutcome(String buildId, String outcome) {
        if (client.isNoop()) {
            return GetTestResultsByOutcomeResponse.getDefaultInstance();
        }

        try {
            return client.getTestExecutionStub()
                .getTestResultsByOutcome(GetTestResultsByOutcomeRequest.newBuilder()
                    .setBuildId(buildId)
                    .setOutcome(outcome)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] get test results by outcome failed", e);
            return GetTestResultsByOutcomeResponse.getDefaultInstance();
        }
    }

    public DetectFlakyTestsResponse detectFlakyTests(String buildId) {
        if (client.isNoop()) {
            return DetectFlakyTestsResponse.getDefaultInstance();
        }

        try {
            return client.getTestExecutionStub()
                .detectFlakyTests(DetectFlakyTestsRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] detect flaky tests failed", e);
            return DetectFlakyTestsResponse.getDefaultInstance();
        }
    }

    public GetTestSummaryResponse getTestSummary(String buildId) {
        if (client.isNoop()) {
            return GetTestSummaryResponse.getDefaultInstance();
        }

        try {
            return client.getTestExecutionStub()
                .getTestSummary(GetTestSummaryRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] get test summary failed", e);
            return GetTestSummaryResponse.getDefaultInstance();
        }
    }
}
