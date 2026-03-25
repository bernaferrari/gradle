package org.gradle.internal.rustbridge.execution;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

/**
 * Shadow listener for the execution plan domain.
 *
 * <p>Compares Java's execution plan decisions against Rust's predictions without
 * affecting build correctness. Fire-and-forget: errors are logged at debug level
 * and never propagated to the build.</p>
 *
 * <p>Style A (compare shadow): runs both Java and Rust, reports matches/mismatches
 * via {@link HashMismatchReporter}, always uses Java as authoritative.</p>
 */
public class ShadowingExecutionPlanAdvisor {

    private static final Logger LOGGER = Logging.getLogger(ShadowingExecutionPlanAdvisor.class);

    private final ExecutionPlanClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingExecutionPlanAdvisor(
        ExecutionPlanClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Shadow-compare Java's execution plan prediction against Rust's prediction.
     *
     * <p>Builds a minimal {@code WorkMetadata} from the work identity string and
     * calls {@link ExecutionPlanClient#predictOutcome(gradle.substrate.v1.WorkMetadata)}.
     * The Rust prediction is compared with Java's prediction string via the
     * mismatch reporter.</p>
     *
     * @param workIdentity   unique identifier for the work unit (e.g. task path)
     * @param javaPrediction Java's predicted outcome as a string (e.g. "EXECUTE", "UP_TO_DATE")
     * @param durationMs     time taken by Java to produce the prediction, in milliseconds
     */
    public void shadowPredictOutcome(String workIdentity, String javaPrediction, long durationMs) {
        if (rustClient == null) {
            return;
        }

        try {
            gradle.substrate.v1.WorkMetadata minimalMetadata =
                gradle.substrate.v1.WorkMetadata.newBuilder()
                    .setWorkIdentity(workIdentity)
                    .build();

            long startTime = System.currentTimeMillis();
            ExecutionPlanClient.Prediction rustPrediction = rustClient.predictOutcome(minimalMetadata);
            long rustDurationMs = System.currentTimeMillis() - startTime;

            String rustPredictionStr = rustPrediction.name();

            if (rustPredictionStr.equals(javaPrediction)) {
                mismatchReporter.reportMatch();
                if (LOGGER.isDebugEnabled()) {
                    LOGGER.debug(
                        "[substrate:execution-plan] shadow OK: {} prediction={} (java={}, rust={}ms)",
                        workIdentity, javaPrediction, javaPrediction, rustDurationMs
                    );
                }
            } else {
                mismatchReporter.reportMismatch(
                    "execution-plan:" + workIdentity,
                    javaPrediction,
                    rustPredictionStr
                );
                LOGGER.info(
                    "[substrate:execution-plan] PREDICTION MISMATCH for {}: java={} rust={} (java_dur={}ms, rust_dur={}ms)",
                    workIdentity, javaPrediction, rustPredictionStr, durationMs, rustDurationMs
                );
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError(
                "execution-plan:" + workIdentity, e
            );
            LOGGER.debug("[substrate:execution-plan] shadow predict failed for {}", workIdentity, e);
        }
    }

    /**
     * Fire-and-forget: record the actual execution outcome to the Rust substrate.
     *
     * <p>This feeds the Rust execution plan service with ground truth so it can
     * improve future predictions. Errors are silently caught to never affect
     * build correctness.</p>
     *
     * @param workIdentity           unique identifier for the work unit
     * @param javaActualOutcome      the actual outcome as determined by Java (e.g. "EXECUTED", "UP_TO_DATE")
     * @param javaPredictionCorrect  whether Java's earlier prediction matched the actual outcome
     * @param durationMs             total execution duration in milliseconds
     */
    public void shadowRecordOutcome(
        String workIdentity,
        String javaActualOutcome,
        boolean javaPredictionCorrect,
        long durationMs
    ) {
        if (rustClient == null) {
            return;
        }

        try {
            ExecutionPlanClient.Prediction predicted = javaPredictionCorrect
                ? ExecutionPlanClient.Prediction.EXECUTE
                : ExecutionPlanClient.Prediction.UNKNOWN;

            rustClient.recordOutcome(
                workIdentity,
                predicted,
                javaActualOutcome,
                javaPredictionCorrect,
                durationMs
            );

            if (LOGGER.isDebugEnabled()) {
                LOGGER.debug(
                    "[substrate:execution-plan] recorded outcome: {} actual={} correct={} duration={}ms",
                    workIdentity, javaActualOutcome, javaPredictionCorrect, durationMs
                );
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:execution-plan] shadow recordOutcome failed for {}", workIdentity, e);
        }
    }
}
