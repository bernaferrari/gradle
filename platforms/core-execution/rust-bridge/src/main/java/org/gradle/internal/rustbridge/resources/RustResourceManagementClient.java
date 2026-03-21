package org.gradle.internal.rustbridge.resources;

import gradle.substrate.v1.GetResourceLimitsRequest;
import gradle.substrate.v1.GetResourceLimitsResponse;
import gradle.substrate.v1.GetResourceUsageRequest;
import gradle.substrate.v1.GetResourceUsageResponse;
import gradle.substrate.v1.ReleaseResourcesRequest;
import gradle.substrate.v1.ReleaseResourcesResponse;
import gradle.substrate.v1.ReserveResourcesRequest;
import gradle.substrate.v1.ReserveResourcesResponse;
import gradle.substrate.v1.ResourceManagementServiceGrpc;
import gradle.substrate.v1.ResourceRequest;
import gradle.substrate.v1.ResourceLimit;
import gradle.substrate.v1.SetResourceLimitsRequest;
import gradle.substrate.v1.SetResourceLimitsResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust resource management service.
 * Manages resource reservations and limits via gRPC.
 */
public class RustResourceManagementClient {

    private static final Logger LOGGER = Logging.getLogger(RustResourceManagementClient.class);

    private final SubstrateClient client;

    public RustResourceManagementClient(SubstrateClient client) {
        this.client = client;
    }

    public ReserveResourcesResponse reserveResources(String buildId, List<ResourceRequest> resources,
                                                      long timeoutMs) {
        if (client.isNoop()) {
            return ReserveResourcesResponse.getDefaultInstance();
        }

        try {
            return client.getResourceManagementStub()
                .reserveResources(ReserveResourcesRequest.newBuilder()
                    .setBuildId(buildId)
                    .addAllResources(resources)
                    .setTimeoutMs(timeoutMs)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:resources] reserve resources failed", e);
            return ReserveResourcesResponse.getDefaultInstance();
        }
    }

    public boolean releaseResources(String reservationId) {
        if (client.isNoop()) {
            return false;
        }

        try {
            ReleaseResourcesResponse response = client.getResourceManagementStub()
                .releaseResources(ReleaseResourcesRequest.newBuilder()
                    .setReservationId(reservationId)
                    .build());
            return response.getReleased();
        } catch (Exception e) {
            LOGGER.debug("[substrate:resources] release resources failed", e);
            return false;
        }
    }

    public GetResourceUsageResponse getResourceUsage(String buildId) {
        if (client.isNoop()) {
            return GetResourceUsageResponse.getDefaultInstance();
        }

        try {
            return client.getResourceManagementStub()
                .getResourceUsage(GetResourceUsageRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:resources] get resource usage failed", e);
            return GetResourceUsageResponse.getDefaultInstance();
        }
    }

    public GetResourceLimitsResponse getResourceLimits(String buildId) {
        if (client.isNoop()) {
            return GetResourceLimitsResponse.getDefaultInstance();
        }

        try {
            return client.getResourceManagementStub()
                .getResourceLimits(GetResourceLimitsRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:resources] get resource limits failed", e);
            return GetResourceLimitsResponse.getDefaultInstance();
        }
    }

    public boolean setResourceLimits(String buildId, List<ResourceLimit> limits) {
        if (client.isNoop()) {
            return false;
        }

        try {
            SetResourceLimitsResponse response = client.getResourceManagementStub()
                .setResourceLimits(SetResourceLimitsRequest.newBuilder()
                    .setBuildId(buildId)
                    .addAllLimits(limits)
                    .build());
            return response.getApplied();
        } catch (Exception e) {
            LOGGER.debug("[substrate:resources] set resource limits failed", e);
            return false;
        }
    }
}
