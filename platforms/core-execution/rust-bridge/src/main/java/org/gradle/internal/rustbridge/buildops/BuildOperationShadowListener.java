package org.gradle.internal.rustbridge.buildops;

import org.gradle.api.logging.Logging;
import org.gradle.internal.operations.BuildOperationDescriptor;
import org.gradle.internal.operations.BuildOperationListener;
import org.gradle.internal.operations.OperationFinishEvent;
import org.gradle.internal.operations.OperationIdentifier;
import org.gradle.internal.operations.OperationProgressEvent;
import org.gradle.internal.operations.OperationStartEvent;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

/**
 * A {@link BuildOperationListener} that shadows all build operations to the Rust substrate.
 * Tracks operation counts, durations, and aggregates by type for analytics.
 * Fire-and-forget: never affects build correctness.
 */
public class BuildOperationShadowListener implements BuildOperationListener {

    private static final Logger LOGGER = Logging.getLogger(BuildOperationShadowListener.class);

    private final SubstrateClient client;

    private final AtomicLong totalOperations = new AtomicLong(0);
    private final AtomicLong totalDurationMs = new AtomicLong(0);
    private final AtomicLong failureCount = new AtomicLong(0);

    // Aggregate by operation type
    private final ConcurrentHashMap<String, AtomicLong> countsByType = new ConcurrentHashMap<>();
    private final ConcurrentHashMap<String, AtomicLong> durationsByType = new ConcurrentHashMap<>();

    // Track the slowest operations (top 10)
    private static final int MAX_SLOW_OPS = 10;
    private final ConcurrentHashMap<String, Long> slowestOps = new ConcurrentHashMap<>();

    public BuildOperationShadowListener(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public void started(BuildOperationDescriptor op, OperationStartEvent event) {
        if (client.isNoop()) {
            return;
        }

        try {
            String id = op.getId() != null ? op.getId().toString() : "";
            String parentId = op.getParentId() != null ? op.getParentId().toString() : "";
            String opType = op.getName() != null ? op.getName() : "unknown";

            client.getBuildOperationsStub().startOperation(
                gradle.substrate.v1.StartOperationRequest.newBuilder()
                    .setOperationId(id)
                    .setDisplayName(op.getDisplayName())
                    .setOperationType(opType)
                    .setParentId(parentId)
                    .setStartTimeMs(event.getStartTime())
                    .putMetadata("type", opType)
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildops] shadow start failed for {}", op.getDisplayName(), e);
        }
    }

    @Override
    public void finished(BuildOperationDescriptor op, OperationFinishEvent event) {
        if (client.isNoop()) {
            return;
        }

        try {
            String id = op.getId() != null ? op.getId().toString() : "";
            long durationMs = event.getEndTime() - event.getStartTime();
            boolean success = event.getFailure() == null;
            String outcome = success ? "SUCCESS" : "FAILED";
            String opType = op.getName() != null ? op.getName() : "unknown";
            String displayName = op.getDisplayName();

            client.getBuildOperationsStub().completeOperation(
                gradle.substrate.v1.CompleteOperationRequest.newBuilder()
                    .setOperationId(id)
                    .setDurationMs(durationMs)
                    .setSuccess(success)
                    .setOutcome(outcome)
                    .build()
            );

            // Update aggregate stats
            totalOperations.incrementAndGet();
            totalDurationMs.addAndGet(durationMs);
            if (!success) {
                failureCount.incrementAndGet();
            }

            countsByType.computeIfAbsent(opType, k -> new AtomicLong(0)).incrementAndGet();
            durationsByType.computeIfAbsent(opType, k -> new AtomicLong(0)).addAndGet(durationMs);

            // Track slowest operations
            slowestOps.compute(displayName, (key, existing) -> {
                if (existing == null || durationMs > existing) {
                    return durationMs;
                }
                return existing;
            });
            if (slowestOps.size() > MAX_SLOW_OPS) {
                slowestOps.entrySet().stream()
                    .min(Map.Entry.comparingByValue())
                    .ifPresent(entry -> slowestOps.remove(entry.getKey()));
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildops] shadow finish failed for {}", op.getDisplayName(), e);
        }
    }

    @Override
    public void progress(OperationIdentifier id, OperationProgressEvent event) {
        // Ignore progress events in shadow mode
    }

    /**
     * Get the total number of operations tracked.
     */
    public long getTotalOperations() {
        return totalOperations.get();
    }

    /**
     * Get the total duration of all operations in milliseconds.
     */
    public long getTotalDurationMs() {
        return totalDurationMs.get();
    }

    /**
     * Get the number of failed operations.
     */
    public long getFailureCount() {
        return failureCount.get();
    }

    /**
     * Get operation counts grouped by type.
     */
    public Map<String, Long> getCountsByType() {
        Map<String, Long> result = new HashMap<>();
        for (Map.Entry<String, AtomicLong> entry : countsByType.entrySet()) {
            result.put(entry.getKey(), entry.getValue().get());
        }
        return result;
    }

    /**
     * Get the slowest operations tracked.
     */
    public Map<String, Long> getSlowestOps() {
        return new HashMap<>(slowestOps);
    }
}
