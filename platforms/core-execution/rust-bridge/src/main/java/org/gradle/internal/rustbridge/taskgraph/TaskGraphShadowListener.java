package org.gradle.internal.rustbridge.taskgraph;

import org.gradle.api.Task;
import org.gradle.api.execution.TaskExecutionGraph;
import org.gradle.api.execution.TaskExecutionGraphListener;

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
public class TaskGraphShadowListener implements TaskExecutionGraphListener {

    private final TaskGraphShadowReporter reporter;

    public TaskGraphShadowListener(TaskGraphShadowReporter reporter) {
        this.reporter = reporter;
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

        reporter.compareExecutionGraph(taskPaths, taskDependencies, "build");
    }
}
