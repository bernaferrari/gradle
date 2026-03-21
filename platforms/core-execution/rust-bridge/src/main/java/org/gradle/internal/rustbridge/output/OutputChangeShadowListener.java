package org.gradle.internal.rustbridge.output;

import org.gradle.api.logging.Logging;
import org.gradle.internal.execution.OutputChangeListener;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicLong;

/**
 * An {@link OutputChangeListener} that shadows output cache invalidation events
 * to the Rust substrate. Records which outputs were invalidated and when,
 * enabling Rust-side analytics of cache invalidation patterns.
 */
public class OutputChangeShadowListener implements OutputChangeListener {

    private static final Logger LOGGER = Logging.getLogger(OutputChangeShadowListener.class);

    private final SubstrateClient client;

    private final AtomicLong invalidationCount = new AtomicLong(0);
    private final AtomicLong totalPathsInvalidated = new AtomicLong(0);

    public OutputChangeShadowListener(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public void invalidateCachesFor(Iterable<String> affectedOutputPaths) {
        if (client.isNoop()) {
            return;
        }

        try {
            List<String> paths = new ArrayList<>();
            for (String path : affectedOutputPaths) {
                paths.add(path);
            }

            int pathCount = paths.size();
            invalidationCount.incrementAndGet();
            totalPathsInvalidated.addAndGet(pathCount);

            // Record the invalidation as a build operation for Rust analytics
            long opId = invalidationCount.get();
            client.getBuildOperationsStub().startOperation(
                gradle.substrate.v1.StartOperationRequest.newBuilder()
                    .setOperationId("output-invalidate:" + opId)
                    .setDisplayName("Output cache invalidation (" + pathCount + " paths)")
                    .setOperationType("OUTPUT_INVALIDATION")
                    .setStartTimeMs(System.currentTimeMillis())
                    .putMetadata("path_count", String.valueOf(pathCount))
                    .build()
            );

            client.getBuildOperationsStub().completeOperation(
                gradle.substrate.v1.CompleteOperationRequest.newBuilder()
                    .setOperationId("output-invalidate:" + opId)
                    .setDurationMs(0)
                    .setSuccess(true)
                    .setOutcome("INVALIDATED")
                    .build()
            );

            LOGGER.debug(
                "[substrate:output] shadow: {} paths invalidated (total: {} invalidations, {} paths)",
                pathCount, invalidationCount.get(), totalPathsInvalidated.get()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:output] shadow invalidate failed", e);
        }
    }

    /**
     * Get the total number of invalidation events.
     */
    public long getInvalidationCount() {
        return invalidationCount.get();
    }

    /**
     * Get the total number of output paths invalidated.
     */
    public long getTotalPathsInvalidated() {
        return totalPathsInvalidated.get();
    }
}
