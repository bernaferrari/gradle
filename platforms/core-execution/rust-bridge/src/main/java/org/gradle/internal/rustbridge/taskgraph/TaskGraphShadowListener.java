package org.gradle.internal.rustbridge.taskgraph;

import gradle.substrate.v1.BuildPlan;
import gradle.substrate.v1.BuildPlanProject;
import gradle.substrate.v1.BuildPlanTask;

import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.execution.TaskExecutionGraph;
import org.gradle.api.execution.TaskExecutionGraphListener;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.bootstrap.RustBootstrapClient;
import org.gradle.internal.rustbridge.eventstream.BuildIdHolder;
import org.gradle.internal.rustbridge.jvmhost.BuildPlanTaskSelectionSnapshot;
import org.gradle.internal.rustbridge.jvmhost.ProjectModelProviderAdapter;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
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

    private static final Logger LOGGER = Logging.getLogger(TaskGraphShadowListener.class);

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
        List<BuildPlanTask> taskContracts = new ArrayList<>();

        for (Task task : tasks) {
            String path = task.getPath();
            taskPaths.add(path);
            Set<Task> deps = graph.getDependencies(task);
            List<String> dependencyPaths = deps.stream().map(Task::getPath).collect(Collectors.toList());
            taskDependencies.put(path, dependencyPaths);
            taskContracts.add(ProjectModelProviderAdapter.toBuildPlanTask(task, dependencyPaths));
        }

        if (taskSelectionSnapshot != null) {
            taskSelectionSnapshot.recordSelectedTasks(taskPaths, taskDependencies, tasks, taskContracts);
        }

        String activeBuildId = BuildIdHolder.getBuildId();
        String buildId = activeBuildId.isEmpty() ? "build" : activeBuildId;
        if (bootstrapClient != null) {
            BuildPlan inlinePlan = inlineBuildPlan(buildId, tasks, taskContracts);
            boolean refreshed = bootstrapClient.refreshBuildPlanShadow(buildId, inlinePlan);
            LOGGER.info(
                "[substrate:taskgraph] inline build-plan refresh for {} sent {} task contracts (refreshed={})",
                buildId,
                inlinePlan.getTasksCount(),
                refreshed
            );
        }

        reporter.compareExecutionGraph(taskPaths, taskDependencies, buildId);
    }

    private static BuildPlan inlineBuildPlan(String buildId, List<Task> tasks, List<BuildPlanTask> taskContracts) {
        Map<String, Project> projectsByPath = new LinkedHashMap<>();
        for (Task task : tasks) {
            projectsByPath.putIfAbsent(task.getProject().getPath(), task.getProject());
        }

        BuildPlan.Builder plan = BuildPlan.newBuilder()
            .setSchemaVersion(2)
            .setBuildId(buildId)
            .addAllTasks(taskContracts)
            .putMetadata("source", "task-graph-listener-inline")
            .putMetadata("taskSource", "jvm-selected-task-graph-inline")
            .putMetadata("selectedTaskReferenceCount", Integer.toString(tasks.size()))
            .putMetadata("selectedTaskContractCount", Integer.toString(taskContracts.size()))
            .putMetadata("dependencyCount", "0")
            .putMetadata("dependencyCapture", "deferred");

        for (Project project : projectsByPath.values()) {
            plan.addProjects(BuildPlanProject.newBuilder()
                .setPath(project.getPath())
                .setName(project.getName())
                .setProjectDir(project.getProjectDir().getAbsolutePath())
                .build());
        }

        return plan.build();
    }
}
