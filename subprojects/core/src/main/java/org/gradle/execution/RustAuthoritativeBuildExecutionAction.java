/*
 * Copyright 2026 the original author or authors.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
package org.gradle.execution;

import gradle.substrate.v1.BuildPlanTask;

import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.internal.GradleInternal;
import org.gradle.api.internal.project.ProjectInternal;
import org.gradle.execution.plan.FinalizedExecutionPlan;
import org.gradle.execution.plan.LocalTaskNode;
import org.gradle.execution.plan.Node;
import org.gradle.internal.build.ExecutionResult;
import org.gradle.internal.rustbridge.SubstrateException;
import org.gradle.internal.rustbridge.bootstrap.RustBootstrapClient;
import org.gradle.internal.rustbridge.eventstream.BuildIdHolder;
import org.gradle.internal.rustbridge.jvmhost.BuildPlanTaskSelectionSnapshot;
import org.gradle.internal.rustbridge.jvmhost.ProjectModelProviderAdapter;
import org.gradle.internal.rustbridge.taskgraph.RustBuildExecutionClient;
import org.gradle.internal.rustbridge.taskgraph.RustBuildExecutionClient.RunBuildResult;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Replaces the JVM task executor with Rust DAG execution when explicitly enabled.
 *
 * <p>This is intentionally fail-closed: Gradle only skips JVM task execution when
 * Rust reports a complete no-fallback run for exactly the scheduled task count.</p>
 */
public class RustAuthoritativeBuildExecutionAction implements BuildWorkExecutor {
    private static final Logger LOGGER = LoggerFactory.getLogger(RustAuthoritativeBuildExecutionAction.class);

    private final BuildWorkExecutor delegate;
    private final RustBuildExecutionClient buildExecutionClient;
    private final boolean failClosed;
    @Nullable
    private final RustBootstrapClient bootstrapClient;
    @Nullable
    private final BuildPlanTaskSelectionSnapshot taskSelectionSnapshot;

    public RustAuthoritativeBuildExecutionAction(
        BuildWorkExecutor delegate,
        RustBuildExecutionClient buildExecutionClient,
        boolean failClosed
    ) {
        this(delegate, buildExecutionClient, failClosed, null, null);
    }

    public RustAuthoritativeBuildExecutionAction(
        BuildWorkExecutor delegate,
        RustBuildExecutionClient buildExecutionClient,
        boolean failClosed,
        @Nullable RustBootstrapClient bootstrapClient,
        @Nullable BuildPlanTaskSelectionSnapshot taskSelectionSnapshot
    ) {
        this.delegate = delegate;
        this.buildExecutionClient = buildExecutionClient;
        this.failClosed = failClosed;
        this.bootstrapClient = bootstrapClient;
        this.taskSelectionSnapshot = taskSelectionSnapshot;
    }

    @Override
    public ExecutionResult<Void> execute(GradleInternal gradle, FinalizedExecutionPlan plan) {
        int expectedTasks = plan.getContents().getTasks().size();
        if (expectedTasks == 0) {
            return ExecutionResult.succeeded();
        }

        bindAllReferencesOfProject(plan);

        String activeBuildId = BuildIdHolder.getBuildId();
        String buildId = activeBuildId.isEmpty() ? "build" : activeBuildId;
        if (!refreshSelectedBuildPlanShadow(plan, buildId, expectedTasks)) {
            SubstrateException failure = new SubstrateException(
                "Rust authoritative run-build could not refresh the selected build-plan shadow for "
                    + buildId + ": expectedTasks=" + expectedTasks
            );
            if (failClosed) {
                return ExecutionResult.failed(failure);
            }
            LOGGER.warn(
                "[substrate:run-build] Rust build-plan shadow refresh failed; delegating to JVM executor: {}",
                failure.getMessage()
            );
            return delegate.execute(gradle, plan);
        }

        RunBuildResult result = buildExecutionClient.runBuild(
            buildId,
            Runtime.getRuntime().availableProcessors(),
            false
        );

        if (isCompleteNoFallbackRun(result, expectedTasks)) {
            LOGGER.info(
                "[substrate:run-build] Rust executed {} Gradle tasks from {} with JVM fallback disabled; JVM task executor skipped",
                result.getTotalTasks(),
                result.getPlanSource()
            );
            return ExecutionResult.succeeded();
        }

        SubstrateException failure = new SubstrateException(
            "Rust authoritative run-build did not complete the scheduled Gradle work for "
                + buildId + ": " + describeFailure(result, expectedTasks)
        );
        if (failClosed) {
            return ExecutionResult.failed(failure);
        }
        LOGGER.warn(
            "[substrate:run-build] Rust run-build was incomplete; delegating to JVM executor: {}",
            failure.getMessage()
        );
        return delegate.execute(gradle, plan);
    }

