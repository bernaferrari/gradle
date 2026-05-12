package org.gradle.internal.rustbridge.taskgraph;

import org.gradle.api.logging.Logging;
import org.gradle.api.execution.TaskExecutionGraph;
import org.gradle.internal.rustbridge.SubstrateException;
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
 * otherwise explicit {@code rust-error} metadata is returned instead of falling
 * back to Java.</p>
 */
public class TaskGraphShadowReporter {

    private static final Logger LOGGER = Logging.getLogger(TaskGraphShadowReporter.class);

    private final RustTaskGraphClient rustClient;
    private final RustBuildExecutionClient buildExecutionClient;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;
    private final boolean runBuildEnabled;
    private final boolean runBuildAuthoritative;

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
        this(rustClient, null, mismatchReporter, authoritative, false, false);
    }

    public TaskGraphShadowReporter(
        RustTaskGraphClient rustClient,
        RustBuildExecutionClient buildExecutionClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative,
        boolean runBuildEnabled,
        boolean runBuildAuthoritative
    ) {
        this.rustClient = rustClient;
        this.buildExecutionClient = buildExecutionClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
        this.runBuildEnabled = runBuildEnabled;
        this.runBuildAuthoritative = runBuildAuthoritative;
    }

    public boolean isAuthoritative() {
        return authoritative;
    }

    public boolean isRunBuildEnabled() {
        return runBuildEnabled;
    }

    public boolean isRunBuildAuthoritative() {
        return runBuildAuthoritative;
    }

    /**
     * Effective task graph decision in authoritative-or-shadow mode.
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
        runBuildFromShadowIfEnabled(taskPaths, buildId);
    }

    public RustBuildExecutionClient.RunBuildResult runBuildFromShadowIfEnabled(
        List<String> taskPaths,
        String buildId
    ) {
        if (!runBuildEnabled || taskPaths.isEmpty()) {
            return RustBuildExecutionClient.RunBuildResult.success("disabled", 0, 0);
        }
        if (buildExecutionClient == null) {
            RustBuildExecutionClient.RunBuildResult result =
                RustBuildExecutionClient.RunBuildResult.error("Rust build execution client is unavailable");
            handleRunBuildFailure(buildId, result);
            return result;
        }

        RustBuildExecutionClient.RunBuildResult result = buildExecutionClient.runBuild(
            buildId,
            Runtime.getRuntime().availableProcessors(),
            false
        );
        if (result.isSuccess() && result.getTasksForwardedToJvm() == 0) {
            mismatchReporter.reportMatch();
            LOGGER.info(
                "[substrate:run-build] Rust executed {} tasks from {} with {} up-to-date, {} no-source/skipped and {} from-cache; JVM forwarding disabled",
                result.getTotalTasks(),
                displayPlanSource(result.getPlanSource()),
                result.getTasksUpToDate(),
                result.getTasksSkipped(),
                result.getTasksFromCache()
            );
            return result;
        }

        handleRunBuildFailure(buildId, result);
        return result;
    }

    private void handleRunBuildFailure(String buildId, RustBuildExecutionClient.RunBuildResult result) {
        String reason = result.getErrorMessage();
        if (reason == null || reason.isEmpty()) {
            reason = result.getFailureMessage();
        }
        if (reason == null || reason.isEmpty()) {
            reason = "status=" + result.getFinalStatus()
                + ", failed=" + result.getTasksFailed()
                + ", jvmForwarded=" + result.getTasksForwardedToJvm();
        }
        RuntimeException failure = new RuntimeException(reason);
        mismatchReporter.reportRustError("run-build:" + buildId, failure);
        LOGGER.warn("[substrate:run-build] Rust run-build failed for {}: {}", buildId, reason);
        if (runBuildAuthoritative) {
            throw new SubstrateException("Rust authoritative run-build failed for " + buildId + ": " + reason, failure);
        }
    }

    static String displayPlanSource(String planSource) {
        if ("build-plan-shadow".equals(planSource)) {
            return "build-plan-cache";
        }
        return planSource;
    }

    /**
     * Resolve Rust task graph and return effective execution order.
     * In authoritative mode, returns Rust order when valid; otherwise an explicit Rust failure.
     */
    public EffectiveExecutionGraphResult resolveExecutionGraphOrFallback(
        List<String> taskPaths,
        java.util.Map<String, List<String>> taskDependencies,
        String buildId
    ) {
        if (taskPaths.isEmpty()) {
            return new EffectiveExecutionGraphResult(taskPaths, "java-shadow");
        }
        if (rustClient == null) {
            if (authoritative) {
                return authoritativeFailureResult();
            }
            return new EffectiveExecutionGraphResult(taskPaths, "java-shadow");
        }

        try {
            RustTaskGraphClient.ExecutionPlanResult shadowPlan = rustClient.resolveExecutionPlan(buildId, true);
            EffectiveExecutionGraphResult shadowResult = effectiveResultFromRustPlan(
                shadowPlan,
                taskPaths,
                taskDependencies,
                buildId,
                false
            );
            if (shadowResult != null) {
                return shadowResult;
            }

            rustClient.clearBuildTasks(buildId);

            // Register all tasks with Rust
            for (String taskPath : taskPaths) {
                List<String> deps = taskDependencies.getOrDefault(taskPath, new ArrayList<>());
                rustClient.registerTask(buildId, taskPath, deps, true, "Task");
            }

            // Resolve Rust execution plan
            RustTaskGraphClient.ExecutionPlanResult rustResult =
                rustClient.resolveExecutionPlan(buildId);

            EffectiveExecutionGraphResult result = effectiveResultFromRustPlan(
                rustResult,
                taskPaths,
                taskDependencies,
                buildId,
                true
            );
            if (result != null) {
                return result;
            }
            return authoritative
                ? authoritativeFailureResult()
                : new EffectiveExecutionGraphResult(taskPaths, "java-shadow");
        } catch (Exception e) {
            mismatchReporter.reportRustError("task-graph:" + buildId, e);
            LOGGER.debug("[substrate:taskgraph] shadow comparison failed", e);
            return authoritative
                ? authoritativeFailureResult()
                : new EffectiveExecutionGraphResult(taskPaths, "java-shadow");
        }
    }

    private EffectiveExecutionGraphResult authoritativeFailureResult() {
        return new EffectiveExecutionGraphResult(Collections.emptyList(), "rust-error");
    }

    private EffectiveExecutionGraphResult effectiveResultFromRustPlan(
        RustTaskGraphClient.ExecutionPlanResult rustResult,
        List<String> taskPaths,
        java.util.Map<String, List<String>> taskDependencies,
        String buildId,
        boolean reportFailures
    ) {
        if (!rustResult.isSuccess()) {
            if (reportFailures) {
                mismatchReporter.reportRustError(
                    "task-graph:" + buildId,
                    new RuntimeException(rustResult.getErrorMessage())
                );
                LOGGER.debug("[substrate:taskgraph] Rust resolve failed: {}",
                    rustResult.getErrorMessage());
            }
            return null;
        }

        if (rustResult.hasCycles()) {
            if (reportFailures) {
                LOGGER.warn("[substrate:taskgraph] Rust detected cycles that Java did not");
                mismatchReporter.reportMismatch(
                    "task-graph:" + buildId,
                    "java:no-cycles",
                    "rust:cycles"
                );
            }
            return null;
        }

        // Compare task counts
        if (rustResult.getTotalTasks() != taskPaths.size()) {
            if (reportFailures) {
                LOGGER.warn("[substrate:taskgraph] Task count mismatch: java={}, rust={}",
                    taskPaths.size(), rustResult.getTotalTasks());
                mismatchReporter.reportMismatch(
                    "task-graph-count:" + buildId,
                    Integer.toString(taskPaths.size()),
                    Integer.toString(rustResult.getTotalTasks())
                );
            }
            return null;
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
                if (reportFailures) {
                    mismatchReporter.reportMismatch(
                        "task-graph-order:" + buildId,
                        taskPaths.toString(),
                        rustOrder.toString()
                    );
                    LOGGER.warn("[substrate:taskgraph] Rust execution order violates Java dependencies");
                }
                return null;
            }
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
