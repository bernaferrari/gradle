package org.gradle.internal.rustbridge.work;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.Map;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Shadow adapter that compares JVM up-to-date checking with Rust.
 *
 * <p>Runs work-avoidance evaluation on both JVM and Rust, then compares
 * the execute/skip decision and input hash.</p>
 */
public class ShadowingWorkAvoidanceChecker {

    private static final Logger LOGGER = Logging.getLogger(ShadowingWorkAvoidanceChecker.class);

    private final WorkerSchedulerClient rustClient;
    private final HashMismatchReporter mismatchReporter;
    private final AtomicLong matchCount = new AtomicLong(0);
    private final AtomicLong mismatchCount = new AtomicLong(0);
    private final AtomicLong errorCount = new AtomicLong(0);

    public ShadowingWorkAvoidanceChecker(
        WorkerSchedulerClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Compare JVM up-to-date decision with Rust work-avoidance evaluation.
     *
     * @param taskPath        the task path being evaluated
     * @param inputProperties the input properties used for up-to-date checking
     * @param javaDecision    the JVM decision ("EXECUTE" or "SKIP")
     * @param javaInputHash   the JVM-computed input hash (may be null)
     */
    public void shadowEvaluate(String taskPath, Map<String, String> inputProperties,
                                String javaDecision, String javaInputHash) {
        try {
            WorkerSchedulerClient.WorkDecision rustDecision = rustClient.evaluate(taskPath, inputProperties);

            if (rustDecision == null) {
                errorCount.incrementAndGet();
                mismatchReporter.reportRustError("work:" + taskPath,
                    new RuntimeException("Rust returned null decision"));
                return;
            }

            String rustDecisionStr = rustDecision.type.name();
            String rustInputHash = rustDecision.inputHash;

            if (!javaDecision.equals(rustDecisionStr)) {
                mismatchCount.incrementAndGet();
                mismatchReporter.reportMismatch(
                    "work:" + taskPath + ":decision",
                    javaDecision,
                    rustDecisionStr
                );
                LOGGER.debug("[substrate:work] DECISION MISMATCH for {}: java={} rust={}",
                    taskPath, javaDecision, rustDecisionStr);
                return;
            }

            if (javaInputHash != null && rustInputHash != null
                    && !javaInputHash.equals(rustInputHash)) {
                mismatchCount.incrementAndGet();
                mismatchReporter.reportMismatch(
                    "work:" + taskPath + ":inputHash",
                    javaInputHash,
                    rustInputHash
                );
                LOGGER.debug("[substrate:work] INPUT HASH MISMATCH for {}: java={} rust={}",
                    taskPath, javaInputHash, rustInputHash);
                return;
            }

            matchCount.incrementAndGet();
            mismatchReporter.reportMatch();
            LOGGER.debug("[substrate:work] shadow OK: {} decision={} hash={}",
                taskPath, rustDecisionStr, rustInputHash != null ? rustInputHash : "n/a");
        } catch (Exception e) {
            errorCount.incrementAndGet();
            mismatchReporter.reportRustError("work:" + taskPath, e);
            LOGGER.debug("[substrate:work] shadow evaluation failed for {}: {}",
                taskPath, e.getMessage());
        }
    }

    /**
     * Fire-and-forget recording of task execution outcome to the Rust scheduler.
     * Errors are silently caught to avoid disrupting the build.
     *
     * @param taskPath   the task path that executed
     * @param durationMs execution duration in milliseconds
     * @param success    whether the task succeeded
     */
    public void shadowRecordExecution(String taskPath, long durationMs, boolean success) {
        try {
            rustClient.recordExecution(taskPath, durationMs, success);
            LOGGER.debug("[substrate:work] recorded execution: {} duration={}ms success={}",
                taskPath, durationMs, success);
        } catch (Exception e) {
            LOGGER.debug("[substrate:work] record execution failed for {}: {}",
                taskPath, e.getMessage());
        }
    }

    /**
     * Get shadow statistics for logging and monitoring.
     */
    public ShadowStats getStats() {
        return new ShadowStats(matchCount.get(), mismatchCount.get(), errorCount.get());
    }

    public static class ShadowStats {
        private final long matches;
        private final long mismatches;
        private final long errors;

        private ShadowStats(long matches, long mismatches, long errors) {
            this.matches = matches;
            this.mismatches = mismatches;
            this.errors = errors;
        }

        public long getMatches() { return matches; }
        public long getMismatches() { return mismatches; }
        public long getErrors() { return errors; }
        public long getTotal() { return matches + mismatches + errors; }
        public double getMismatchRate() {
            long total = matches + mismatches;
            return total == 0 ? 0 : (double) mismatches / total;
        }

        @Override
        public String toString() {
            return String.format("matches=%d, mismatches=%d, errors=%d, mismatchRate=%.1f%%",
                matches, mismatches, errors, getMismatchRate() * 100);
        }
    }
}