    private static boolean isCompleteNoFallbackRun(RunBuildResult result, int expectedTasks) {
        return result.isSuccess()
            && result.getTasksForwardedToJvm() == 0
            && result.getTasksFailed() == 0
            && result.getTotalTasks() == expectedTasks;
    }

    private static String describeFailure(RunBuildResult result, int expectedTasks) {
        String reason = result.getErrorMessage();
        if (reason == null || reason.isEmpty()) {
            reason = result.getFailureMessage();
        }
        if (reason == null || reason.isEmpty()) {
            reason = "status=" + result.getFinalStatus();
        }
        return reason
            + ", expectedTasks=" + expectedTasks
            + ", totalTasks=" + result.getTotalTasks()
            + ", succeeded=" + result.getTasksSucceeded()
            + ", failed=" + result.getTasksFailed()
            + ", jvmForwarded=" + result.getTasksForwardedToJvm();
    }

    private static void bindAllReferencesOfProject(FinalizedExecutionPlan plan) {
        Set<Project> seen = new HashSet<>();
        plan.getContents().getScheduledNodes().visitNodes((nodes, entryNodes) -> {
            for (Node node : nodes) {
                if (node instanceof LocalTaskNode) {
                    ProjectInternal taskProject = node.getOwningProject();
                    if (seen.add(taskProject)) {
                        taskProject.bindAllModelRules();
                    }
                }
            }
        });
    }

    private boolean refreshSelectedBuildPlanShadow(FinalizedExecutionPlan plan, String buildId, int expectedTasks) {
        if (bootstrapClient == null || taskSelectionSnapshot == null) {
            return true;
        }

        BuildPlanTaskSelectionSnapshot.Snapshot earlySnapshot = taskSelectionSnapshot.snapshot();
        ScheduledTaskGraph scheduledTaskGraph = captureScheduledTaskGraph(plan);
        if (scheduledTaskGraph.taskPaths.size() != expectedTasks) {
            LOGGER.warn(
                "[substrate:run-build] selected task snapshot has {} tasks but Gradle scheduled {}; refusing Rust run-build",
                scheduledTaskGraph.taskPaths.size(),
                expectedTasks
            );
            return false;
        }
        List<BuildPlanTask> taskContracts = taskContractsForRust(scheduledTaskGraph, earlySnapshot);

        taskSelectionSnapshot.recordSelectedTasks(
            scheduledTaskGraph.taskPaths,
            scheduledTaskGraph.dependencies,
            scheduledTaskGraph.taskReferences,
            taskContracts
        );
        boolean refreshed = bootstrapClient.refreshBuildPlanShadow(
            buildId,
            taskContracts,
            scheduledTaskGraph.taskReferences,
            "finalized-execution-plan-inline",
            "gradle-finalized-execution-plan-inline"
        );
        if (!refreshed) {
            LOGGER.warn("[substrate:run-build] build-plan shadow refresh failed for {}", buildId);
        }
        return refreshed;
    }

    private static List<BuildPlanTask> taskContractsForRust(
        ScheduledTaskGraph scheduledTaskGraph,
        BuildPlanTaskSelectionSnapshot.Snapshot earlySnapshot
    ) {
        if (canReuseEarlyTaskContracts(scheduledTaskGraph, earlySnapshot)) {
            LOGGER.info(
                "[substrate:run-build] reusing {} task contracts captured at task graph population",
                earlySnapshot.getTaskContracts().size()
            );
            return orderedTaskContracts(earlySnapshot, scheduledTaskGraph);
        }
        if (earlySnapshot.isPopulated()) {
            LOGGER.debug(
                "[substrate:run-build] early task contracts were not reusable; earlyPaths={}, earlyContracts={}, finalizedPaths={}",
                earlySnapshot.getTaskPaths().size(),
                earlySnapshot.getTaskContracts().size(),
                scheduledTaskGraph.taskPaths.size()
            );
        }
        return scheduledTaskGraph.taskContracts;
    }

