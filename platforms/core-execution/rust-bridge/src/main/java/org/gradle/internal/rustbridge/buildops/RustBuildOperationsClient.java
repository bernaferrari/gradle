package org.gradle.internal.rustbridge.buildops;

import gradle.substrate.v1.BuildOperationsServiceGrpc;
import gradle.substrate.v1.BuildEvent;
import gradle.substrate.v1.CompleteOperationRequest;
import gradle.substrate.v1.CompleteOperationResponse;
import gradle.substrate.v1.GetBuildSummaryRequest;
import gradle.substrate.v1.GetBuildSummaryResponse;
import gradle.substrate.v1.ReportProgressRequest;
import gradle.substrate.v1.ReportProgressResponse;
import gradle.substrate.v1.StartOperationRequest;
import gradle.substrate.v1.StartOperationResponse;
import gradle.substrate.v1.StreamEventsRequest;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.Collections;
import java.util.Iterator;
import java.util.List;
import java.util.Map;

/**
 * Client for the Rust build operations service.
 * Tracks build operation lifecycle (start/complete/progress) via gRPC.
 */
public class RustBuildOperationsClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildOperationsClient.class);

    private final SubstrateClient client;

    public RustBuildOperationsClient(SubstrateClient client) {
        this.client = client;
    }

    public boolean startOperation(String operationId, String displayName,
                                   String operationType, String parentId, Map<String, String> metadata) {
        if (client.isNoop()) {
            return false;
        }

        try {
            StartOperationRequest.Builder builder = StartOperationRequest.newBuilder()
                .setOperationId(operationId)
                .setDisplayName(displayName)
                .setOperationType(operationType)
                .setParentId(parentId != null ? parentId : "")
                .setStartTimeMs(System.currentTimeMillis())
                .putAllMetadata(metadata);

            StartOperationResponse response = client.getBuildOperationsStub()
                .startOperation(builder.build());
            return response.getSuccess();
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildops] start operation failed for {}", operationId, e);
            return false;
        }
    }

    public boolean completeOperation(String operationId, long durationMs,
                                      boolean success, String outcome) {
        if (client.isNoop()) {
            return false;
        }

        try {
            CompleteOperationResponse response = client.getBuildOperationsStub()
                .completeOperation(CompleteOperationRequest.newBuilder()
                    .setOperationId(operationId)
                    .setDurationMs(durationMs)
                    .setSuccess(success)
                    .setOutcome(outcome != null ? outcome : "")
                    .build());
            return response.getSuccess();
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildops] complete operation failed for {}", operationId, e);
            return false;
        }
    }

    public boolean reportProgress(String operationId, String message,
                                   float progress, long elapsedMs) {
        if (client.isNoop()) {
            return false;
        }

        try {
            ReportProgressResponse response = client.getBuildOperationsStub()
                .reportProgress(ReportProgressRequest.newBuilder()
                    .setOperationId(operationId)
                    .setMessage(message)
                    .setProgress(progress)
                    .setElapsedMs(elapsedMs)
                    .build());
            return response.getAcknowledged();
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildops] report progress failed for {}", operationId, e);
            return false;
        }
    }

    public GetBuildSummaryResponse getBuildSummary() {
        if (client.isNoop()) {
            return GetBuildSummaryResponse.getDefaultInstance();
        }

        try {
            return client.getBuildOperationsStub()
                .getBuildSummary(GetBuildSummaryRequest.newBuilder().build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildops] get build summary failed", e);
            return GetBuildSummaryResponse.getDefaultInstance();
        }
    }

    /**
     * Stream build events for a build. Returns an empty iterator if substrate is disabled.
     */
    public Iterator<BuildEvent> streamEvents(String buildId) {
        if (client.isNoop()) {
            return Collections.emptyIterator();
        }

        try {
            return client.getBuildOperationsStub()
                .streamEvents(StreamEventsRequest.newBuilder()
                    .setBuildId(buildId != null ? buildId : "")
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildops] stream events failed for build {}", buildId, e);
            return Collections.emptyIterator();
        }
    }
}
