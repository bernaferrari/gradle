package org.gradle.internal.rustbridge.metrics;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

/**
 * Shadow adapter that monitors JVM build metrics against Rust.
 *
 * <p>In shadow mode, shadows counter and timer recordings in fire-and-forget
 * mode, and compares performance summary total durations between JVM and Rust.</p>
 */
public class ShadowingMetricsRecorder {

    private static final Logger LOGGER = Logging.getLogger(ShadowingMetricsRecorder.class);

    private final RustBuildMetricsClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingMetricsRecorder(
        RustBuildMetricsClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Shadow a counter recording in fire-and-forget mode.
     *
     * <p>Counter metrics are recorded in Rust but not compared, as they are
     * cumulative and timing-dependent. The Java value is authoritative.</p>
     *
     * @param buildId    the build identifier
     * @param name       the counter name
     * @param javaValue  the counter value from the JVM
     */
    public void shadowRecordCounter(String buildId, String name, long javaValue) {
        try {
            rustClient.recordCounter(buildId, name, (double) javaValue);
            mismatchReporter.reportMatch();
            LOGGER.debug("[substrate:metrics] shadow recordCounter (fire-and-forget): buildId={}, name={}, value={}",
                buildId, name, javaValue);
        } catch (Exception e) {
            mismatchReporter.reportRustError("metrics:recordCounter:" + buildId + ":" + name, e);
            LOGGER.debug("[substrate:metrics] shadow recordCounter error for buildId={}, name={}: {}",
                buildId, name, e.getMessage());
        }
    }

    /**
     * Shadow a timer recording in fire-and-forget mode.
     *
     * <p>Timer metrics are recorded in Rust but not compared individually,
     * as timing values may differ due to measurement overhead. The Java
     * value is authoritative.</p>
     *
     * @param buildId         the build identifier
     * @param name            the timer name
     * @param javaDurationMs  the timer duration from the JVM in milliseconds
     */
    public void shadowRecordTimer(String buildId, String name, long javaDurationMs) {
        try {
            rustClient.recordTimer(buildId, name, javaDurationMs);
            mismatchReporter.reportMatch();
            LOGGER.debug("[substrate:metrics] shadow recordTimer (fire-and-forget): buildId={}, name={}, duration={}ms",
                buildId, name, javaDurationMs);
        } catch (Exception e) {
            mismatchReporter.reportRustError("metrics:recordTimer:" + buildId + ":" + name, e);
            LOGGER.debug("[substrate:metrics] shadow recordTimer error for buildId={}, name={}: {}",
                buildId, name, e.getMessage());
        }
    }

    /**
     * Shadow a performance summary query, comparing JVM and Rust total durations.
     *
     * <p>Retrieves the Rust performance summary and compares the total build
     * duration against the JVM value. A mismatch is reported if the durations
     * differ by more than a small tolerance (10ms) to account for measurement
     * overhead.</p>
     *
     * @param buildId              the build identifier
     * @param javaTotalDurationMs  the total build duration from the JVM in milliseconds
     */
    public void shadowGetPerformanceSummary(String buildId, long javaTotalDurationMs) {
        try {
            RustBuildMetricsClient.PerformanceSummary rustSummary =
                rustClient.getPerformanceSummary(buildId);

            if (rustSummary == null) {
                mismatchReporter.reportRustError(
                    "metrics:performanceSummary:" + buildId,
                    new RuntimeException("Rust returned null summary")
                );
                LOGGER.debug("[substrate:metrics] shadow getPerformanceSummary: Rust returned null for buildId={}",
                    buildId);
                return;
            }

            long rustDurationMs = rustSummary.getDurationMs();
            long toleranceMs = 10L;

            if (Math.abs(javaTotalDurationMs - rustDurationMs) <= toleranceMs) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:metrics] shadow getPerformanceSummary MATCH: buildId={}, java={}ms, rust={}ms",
                    buildId, javaTotalDurationMs, rustDurationMs);
            } else {
                mismatchReporter.reportMismatch(
                    "metrics:performanceSummary:duration:" + buildId,
                    String.valueOf(javaTotalDurationMs),
                    String.valueOf(rustDurationMs)
                );
                LOGGER.debug("[substrate:metrics] shadow getPerformanceSummary MISMATCH: buildId={}, java={}ms, rust={}ms",
                    buildId, javaTotalDurationMs, rustDurationMs);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("metrics:performanceSummary:" + buildId, e);
            LOGGER.debug("[substrate:metrics] shadow getPerformanceSummary error for buildId={}: {}",
                buildId, e.getMessage());
        }
    }
}