    private static boolean canReuseEarlyTaskContracts(
        ScheduledTaskGraph scheduledTaskGraph,
        BuildPlanTaskSelectionSnapshot.Snapshot earlySnapshot
    ) {
        if (!earlySnapshot.isPopulated()
            || earlySnapshot.getTaskContracts().isEmpty()
            || earlySnapshot.getTaskContracts().size() != scheduledTaskGraph.taskPaths.size()
            || earlySnapshot.getTaskPaths().size() != scheduledTaskGraph.taskPaths.size()) {
            return false;
        }

        Set<String> earlyTaskPaths = new HashSet<>(earlySnapshot.getTaskPaths());
        if (earlyTaskPaths.size() != scheduledTaskGraph.taskPaths.size()) {
            return false;
        }
        for (String taskPath : scheduledTaskGraph.taskPaths) {
            if (!earlyTaskPaths.contains(taskPath)) {
                return false;
            }
        }
        return true;
    }

    private static List<BuildPlanTask> orderedTaskContracts(
        BuildPlanTaskSelectionSnapshot.Snapshot earlySnapshot,
        ScheduledTaskGraph scheduledTaskGraph
    ) {
        Map<String, BuildPlanTask> contractsByPath = new LinkedHashMap<>();
        List<String> earlyTaskPaths = earlySnapshot.getTaskPaths();
        List<BuildPlanTask> earlyTaskContracts = earlySnapshot.getTaskContracts();
        for (int i = 0; i < earlyTaskPaths.size(); i++) {
            contractsByPath.put(earlyTaskPaths.get(i), earlyTaskContracts.get(i));
        }
        List<BuildPlanTask> orderedContracts = new ArrayList<>();
        for (String taskPath : scheduledTaskGraph.taskPaths) {
            orderedContracts.add(contractsByPath.get(taskPath));
        }
        return orderedContracts;
    }

    private static ScheduledTaskGraph captureScheduledTaskGraph(FinalizedExecutionPlan plan) {
        List<String> taskPaths = new ArrayList<>();
        List<Task> taskReferences = new ArrayList<>();
        List<BuildPlanTask> taskContracts = new ArrayList<>();
        Map<String, List<String>> dependencies = new LinkedHashMap<>();
        Map<Node, String> nodePaths = new LinkedHashMap<>();
        Map<Node, Task> nodeTasks = new LinkedHashMap<>();

        plan.getContents().getScheduledNodes().visitNodes((nodes, entryNodes) -> {
            for (Node node : nodes) {
                if (node instanceof LocalTaskNode) {
                    Task task = ((LocalTaskNode) node).getTask();
                    String taskPath = task.getPath();
                    taskPaths.add(taskPath);
                    taskReferences.add(task);
                    nodePaths.put(node, taskPath);
                    nodeTasks.put(node, task);
                }
            }
            for (Map.Entry<Node, String> entry : nodePaths.entrySet()) {
                List<String> taskDependencies = new ArrayList<>();
                for (Node dependency : entry.getKey().getDependencySuccessors()) {
                    String dependencyPath = nodePaths.get(dependency);
                    if (dependencyPath != null) {
                        taskDependencies.add(dependencyPath);
                    }
                }
                dependencies.put(entry.getValue(), taskDependencies);
                Task task = nodeTasks.get(entry.getKey());
                if (task != null) {
                    taskContracts.add(ProjectModelProviderAdapter.toBuildPlanTask(task, taskDependencies));
                }
            }
        });

        return new ScheduledTaskGraph(taskPaths, taskReferences, dependencies, taskContracts);
    }

    private static final class ScheduledTaskGraph {
        private final List<String> taskPaths;
        private final List<Task> taskReferences;
        private final Map<String, List<String>> dependencies;
        private final List<BuildPlanTask> taskContracts;

        private ScheduledTaskGraph(
            List<String> taskPaths,
            List<Task> taskReferences,
            Map<String, List<String>> dependencies,
            List<BuildPlanTask> taskContracts
        ) {
            this.taskPaths = taskPaths;
            this.taskReferences = taskReferences;
            this.dependencies = dependencies;
            this.taskContracts = taskContracts;
        }
    }
}
