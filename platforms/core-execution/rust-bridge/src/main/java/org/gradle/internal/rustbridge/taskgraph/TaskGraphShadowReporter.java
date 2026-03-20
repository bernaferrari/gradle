package org.gradle.internal.rustbridge.taskgraph;

import org.gradle.api.logging.Logging;
import org.gradle.api.tasks.TaskExecutionGraph;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.List;
import java.util.Set;

/**
 * Hooks into {@link TaskExecutionGraph#whenReady()} to shadow-compare
 * the Java task execution ordering with the Rust task graph service.
 *
 * <p>Registers all tasks with the Rust service, resolves its execution plan,
 * and compares the topological ordering with Gradle's Java resolver.</p>
 */
public class TaskGraphShadowReporter {

    private static final Logger LOGGER = Logging.getLogger(TaskGraphShadowReporter.class);

    private final RustTaskGraphClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public TaskGraphShadowReporter(
        RustTaskGraphClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Shadow-compare the task graph after it is ready.
     *
     * @param taskPaths ordered list of task paths from the Java execution graph
     * @param taskDependencies map of task path to its dependency task paths
     * @param buildId unique build identifier
     */
    public void compareExecutionGraph(
        List<String> taskPaths,
        java.util.Map<String, List<String>> taskDependencies,
        String buildId
    ) {
        if (rustClient == null || taskPaths.isEmpty()) {
            return;
        }

        try {
            // Register all tasks with Rust
            for (String taskPath : taskPaths) {
                List<String> deps = taskDependencies.getOrDefault(taskPath, new ArrayList<>());
                rustClient.registerTask(taskPath, deps, true, "Task");
            }

            // Resolve Rust execution plan
            RustTaskGraphClient.ExecutionPlanResult rustResult =
                rustClient.resolveExecutionPlan(buildId);

            if (!rustResult.isSuccess()) {
                LOGGER.debug("[substrate:taskgraph] Rust resolve failed: {}",
                    rustResult.getErrorMessage());
                return;
            }

            if (rustResult.hasCycles()) {
                LOGGER.warn("[substrate:taskgraph] Rust detected cycles that Java did not");
                return;
            }

            // Compare task counts
            if (rustResult.getTotalTasks() != taskPaths.size()) {
                LOGGER.warn("[substrate:taskgraph] Task count mismatch: java={}, rust={}",
                    taskPaths.size(), rustResult.getTotalTasks());
                return;
            }

            // Compare execution order
            List<String> rustOrder = new ArrayList<>();
            for (gradle.substrate.v1.ExecutionNode node : rustResult.getExecutionOrder()) {
                rustOrder.add(node.getTaskPath());
            }

            if (rustOrder.equals(taskPaths)) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:taskgraph] shadow OK: {} tasks, order matches",
                    taskPaths.size());
            } else {
                // Order may differ if Java uses a different tie-breaking strategy.
                // Check that the ordering is still topologically valid relative to Java.
                boolean isValid = validateTopologicalOrder(rustOrder, taskDependencies);
                if (isValid) {
                    mismatchReporter.reportMatch();
                    LOGGER.debug("[substrate:taskgraph] shadow OK: {} tasks, different valid order",
                        taskPaths.size());
                } else {
                    LOGGER.warn("[substrate:taskgraph] Rust execution order violates Java dependencies");
                }
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:taskgraph] shadow comparison failed", e);
        }
    }

    /**
     * Verify that the Rust ordering is topologically valid according to Java's dependency graph.
     */
    private boolean validateTopologicalOrder(
        List<String> order,
        java.util.Map<String, List<String>> dependencies
    ) {
        java.util.Set<String> seen = new java.util.HashSet<>();
        for (String task : order) {
            List<String> deps = dependencies.getOrDefault(task, new ArrayList<>());
            for (String dep : deps) {
                if (!seen.contains(dep)) {
                    LOGGER.debug("[substrate:taskgraph] Task {} depends on {} which hasn't been scheduled yet",
                        task, dep);
                    return false;
                }
            }
            seen.add(task);
        }
        return true;
    }
}
