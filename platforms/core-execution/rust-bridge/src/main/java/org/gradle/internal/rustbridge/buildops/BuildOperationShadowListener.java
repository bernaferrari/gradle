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

import java.util.Collections;

/**
 * A {@link BuildOperationListener} that shadows all build operations to the Rust substrate.
 * Fire-and-forget: never affects build correctness, only sends operation metadata to Rust.
 */
public class BuildOperationShadowListener implements BuildOperationListener {

    private static final Logger LOGGER = Logging.getLogger(BuildOperationShadowListener.class);

    private final SubstrateClient client;

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

            client.getBuildOperationsStub().startOperation(
                gradle.substrate.v1.StartOperationRequest.newBuilder()
                    .setOperationId(id)
                    .setDisplayName(op.getDisplayName())
                    .setOperationType(op.getName())
                    .setParentId(parentId)
                    .setStartTimeMs(event.getStartTime())
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

            client.getBuildOperationsStub().completeOperation(
                gradle.substrate.v1.CompleteOperationRequest.newBuilder()
                    .setOperationId(id)
                    .setDurationMs(durationMs)
                    .setSuccess(success)
                    .setOutcome(outcome)
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildops] shadow finish failed for {}", op.getDisplayName(), e);
        }
    }

    @Override
    public void progress(OperationIdentifier id, OperationProgressEvent event) {
        // Ignore progress events in shadow mode
    }
}
