package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlanTask;

import org.gradle.api.Task;
import org.gradle.api.execution.TaskExecutionGraph;
import org.gradle.api.execution.TaskExecutionGraphListener;
import org.gradle.api.logging.Logging;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;

/**
 * Captures selected task contracts as soon as Gradle has populated the execution graph.
 *
 * <p>This runs before the authoritative executor asks Rust to execute the DAG. Some task
 * inputs, especially dependency-backed classpaths, cannot be resolved safely from the
 * later execution-plan hook because Gradle has already left the exclusive resolution
 * window. Capturing here gives Rust a native-ready contract while the later executor
 * still owns the final fail-closed decision.</p>
 */
public class BuildPlanTaskSelectionCaptureListener implements TaskExecutionGraphListener {
    private static final Logger LOGGER = Logging.getLogger(BuildPlanTaskSelectionCaptureListener.class);

    private final BuildPlanTaskSelectionSnapshot taskSelectionSnapshot;

    public BuildPlanTaskSelectionCaptureListener(BuildPlanTaskSelectionSnapshot taskSelectionSnapshot) {
        this.taskSelectionSnapshot = taskSelectionSnapshot;
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

        taskSelectionSnapshot.recordSelectedTasks(taskPaths, taskDependencies, tasks, taskContracts);
        LOGGER.info(
            "[substrate:taskgraph] captured {} selected task contracts at task graph population",
            taskContracts.size()
        );
    }
}
