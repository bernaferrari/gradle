package org.gradle.internal.rustbridge.testexec;

import org.gradle.api.logging.Logging;
import org.gradle.api.tasks.testing.TestDescriptor;
import org.gradle.api.tasks.testing.TestListener;
import org.gradle.api.tasks.testing.TestResult;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * A {@link TestListener} that shadows test execution events to the Rust substrate.
 * Fire-and-forget: never affects build correctness.
 *
 * Note: TestListener is @DeprecatedInGradleScope (not config-cache compatible),
 * but is still functional when config cache is disabled. Since shadow mode is
 * opt-in, this is acceptable.
 */
public class TestExecutionShadowListener implements TestListener {

    private static final Logger LOGGER = Logging.getLogger(TestExecutionShadowListener.class);

    private final SubstrateClient client;
    private final AtomicInteger suiteCount = new AtomicInteger(0);
    private final AtomicInteger testCount = new AtomicInteger(0);
    private final AtomicInteger passCount = new AtomicInteger(0);
    private final AtomicInteger failCount = new AtomicInteger(0);
    private final AtomicInteger skipCount = new AtomicInteger(0);
    private final List<String> failedTests = new ArrayList<>();

    public TestExecutionShadowListener(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public void beforeSuite(TestDescriptor suite) {
        if (client.isNoop()) {
            return;
        }

        try {
            client.getTestExecutionStub().registerTestSuite(
                gradle.substrate.v1.RegisterTestSuiteRequest.newBuilder()
                    .setBuildId("build")
                    .setSuite(gradle.substrate.v1.TestSuiteDescriptor.newBuilder()
                        .setSuiteId(suite.getClassName() != null ? suite.getClassName() : "suite-" + suiteCount.incrementAndGet())
                        .setSuiteName(suite.getName())
                        .setSuiteType(suite.getClassName() != null ? "junit" : "unknown")
                        .setTestCount(0)
                        .setModulePath(suite.getClassName() != null ? "" : "")
                        .build())
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] shadow beforeSuite failed for {}", suite.getName(), e);
        }
    }

    @Override
    public void afterSuite(TestDescriptor suite, TestResult result) {
        if (client.isNoop()) {
            return;
        }

        try {
            LOGGER.debug("[substrate:testexec] suite '{}' completed: {} passed, {} failed, {} skipped",
                suite.getName(), result.getSuccessfulTestCount(),
                result.getFailedTestCount(), result.getSkippedTestCount());
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] shadow afterSuite failed", e);
        }
    }

    @Override
    public void beforeTest(TestDescriptor testDescriptor) {
        // Nothing to do before individual test
    }

    @Override
    public void afterTest(TestDescriptor testDescriptor, TestResult result) {
        if (client.isNoop()) {
            return;
        }

        try {
            testCount.incrementAndGet();

            String outcome;
            switch (result.getResultType()) {
                case SUCCESS:
                    outcome = "PASSED";
                    passCount.incrementAndGet();
                    break;
                case FAILURE:
                    outcome = "FAILED";
                    failCount.incrementAndGet();
                    failedTests.add(testDescriptor.getName());
                    break;
                case SKIPPED:
                default:
                    outcome = "SKIPPED";
                    skipCount.incrementAndGet();
                    break;
            }

            String suiteId = testDescriptor.getClassName() != null
                ? testDescriptor.getClassName()
                : "unknown";

            client.getTestExecutionStub().reportTestResult(
                gradle.substrate.v1.ReportTestResultRequest.newBuilder()
                    .setBuildId("build")
                    .setResult(gradle.substrate.v1.TestResultEntry.newBuilder()
                        .setTestId(suiteId + "." + testDescriptor.getName())
                        .setSuiteId(suiteId)
                        .setTestName(testDescriptor.getName())
                        .setTestClass(suiteId)
                        .setOutcome(outcome)
                        .setStartTimeMs(result.getStartTime())
                        .setEndTimeMs(result.getEndTime())
                        .setDurationMs(result.getEndTime() - result.getStartTime())
                        .setFailureMessage(result.getException() != null ? result.getException().getMessage() : "")
                        .setFailureType(result.getException() != null ? result.getException().getClass().getSimpleName() : "")
                        .build())
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:testexec] shadow afterTest failed for {}", testDescriptor.getName(), e);
        }
    }
}
