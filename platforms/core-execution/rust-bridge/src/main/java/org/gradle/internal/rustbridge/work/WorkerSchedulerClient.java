package org.gradle.internal.rustbridge.work;

import gradle.substrate.v1.WorkEvaluateRequest;
import gradle.substrate.v1.WorkEvaluateResponse;
import gradle.substrate.v1.WorkRecordRequest;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.HashMap;
import java.util.Map;

/**
 * Client for the Rust worker scheduling service.
 * Provides work-avoidance decisions and execution recording.
 */
public class WorkerSchedulerClient {

    private static final Logger LOGGER = Logging.getLogger(WorkerSchedulerClient.class);

    private final SubstrateClient client;

    public WorkerSchedulerClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Ask the Rust scheduler whether a task should execute.
     * The scheduler checks execution history for input changes.
     *
     * @return a WorkDecision with the decision and the computed input hash
     */
    public WorkDecision evaluate(String taskPath, Map<String, String> inputProperties) {
        if (client.isNoop()) {
            return WorkDecision.EXECUTE;
        }

        WorkEvaluateResponse response = client.getWorkStub().evaluate(
            WorkEvaluateRequest.newBuilder()
                .setTaskPath(taskPath)
                .putAllInputProperties(inputProperties)
                .build()
        );

        if (response.getShouldExecute()) {
            LOGGER.debug("[substrate:work] {}: {}", taskPath, response.getReason());
            return new WorkDecision(WorkDecision.Type.EXECUTE, response.getInputHash());
        } else {
            LOGGER.lifecycle("[substrate:work] {}: {}", taskPath, response.getReason());
            return new WorkDecision(WorkDecision.Type.SKIP, response.getInputHash());
        }
    }

    /**
     * Record the outcome of a task execution for future up-to-date checks.
     */
    public void recordExecution(String taskPath, long durationMs, boolean success, String inputHash) {
        if (client.isNoop()) {
            return;
        }

        client.getWorkStub().recordExecution(
            WorkRecordRequest.newBuilder()
                .setTaskPath(taskPath)
                .setDurationMs(durationMs)
                .setSuccess(success)
                .setInputHash(inputHash != null ? inputHash : "")
                .build()
        );
    }

    /**
     * Backward-compatible overload without input hash.
     */
    public void recordExecution(String taskPath, long durationMs, boolean success) {
        recordExecution(taskPath, durationMs, success, "");
    }

    public static class WorkDecision {
        public enum Type {
            EXECUTE,
            SKIP
        }

        public final Type type;
        public final String inputHash;

        WorkDecision(Type type, String inputHash) {
            this.type = type;
            this.inputHash = inputHash;
        }
    }

    public enum WorkDecision {
        EXECUTE,
        SKIP
    }
}
