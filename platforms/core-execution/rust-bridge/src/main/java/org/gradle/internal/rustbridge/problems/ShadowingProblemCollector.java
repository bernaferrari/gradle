package org.gradle.internal.rustbridge.problems;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

/**
 * Shadow adapter that compares JVM problem reporting with Rust.
 *
 * <p>In shadow mode, reports problems and queries problem lists
 * through both JVM and Rust paths, reporting mismatches.</p>
 */
public class ShadowingProblemCollector {

    private static final Logger LOGGER = Logging.getLogger(ShadowingProblemCollector.class);

    private final RustProblemReportingClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingProblemCollector(
        RustProblemReportingClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Fire-and-forget shadow of a problem report.
     *
     * <p>Reports the problem to Rust, catches any errors, and reports
     * via the mismatch reporter.</p>
     *
     * @param buildId    the build identifier
     * @param severity   the problem severity
     * @param message    the problem message
     * @param javaResult whether the JVM problem report succeeded
     */
    public void shadowReportProblem(String buildId, String severity, String message, boolean javaResult) {
        try {
            boolean rustResult = rustClient.reportProblem(
                buildId, severity, "", message, "", "", 0, 0, "", ""
            );

            if (javaResult == rustResult) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:problems] shadow reportProblem MATCH: buildId={}, severity={}",
                    buildId, severity);
            } else {
                mismatchReporter.reportMismatch(
                    "problems:reportProblem:" + buildId + ":" + severity,
                    String.valueOf(javaResult),
                    String.valueOf(rustResult)
                );
                LOGGER.debug("[substrate:problems] shadow reportProblem MISMATCH: buildId={}, severity={}, java={}, rust={}",
                    buildId, severity, javaResult, rustResult);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("problems:reportProblem:" + buildId, e);
            LOGGER.debug("[substrate:problems] shadow reportProblem error for buildId={}, severity={}: {}",
                buildId, severity, e.getMessage());
        }
    }

    /**
     * Shadow a get-problems query, comparing problem counts.
     *
     * <p>Queries the Rust problem store and compares the count
     * with the Java problem count.</p>
     *
     * @param buildId           the build identifier
     * @param javaProblemCount  the number of problems in the JVM store
     */
    public void shadowGetProblems(String buildId, int javaProblemCount) {
        try {
            gradle.substrate.v1.GetProblemsResponse rustResponse =
                rustClient.getProblems(buildId);
            int rustProblemCount = rustResponse.getProblemsCount();

            if (javaProblemCount == rustProblemCount) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:problems] shadow getProblems MATCH: buildId={}, count={}",
                    buildId, javaProblemCount);
            } else {
                mismatchReporter.reportMismatch(
                    "problems:getProblems:" + buildId,
                    String.valueOf(javaProblemCount),
                    String.valueOf(rustProblemCount)
                );
                LOGGER.debug("[substrate:problems] shadow getProblems MISMATCH: buildId={}, java={}, rust={}",
                    buildId, javaProblemCount, rustProblemCount);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("problems:getProblems:" + buildId, e);
            LOGGER.debug("[substrate:problems] shadow getProblems error for buildId={}: {}",
                buildId, e.getMessage());
        }
    }
}
