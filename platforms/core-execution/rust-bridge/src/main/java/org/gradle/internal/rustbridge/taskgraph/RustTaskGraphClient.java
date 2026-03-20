package org.gradle.internal.rustbridge.taskgraph;

import gradle.substrate.v1.RegisterTaskRequest;
import gradle.substrate.v1.RegisterTaskResponse;
import gradle.substrate.v1.ResolveExecutionPlanRequest;
import gradle.substrate.v1.ResolveExecutionPlanResponse;
import gradle.substrate.v1.TaskGraphServiceGrpc;
import org.gradle.internal.rustbridge.SubstrateClient;

import java.util.List;
import java.util.Map;

/**
 * Client for the Rust task graph service.
 * Registers tasks and resolves execution plans (topological ordering).
 */
public class RustTaskGraphClient {

    private final SubstrateClient client;

    public RustTaskGraphClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Register a task with its dependencies in the Rust task graph.
     */
    public boolean registerTask(String taskPath, List<String> dependsOn,
                                boolean shouldExecute, String taskType) {
        if (client.isNoop()) {
            return false;
        }

        try {
            RegisterTaskResponse response = client.getTaskGraphStub()
                .registerTask(RegisterTaskRequest.newBuilder()
                    .setTaskPath(taskPath)
                    .addAllDependsOn(dependsOn)
                    .setShouldExecute(shouldExecute)
                    .setTaskType(taskType)
                    .build());

            return response.getSuccess();
        } catch (Exception e) {
            return false;
        }
    }

    /**
     * Resolve the execution plan (topological sort) from the Rust task graph.
     */
    public ExecutionPlanResult resolveExecutionPlan(String buildId) {
        if (client.isNoop()) {
            return ExecutionPlanResult.error("Substrate client is in no-op mode");
        }

        try {
            ResolveExecutionPlanResponse response = client.getTaskGraphStub()
                .resolveExecutionPlan(ResolveExecutionPlanRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());

            return ExecutionPlanResult.success(
                response.getExecutionOrderList(),
                response.getTotalTasks(),
                response.getReadyToExecute(),
                response.getCriticalPathMs(),
                response.getHasCycles()
            );
        } catch (Exception e) {
            return ExecutionPlanResult.error("Rust task graph resolve failed: " + e.getMessage());
        }
    }

    /**
     * Result of resolving a Rust execution plan.
     */
    public static class ExecutionPlanResult {
        private final boolean success;
        private final List<gradle.substrate.v1.ExecutionNode> executionOrder;
        private final int totalTasks;
        private final int readyToExecute;
        private final long criticalPathMs;
        private final boolean hasCycles;
        private final String errorMessage;

        private ExecutionPlanResult(boolean success,
                                    List<gradle.substrate.v1.ExecutionNode> executionOrder,
                                    int totalTasks, int readyToExecute,
                                    long criticalPathMs, boolean hasCycles,
                                    String errorMessage) {
            this.success = success;
            this.executionOrder = executionOrder;
            this.totalTasks = totalTasks;
            this.readyToExecute = readyToExecute;
            this.criticalPathMs = criticalPathMs;
            this.hasCycles = hasCycles;
            this.errorMessage = errorMessage;
        }

        public static ExecutionPlanResult success(
            List<gradle.substrate.v1.ExecutionNode> executionOrder,
            int totalTasks, int readyToExecute,
            long criticalPathMs, boolean hasCycles
        ) {
            return new ExecutionPlanResult(true, executionOrder, totalTasks,
                readyToExecute, criticalPathMs, hasCycles, null);
        }

        public static ExecutionPlanResult error(String errorMessage) {
            return new ExecutionPlanResult(false, null, 0, 0, 0, false, errorMessage);
        }

        public boolean isSuccess() { return success; }
        public List<gradle.substrate.v1.ExecutionNode> getExecutionOrder() { return executionOrder; }
        public int getTotalTasks() { return totalTasks; }
        public int getReadyToExecute() { return readyToExecute; }
        public long getCriticalPathMs() { return criticalPathMs; }
        public boolean hasCycles() { return hasCycles; }
        public String getErrorMessage() { return errorMessage; }
    }
}
