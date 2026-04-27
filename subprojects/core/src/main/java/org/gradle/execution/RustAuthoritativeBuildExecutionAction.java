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

import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.internal.GradleInternal;
import org.gradle.api.internal.project.ProjectInternal;
import org.gradle.execution.plan.FinalizedExecutionPlan;
import org.gradle.execution.plan.LocalTaskNode;
import org.gradle.execution.plan.Node;
import org.gradle.internal.build.ExecutionResult;
import org.gradle.internal.rustbridge.SubstrateException;
import org.gradle.internal.rustbridge.eventstream.BuildIdHolder;
import org.gradle.internal.rustbridge.taskgraph.RustBuildExecutionClient;
import org.gradle.internal.rustbridge.taskgraph.RustBuildExecutionClient.RunBuildResult;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.HashSet;
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

    public RustAuthoritativeBuildExecutionAction(
        BuildWorkExecutor delegate,
        RustBuildExecutionClient buildExecutionClient,
        boolean failClosed
    ) {
        this.delegate = delegate;
        this.buildExecutionClient = buildExecutionClient;
        this.failClosed = failClosed;
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
}
