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
import org.gradle.internal.rustbridge.SubstrateException;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust build result service.
 * Records and queries task/build results via gRPC.
 */
@ServiceScope(Scope.Build.class)
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
            throw unavailable("report task result");
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
            throw failed("report task result", e);
        }
    }

    public boolean reportBuildFailure(String buildId, String failureType, String failureMessage,
                                       List<String> failedTaskPaths) {
        if (client.isNoop()) {
            throw unavailable("report build failure");
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
            throw failed("report build failure", e);
        }
    }

    public GetBuildResultResponse getBuildResult(String buildId) {
        if (client.isNoop()) {
            throw unavailable("get build result");
        }

        try {
            return client.getBuildResultStub()
                .getBuildResult(GetBuildResultRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildresult] get build result failed", e);
            throw failed("get build result", e);
        }
    }

    public GetTaskSummaryResponse getTaskSummary(String buildId) {
        if (client.isNoop()) {
            throw unavailable("get task summary");
        }

        try {
            return client.getBuildResultStub()
                .getTaskSummary(GetTaskSummaryRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildresult] get task summary failed", e);
            throw failed("get task summary", e);
        }
    }

    private SubstrateException unavailable(String operation) {
        return new SubstrateException("Rust build result service is unavailable for " + operation + ": " + client.getNoopReason());
    }

    private SubstrateException failed(String operation, Exception cause) {
        if (cause instanceof SubstrateException) {
            return (SubstrateException) cause;
        }
        return new SubstrateException("Rust build result service failed to " + operation, cause);
    }
}
