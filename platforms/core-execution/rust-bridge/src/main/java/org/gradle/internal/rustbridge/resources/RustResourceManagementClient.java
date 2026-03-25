package org.gradle.internal.rustbridge.resources;

import gradle.substrate.v1.GetResourceUsageRequest;
import gradle.substrate.v1.GetResourceUsageResponse;
import gradle.substrate.v1.ReserveResourcesRequest;
import gradle.substrate.v1.ReserveResourcesResponse;
import gradle.substrate.v1.ResourceRequest;
import gradle.substrate.v1.ResourceManagementServiceGrpc;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

/**
 * Client for the Rust resource management service.
 * Manages build resource reservations and queries usage via gRPC.
 */
public class RustResourceManagementClient {

    private static final Logger LOGGER = Logging.getLogger(RustResourceManagementClient.class);

    private final SubstrateClient client;

    public RustResourceManagementClient(SubstrateClient client) {
        this.client = client;
    }

    public String getResourceUsage(String buildId) {
        if (client.isNoop()) {
            return "";
        }

        try {
            GetResourceUsageResponse response = client.getResourceManagementStub()
                .getResourceUsage(GetResourceUsageRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
            return response.toString();
        } catch (Exception e) {
            LOGGER.debug("[substrate:resources] get resource usage failed", e);
            return "";
        }
    }

    public boolean reserveResources(String buildId, long maxMemoryBytes, int maxCpuCores) {
        if (client.isNoop()) {
            return false;
        }

        try {
            ReserveResourcesResponse response = client.getResourceManagementStub()
                .reserveResources(ReserveResourcesRequest.newBuilder()
                    .setBuildId(buildId)
                    .addResources(ResourceRequest.newBuilder()
                        .setResourceType("memory_mb")
                        .setAmount(Math.max(1L, maxMemoryBytes / (1024L * 1024L)))
                        .setRequesterId("gradle-rust-bridge")
                        .build())
                    .addResources(ResourceRequest.newBuilder()
                        .setResourceType("cpu_cores")
                        .setAmount(Math.max(1, maxCpuCores))
                        .setRequesterId("gradle-rust-bridge")
                        .build())
                    .setTimeoutMs(0L)
                    .build());
            return response.getGranted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:resources] reserve resources failed", e);
            return false;
        }
    }
}
