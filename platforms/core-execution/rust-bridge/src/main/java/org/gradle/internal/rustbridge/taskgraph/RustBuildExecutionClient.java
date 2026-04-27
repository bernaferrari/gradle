package org.gradle.internal.rustbridge.taskgraph;

import gradle.substrate.v1.RunBuildRequest;
import gradle.substrate.v1.RunBuildResponse;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;

import java.util.Collections;
import java.util.List;

/**
 * Client for Rust authoritative build execution over the DAG executor service.
 */
@ServiceScope(Scope.Build.class)
public class RustBuildExecutionClient {
    private final SubstrateClient client;

    public RustBuildExecutionClient(SubstrateClient client) {
        this.client = client;
    }

    public RunBuildResult runBuild(String buildId, int maxParallelism, boolean allowJvmForwarding) {
        if (client.isNoop()) {
            return RunBuildResult.error("Substrate client is in no-op mode");
        }
        try {
            RunBuildResponse response = client.getDagExecutorStub()
                .runBuild(RunBuildRequest.newBuilder()
                    .setBuildId(buildId)
                    .setMaxParallelism(Math.max(1, maxParallelism))
                    .setAllowJvmForwarding(allowJvmForwarding)
                    .build());
            return RunBuildResult.fromResponse(response);
        } catch (Exception e) {
            return RunBuildResult.error("Rust run-build failed: " + e.getMessage());
        }
    }

    public static class RunBuildResult {
        private final boolean success;
        private final String finalStatus;
        private final int totalTasks;
        private final int tasksSucceeded;
        private final int tasksFailed;
        private final int tasksForwardedToJvm;
        private final String planSource;
        private final String failureMessage;
        private final String errorMessage;
        private final List<gradle.substrate.v1.TaskExecutionDetail> taskDetails;

        private RunBuildResult(
            boolean success,
            String finalStatus,
            int totalTasks,
            int tasksSucceeded,
            int tasksFailed,
            int tasksForwardedToJvm,
            String planSource,
            String failureMessage,
            String errorMessage,
            List<gradle.substrate.v1.TaskExecutionDetail> taskDetails
        ) {
            this.success = success;
            this.finalStatus = finalStatus;
            this.totalTasks = totalTasks;
            this.tasksSucceeded = tasksSucceeded;
            this.tasksFailed = tasksFailed;
            this.tasksForwardedToJvm = tasksForwardedToJvm;
            this.planSource = planSource;
            this.failureMessage = failureMessage;
            this.errorMessage = errorMessage;
            this.taskDetails = taskDetails;
        }

        public static RunBuildResult fromResponse(RunBuildResponse response) {
            boolean completed = "COMPLETED".equals(response.getFinalStatus());
            return new RunBuildResult(
                completed && response.getTasksFailed() == 0,
                response.getFinalStatus(),
                response.getTotalTasks(),
                response.getTasksSucceeded(),
                response.getTasksFailed(),
                response.getTasksForwardedToJvm(),
                response.getPlanSource(),
                response.getFailureMessage(),
                "",
                response.getTaskDetailsList()
            );
        }

        public static RunBuildResult success(String planSource, int totalTasks, int tasksSucceeded) {
            return new RunBuildResult(
                true,
                "COMPLETED",
                totalTasks,
                tasksSucceeded,
                0,
                0,
                planSource,
                "",
                "",
                Collections.emptyList()
            );
        }

        public static RunBuildResult error(String errorMessage) {
            return new RunBuildResult(
                false,
                "FAILED",
                0,
                0,
                0,
                0,
                "error",
                "",
                errorMessage,
                Collections.emptyList()
            );
        }

        public boolean isSuccess() {
            return success;
        }

        public String getFinalStatus() {
            return finalStatus;
        }

        public int getTotalTasks() {
            return totalTasks;
        }

        public int getTasksSucceeded() {
            return tasksSucceeded;
        }

        public int getTasksFailed() {
            return tasksFailed;
        }

        public int getTasksForwardedToJvm() {
            return tasksForwardedToJvm;
        }

        public String getPlanSource() {
            return planSource;
        }

        public String getFailureMessage() {
            return failureMessage;
        }

        public String getErrorMessage() {
            return errorMessage;
        }

        public List<gradle.substrate.v1.TaskExecutionDetail> getTaskDetails() {
            return taskDetails;
        }
    }
}
