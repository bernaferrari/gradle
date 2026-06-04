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

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
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
            String stableBuildId = stableBuildIdentity(tasks);
            if (!stableBuildId.equals(buildId)) {
                BuildPlan stablePlan = inlineBuildPlan(stableBuildId, tasks, taskContracts)
                    .toBuilder()
                    .putMetadata("sessionBuildId", buildId)
                    .putMetadata("stableBuildIdentity", stableBuildId)
                    .build();
                boolean stableRefreshed = bootstrapClient.refreshBuildPlanShadow(stableBuildId, stablePlan);
                LOGGER.info(
                    "[substrate:taskgraph] stable inline build-plan refresh for {} sent {} task contracts (refreshed={})",
                    stableBuildId,
                    stablePlan.getTasksCount(),
                    stableRefreshed
                );
            }
        }

        reporter.compareExecutionGraph(taskPaths, taskDependencies, buildId);
    }

    private static BuildPlan inlineBuildPlan(String buildId, List<Task> tasks, List<BuildPlanTask> taskContracts) {
        Map<String, Project> projectsByPath = new LinkedHashMap<>();
        for (Task task : tasks) {
            projectsByPath.putIfAbsent(task.getProject().getPath(), task.getProject());
        }

        BuildPlan.Builder plan = BuildPlan.newBuilder()
            .setSchemaVersion(4)
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

    private static String stableBuildIdentity(List<Task> tasks) {
        if (tasks.isEmpty()) {
            return "stable-root-empty";
        }
        return stableBuildIdentityForRoot(tasks.get(0).getProject().getRootProject().getProjectDir());
    }

    static String stableBuildIdentityForRoot(File rootDir) {
        byte[] digest = sha256(canonicalRootPath(rootDir));
        StringBuilder suffix = new StringBuilder(16);
        for (int i = 0; i < 8; i++) {
            suffix.append(String.format("%02x", digest[i] & 0xff));
        }
        return "stable-root-" + suffix;
    }

    private static String canonicalRootPath(File rootDir) {
        try {
            return rootDir.getCanonicalPath();
        } catch (IOException e) {
            return rootDir.getAbsoluteFile().toPath().normalize().toString();
        }
    }

    private static byte[] sha256(String value) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            return digest.digest(value.getBytes(StandardCharsets.UTF_8));
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException("SHA-256 digest is unavailable", e);
        }
    }
}
