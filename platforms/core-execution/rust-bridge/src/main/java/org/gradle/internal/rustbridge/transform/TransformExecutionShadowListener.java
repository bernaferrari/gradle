package org.gradle.internal.rustbridge.transform;

import org.gradle.api.Describable;
import org.gradle.api.internal.artifacts.transform.TransformExecutionListener;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.concurrent.atomic.AtomicLong;

/**
 * A {@link TransformExecutionListener} that shadows artifact transform executions
 * to the Rust substrate. Fire-and-forget: never affects build correctness.
 */
public class TransformExecutionShadowListener implements TransformExecutionListener {

    private static final Logger LOGGER = Logging.getLogger(TransformExecutionShadowListener.class);

    private final SubstrateClient client;
    private final AtomicLong transformCount = new AtomicLong(0);

    public TransformExecutionShadowListener(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public void beforeTransformExecution(Describable transform, Describable subject) {
        if (client.isNoop()) {
            return;
        }

        try {
            String transformName = transform.getDisplayName();
            String subjectName = subject.getDisplayName();

            client.getBuildOperationsStub().startOperation(
                gradle.substrate.v1.StartOperationRequest.newBuilder()
                    .setOperationId("transform:" + transformCount.incrementAndGet())
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
            client.getBuildOperationsStub().completeOperation(
                gradle.substrate.v1.CompleteOperationRequest.newBuilder()
                    .setOperationId("transform:" + transformCount.get())
                    .setDurationMs(0)
                    .setSuccess(true)
                    .setOutcome("SUCCESS")
                    .build()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:transform] shadow after failed for {}", transform.getDisplayName(), e);
        }
    }
}
