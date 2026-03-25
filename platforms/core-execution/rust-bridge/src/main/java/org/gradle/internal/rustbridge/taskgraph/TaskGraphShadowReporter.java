package org.gradle.internal.rustbridge.taskgraph;

import org.gradle.api.logging.Logging;
import org.gradle.api.execution.TaskExecutionGraph;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * Hooks into {@link TaskExecutionGraph#whenReady()} to shadow-compare
 * the Java task execution ordering with the Rust task graph service.
 *
 * <p>Registers all tasks with the Rust service, resolves its execution plan,
 * and compares the topological ordering with Gradle's Java resolver.
 * In authoritative mode, a valid Rust plan becomes the effective order
 * returned by {@link #resolveExecutionGraphOrFallback(List, java.util.Map, String)};
 * otherwise Java order is used as fallback.</p>
 */
public class TaskGraphShadowReporter {

    private static final Logger LOGGER = Logging.getLogger(TaskGraphShadowReporter.class);

    private final RustTaskGraphClient rustClient;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public TaskGraphShadowReporter(
        RustTaskGraphClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this(rustClient, mismatchReporter, false);
    }

    public TaskGraphShadowReporter(
        RustTaskGraphClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    public boolean isAuthoritative() {
        return authoritative;
    }

    /**
     * Effective task graph decision in authoritative-or-fallback mode.
     */
    public static class EffectiveExecutionGraphResult {
        private final List<String> executionOrder;
        private final String source;

        private EffectiveExecutionGraphResult(List<String> executionOrder, String source) {
            this.executionOrder = executionOrder;
            this.source = source;
        }

        public List<String> getExecutionOrder() {
            return executionOrder;
        }

        public String getSource() {
            return source;
        }

        public boolean isRustSource() {
            return "rust".equals(source);
        }
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
        resolveExecutionGraphOrFallback(taskPaths, taskDependencies, buildId);
    }

    /**
     * Resolve Rust task graph and return effective execution order.
     * In authoritative mode, returns Rust order when valid; otherwise Java order fallback.
     */
    public EffectiveExecutionGraphResult resolveExecutionGraphOrFallback(
        List<String> taskPaths,
        java.util.Map<String, List<String>> taskDependencies,
        String buildId
    ) {
        if (rustClient == null || taskPaths.isEmpty()) {
            return new EffectiveExecutionGraphResult(taskPaths, "java-shadow");
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
                mismatchReporter.reportRustError(
                    "task-graph:" + buildId,
                    new RuntimeException(rustResult.getErrorMessage())
                );
                LOGGER.debug("[substrate:taskgraph] Rust resolve failed: {}",
                    rustResult.getErrorMessage());
                return new EffectiveExecutionGraphResult(taskPaths, "java-fallback");
            }

            if (rustResult.hasCycles()) {
                LOGGER.warn("[substrate:taskgraph] Rust detected cycles that Java did not");
                mismatchReporter.reportMismatch(
                    "task-graph:" + buildId,
                    "java:no-cycles",
                    "rust:cycles"
                );
                return new EffectiveExecutionGraphResult(taskPaths, "java-fallback");
            }

            // Compare task counts
            if (rustResult.getTotalTasks() != taskPaths.size()) {
                LOGGER.warn("[substrate:taskgraph] Task count mismatch: java={}, rust={}",
                    taskPaths.size(), rustResult.getTotalTasks());
                mismatchReporter.reportMismatch(
                    "task-graph-count:" + buildId,
                    Integer.toString(taskPaths.size()),
                    Integer.toString(rustResult.getTotalTasks())
                );
                return new EffectiveExecutionGraphResult(taskPaths, "java-fallback");
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
                if (authoritative) {
                    return new EffectiveExecutionGraphResult(
                        Collections.unmodifiableList(new ArrayList<>(rustOrder)),
                        "rust"
                    );
                }
                return new EffectiveExecutionGraphResult(taskPaths, "java-shadow");
            } else {
                // Order may differ if Java uses a different tie-breaking strategy.
                // Check that the ordering is still topologically valid relative to Java.
                boolean isValid = validateTopologicalOrder(rustOrder, taskDependencies);
                if (isValid) {
                    mismatchReporter.reportMatch();
                    LOGGER.debug("[substrate:taskgraph] shadow OK: {} tasks, different valid order",
                        taskPaths.size());
                    if (authoritative) {
                        return new EffectiveExecutionGraphResult(
                            Collections.unmodifiableList(new ArrayList<>(rustOrder)),
                            "rust"
                        );
                    }
                    return new EffectiveExecutionGraphResult(taskPaths, "java-shadow");
                } else {
                    mismatchReporter.reportMismatch(
                        "task-graph-order:" + buildId,
                        taskPaths.toString(),
                        rustOrder.toString()
                    );
                    LOGGER.warn("[substrate:taskgraph] Rust execution order violates Java dependencies");
                    return new EffectiveExecutionGraphResult(taskPaths, "java-fallback");
                }
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("task-graph:" + buildId, e);
            LOGGER.debug("[substrate:taskgraph] shadow comparison failed", e);
            return new EffectiveExecutionGraphResult(taskPaths, "java-fallback");
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
