package org.gradle.internal.rustbridge.eventstream;

import gradle.substrate.v1.BuildEventStreamServiceGrpc;
import gradle.substrate.v1.GetEventLogRequest;
import gradle.substrate.v1.GetEventLogResponse;
import gradle.substrate.v1.SendBuildEventRequest;
import gradle.substrate.v1.SendBuildEventResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;
import java.util.Map;

/**
 * Client for the Rust build event stream service.
 * Sends and queries build events via gRPC.
 */
public class RustBuildEventStreamClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildEventStreamClient.class);

    private final SubstrateClient client;

    public RustBuildEventStreamClient(SubstrateClient client) {
        this.client = client;
    }

    public boolean sendBuildEvent(String buildId, String eventType, String eventId,
                                   Map<String, String> properties, String displayName, String parentId) {
        if (client.isNoop()) {
            return false;
        }

        try {
            SendBuildEventResponse response = client.getBuildEventStreamStub()
                .sendBuildEvent(SendBuildEventRequest.newBuilder()
                    .setBuildId(buildId)
                    .setEventType(eventType)
                    .setEventId(eventId)
                    .putAllProperties(properties)
                    .setDisplayName(displayName != null ? displayName : "")
                    .setParentId(parentId != null ? parentId : "")
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:eventstream] send build event failed", e);
            return false;
        }
    }

    public GetEventLogResponse getEventLog(String buildId, long sinceTimestampMs, int maxEvents) {
        if (client.isNoop()) {
            return GetEventLogResponse.getDefaultInstance();
        }

        try {
            return client.getBuildEventStreamStub()
                .getEventLog(GetEventLogRequest.newBuilder()
                    .setBuildId(buildId)
                    .setSinceTimestampMs(sinceTimestampMs)
                    .setMaxEvents(maxEvents)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:eventstream] get event log failed", e);
            return GetEventLogResponse.getDefaultInstance();
        }
    }
}
