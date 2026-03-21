package org.gradle.internal.rustbridge.console;

import gradle.substrate.v1.ConsoleServiceGrpc;
import gradle.substrate.v1.LogMessageRequest;
import gradle.substrate.v1.LogMessageResponse;
import gradle.substrate.v1.SetBuildDescriptionRequest;
import gradle.substrate.v1.SetBuildDescriptionResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

/**
 * Client for the Rust console service.
 * Logs messages and sets build descriptions via gRPC.
 */
public class RustConsoleClient {

    private static final Logger LOGGER = Logging.getLogger(RustConsoleClient.class);

    private final SubstrateClient client;

    public RustConsoleClient(SubstrateClient client) {
        this.client = client;
    }

    public boolean logMessage(String buildId, String level, String category,
                               String message, String throwable) {
        if (client.isNoop()) {
            return false;
        }

        try {
            LogMessageResponse response = client.getConsoleStub()
                .logMessage(LogMessageRequest.newBuilder()
                    .setBuildId(buildId)
                    .setLevel(level)
                    .setCategory(category != null ? category : "")
                    .setMessage(message)
                    .setThrowable(throwable != null ? throwable : "")
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:console] log message failed", e);
            return false;
        }
    }

    public boolean setBuildDescription(String buildId, String description) {
        if (client.isNoop()) {
            return false;
        }

        try {
            SetBuildDescriptionResponse response = client.getConsoleStub()
                .setBuildDescription(SetBuildDescriptionRequest.newBuilder()
                    .setBuildId(buildId)
                    .setDescription(description)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:console] set build description failed", e);
            return false;
        }
    }
}
