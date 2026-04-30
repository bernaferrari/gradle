package org.gradle.internal.rustbridge.taskgraph;

import org.gradle.api.Task;
import org.gradle.api.execution.TaskExecutionGraph;
import org.gradle.api.execution.TaskExecutionGraphListener;
import org.gradle.internal.rustbridge.bootstrap.RustBootstrapClient;
import org.gradle.internal.rustbridge.eventstream.BuildIdHolder;
import org.gradle.internal.rustbridge.jvmhost.BuildPlanTaskSelectionSnapshot;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;
import org.jspecify.annotations.Nullable;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;

/**
 * A {@link TaskExecutionGraphListener} that shadow-compares the Java task execution
 * ordering with the Rust task graph service.
 *
 * <p>Extracts task paths and dependency information from the populated graph and
 * delegates to {@link TaskGraphShadowReporter} for the actual comparison.</p>
 */
@ServiceScope(Scope.Build.class)
public class TaskGraphShadowListener implements TaskExecutionGraphListener {

    private final TaskGraphShadowReporter reporter;
    @Nullable
    private final BuildPlanTaskSelectionSnapshot taskSelectionSnapshot;
    @Nullable
    private final RustBootstrapClient bootstrapClient;

    public TaskGraphShadowListener(TaskGraphShadowReporter reporter) {
        this(reporter, null, null);
    }

    public TaskGraphShadowListener(
        TaskGraphShadowReporter reporter,
        @Nullable BuildPlanTaskSelectionSnapshot taskSelectionSnapshot
    ) {
        this(reporter, taskSelectionSnapshot, null);
    }

    public TaskGraphShadowListener(
        TaskGraphShadowReporter reporter,
        @Nullable BuildPlanTaskSelectionSnapshot taskSelectionSnapshot,
        @Nullable RustBootstrapClient bootstrapClient
    ) {
        this.reporter = reporter;
        this.taskSelectionSnapshot = taskSelectionSnapshot;
        this.bootstrapClient = bootstrapClient;
    }

    @Override
    public void graphPopulated(TaskExecutionGraph graph) {
        List<Task> tasks = graph.getAllTasks();
        List<String> taskPaths = new ArrayList<>();
        Map<String, List<String>> taskDependencies = new HashMap<>();

        for (Task task : tasks) {
            String path = task.getPath();
            taskPaths.add(path);
            Set<Task> deps = graph.getDependencies(task);
            taskDependencies.put(path,
                deps.stream().map(Task::getPath).collect(Collectors.toList()));
        }

        if (taskSelectionSnapshot != null) {
            taskSelectionSnapshot.recordSelectedTasks(taskPaths, taskDependencies);
        }

        String activeBuildId = BuildIdHolder.getBuildId();
        String buildId = activeBuildId.isEmpty() ? "build" : activeBuildId;
        if (bootstrapClient != null) {
            bootstrapClient.refreshBuildPlanShadow(buildId);
        }

        reporter.compareExecutionGraph(taskPaths, taskDependencies, buildId);
    }
}
