package org.gradle.internal.rustbridge.comparison;

import gradle.substrate.v1.BuildComparisonServiceGrpc;
import gradle.substrate.v1.BuildDataSnapshot;
import gradle.substrate.v1.GetComparisonResultRequest;
import gradle.substrate.v1.GetComparisonResultResponse;
import gradle.substrate.v1.RecordBuildDataRequest;
import gradle.substrate.v1.RecordBuildDataResponse;
import gradle.substrate.v1.StartComparisonRequest;
import gradle.substrate.v1.StartComparisonResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;
import java.util.Map;

/**
 * Client for the Rust build comparison service.
 * Compares build results between baseline and candidate builds via gRPC.
 */
public class RustBuildComparisonClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildComparisonClient.class);

    private final SubstrateClient client;

    public RustBuildComparisonClient(SubstrateClient client) {
        this.client = client;
    }

    public StartComparisonResponse startComparison(String baselineBuildId, String candidateBuildId) {
        if (client.isNoop()) {
            return StartComparisonResponse.getDefaultInstance();
        }

        try {
            return client.getBuildComparisonStub()
                .startComparison(StartComparisonRequest.newBuilder()
                    .setBaselineBuildId(baselineBuildId)
                    .setCandidateBuildId(candidateBuildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:comparison] start comparison failed", e);
            return StartComparisonResponse.getDefaultInstance();
        }
    }

    public boolean recordBuildData(String buildId, long startTimeMs, long endTimeMs,
                                    Map<String, Long> taskDurations, Map<String, String> taskOutcomes,
                                    List<String> taskOrder, String rootDir) {
        if (client.isNoop()) {
            return false;
        }

        try {
            BuildDataSnapshot snapshot = BuildDataSnapshot.newBuilder()
                .setBuildId(buildId)
                .setStartTimeMs(startTimeMs)
                .setEndTimeMs(endTimeMs)
                .putAllTaskDurations(taskDurations)
                .putAllTaskOutcomes(taskOutcomes)
                .addAllTaskOrder(taskOrder)
                .setRootDir(rootDir)
                .build();

            RecordBuildDataResponse response = client.getBuildComparisonStub()
                .recordBuildData(RecordBuildDataRequest.newBuilder()
                    .setSnapshot(snapshot)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:comparison] record build data failed", e);
            return false;
        }
    }

    public GetComparisonResultResponse getComparisonResult(String comparisonId) {
        if (client.isNoop()) {
            return GetComparisonResultResponse.getDefaultInstance();
        }

        try {
            return client.getBuildComparisonStub()
                .getComparisonResult(GetComparisonResultRequest.newBuilder()
                    .setComparisonId(comparisonId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:comparison] get comparison result failed", e);
            return GetComparisonResultResponse.getDefaultInstance();
        }
    }
}
