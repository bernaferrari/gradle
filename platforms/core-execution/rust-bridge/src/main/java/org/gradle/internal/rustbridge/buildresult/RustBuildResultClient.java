package org.gradle.internal.rustbridge.buildresult;

import gradle.substrate.v1.BuildResultServiceGrpc;
import gradle.substrate.v1.GetBuildResultRequest;
import gradle.substrate.v1.GetBuildResultResponse;
import gradle.substrate.v1.GetTaskSummaryRequest;
import gradle.substrate.v1.GetTaskSummaryResponse;
import gradle.substrate.v1.ReportBuildFailureRequest;
import gradle.substrate.v1.ReportBuildFailureResponse;
import gradle.substrate.v1.ReportTaskResultRequest;
import gradle.substrate.v1.ReportTaskResultResponse;
import gradle.substrate.v1.TaskResult;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust build result service.
 * Records and queries task/build results via gRPC.
 */
public class RustBuildResultClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildResultClient.class);

    private final SubstrateClient client;

    public RustBuildResultClient(SubstrateClient client) {
        this.client = client;
    }

    public boolean reportTaskResult(String buildId, String taskPath, String outcome,
                                     long durationMs, boolean didWork, String cacheKey,
                                     long startTimeMs, long endTimeMs, String failureMessage) {
        if (client.isNoop()) {
            return false;
        }

        try {
            TaskResult.Builder resultBuilder = TaskResult.newBuilder()
                .setTaskPath(taskPath)
                .setOutcome(outcome)
                .setDurationMs(durationMs)
                .setDidWork(didWork)
                .setCacheKey(cacheKey != null ? cacheKey : "")
                .setStartTimeMs(startTimeMs)
                .setEndTimeMs(endTimeMs)
                .setFailureMessage(failureMessage != null ? failureMessage : "");

            ReportTaskResultResponse response = client.getBuildResultStub()
                .reportTaskResult(ReportTaskResultRequest.newBuilder()
                    .setBuildId(buildId)
                    .setResult(resultBuilder.build())
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildresult] report task result failed for {}", taskPath, e);
            return false;
        }
    }

    public boolean reportBuildFailure(String buildId, String failureType, String failureMessage,
                                       List<String> failedTaskPaths) {
        if (client.isNoop()) {
            return false;
        }

        try {
            ReportBuildFailureResponse response = client.getBuildResultStub()
                .reportBuildFailure(ReportBuildFailureRequest.newBuilder()
                    .setBuildId(buildId)
                    .setFailureType(failureType)
                    .setFailureMessage(failureMessage)
                    .addAllFailedTaskPaths(failedTaskPaths)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildresult] report build failure failed", e);
            return false;
        }
    }

    public GetBuildResultResponse getBuildResult(String buildId) {
        if (client.isNoop()) {
            return GetBuildResultResponse.getDefaultInstance();
        }

        try {
            return client.getBuildResultStub()
                .getBuildResult(GetBuildResultRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildresult] get build result failed", e);
            return GetBuildResultResponse.getDefaultInstance();
        }
    }

    public GetTaskSummaryResponse getTaskSummary(String buildId) {
        if (client.isNoop()) {
            return GetTaskSummaryResponse.getDefaultInstance();
        }

        try {
            return client.getBuildResultStub()
                .getTaskSummary(GetTaskSummaryRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildresult] get task summary failed", e);
            return GetTaskSummaryResponse.getDefaultInstance();
        }
    }
}
