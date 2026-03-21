package org.gradle.internal.rustbridge.transform;

import org.gradle.api.Describable;
import org.gradle.api.internal.artifacts.transform.TransformExecutionListener;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

/**
 * A {@link TransformExecutionListener} that shadows artifact transform executions
 * to the Rust substrate. Tracks timing and counts for transform analytics.
 */
public class TransformExecutionShadowListener implements TransformExecutionListener {

    private static final Logger LOGGER = Logging.getLogger(TransformExecutionShadowListener.class);

    private final SubstrateClient client;
    private final AtomicLong transformCount = new AtomicLong(0);

    // Track start times by operation ID for duration computation
    private final Map<String, Long> startTimes = new ConcurrentHashMap<>();

    // Aggregate timing stats
    private final AtomicLong totalTransformTimeMs = new AtomicLong(0);
    private final AtomicLong transformCountCompleted = new AtomicLong(0);
    private final Map<String, AtomicLong> transformTimeByType = new ConcurrentHashMap<>();

    public TransformExecutionShadowListener(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public void beforeTransformExecution(Describable transform, Describable subject) {
        if (client.isNoop()) {
            return;
        }

        try {
            long id = transformCount.incrementAndGet();
            String opId = "transform:" + id;
            String transformName = transform.getDisplayName();
            String subjectName = subject.getDisplayName();

            startTimes.put(opId, System.currentTimeMillis());

            client.getBuildOperationsStub().startOperation(
                gradle.substrate.v1.StartOperationRequest.newBuilder()
                    .setOperationId(opId)
                    .setDisplayName("Transform: " + transformName + " on " + subjectName)
                    .setOperationType("ARTIFACT_TRANSFORM")
                    .setStartTimeMs(System.currentTimeMillis())
                    .putMetadata("transform", transformName)
                    .putMetadata("subject", subjectName)
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:transform] shadow before failed for {}", transform.getDisplayName(), e);
        }
    }

    @Override
    public void afterTransformExecution(Describable transform, Describable subject) {
        if (client.isNoop()) {
            return;
        }

        try {
            long id = transformCount.get();
            String opId = "transform:" + id;
            Long startTime = startTimes.remove(opId);

            long durationMs = startTime != null
                ? System.currentTimeMillis() - startTime
                : 0;

            boolean success = true;

            // Complete the operation with real duration
            client.getBuildOperationsStub().completeOperation(
                gradle.substrate.v1.CompleteOperationRequest.newBuilder()
                    .setOperationId(opId)
                    .setDurationMs(durationMs)
                    .setSuccess(success)
                    .setOutcome(success ? "SUCCESS" : "FAILED")
                    .build()
            );

            // Update aggregate stats
            transformCountCompleted.incrementAndGet();
            totalTransformTimeMs.addAndGet(durationMs);

            String transformName = transform.getDisplayName();
            transformTimeByType
                .computeIfAbsent(transformName, k -> new AtomicLong(0))
                .addAndGet(durationMs);

            LOGGER.debug(
                "[substrate:transform] shadow OK: {} on {} ({}ms)",
                transformName, subject.getDisplayName(), durationMs
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:transform] shadow after failed for {}", transform.getDisplayName(), e);
        }
    }

    /**
     * Get total time spent in artifact transforms.
     */
    public long getTotalTransformTimeMs() {
        return totalTransformTimeMs.get();
    }

    /**
     * Get the number of completed transform executions.
     */
    public long getTransformCountCompleted() {
        return transformCountCompleted.get();
    }

    /**
     * Get per-transform-type timing breakdown.
     */
    public Map<String, Long> getTransformTimeByType() {
        Map<String, Long> result = new HashMap<>();
        for (Map.Entry<String, AtomicLong> entry : transformTimeByType.entrySet()) {
            result.put(entry.getKey(), entry.getValue().get());
        }
        return result;
    }
}
