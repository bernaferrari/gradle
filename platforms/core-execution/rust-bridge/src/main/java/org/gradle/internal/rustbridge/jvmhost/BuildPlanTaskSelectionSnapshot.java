package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlanTask;

import org.gradle.api.Task;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Thread-safe handoff from Gradle's populated execution graph to the JVM-host build-plan endpoint.
 */
@ServiceScope(Scope.Build.class)
public class BuildPlanTaskSelectionSnapshot {

    private volatile Snapshot snapshot = Snapshot.notPopulated();

    public void recordSelectedTasks(List<String> taskPaths, Map<String, List<String>> taskDependencies) {
        snapshot = Snapshot.populated(taskPaths, taskDependencies, Collections.emptyList(), Collections.emptyList());
    }

    public void recordSelectedTasks(List<String> taskPaths, Map<String, List<String>> taskDependencies, List<BuildPlanTask> taskContracts) {
        snapshot = Snapshot.populated(taskPaths, taskDependencies, Collections.emptyList(), taskContracts);
    }

    public void recordSelectedTasks(
        List<String> taskPaths,
        Map<String, List<String>> taskDependencies,
        List<Task> taskReferences,
        List<BuildPlanTask> taskContracts
    ) {
        snapshot = Snapshot.populated(taskPaths, taskDependencies, taskReferences, taskContracts);
    }

    public Snapshot snapshot() {
        return snapshot;
    }

    public static class Snapshot {
        private final boolean populated;
        private final List<String> taskPaths;
        private final Map<String, List<String>> taskDependencies;
        private final List<Task> taskReferences;
        private final List<BuildPlanTask> taskContracts;

        private Snapshot(
            boolean populated,
            List<String> taskPaths,
            Map<String, List<String>> taskDependencies,
            List<Task> taskReferences,
            List<BuildPlanTask> taskContracts
        ) {
            this.populated = populated;
            this.taskPaths = Collections.unmodifiableList(new ArrayList<>(taskPaths));
            Map<String, List<String>> copiedDependencies = new LinkedHashMap<>();
            for (Map.Entry<String, List<String>> entry : taskDependencies.entrySet()) {
                copiedDependencies.put(entry.getKey(), Collections.unmodifiableList(new ArrayList<>(entry.getValue())));
            }
            this.taskDependencies = Collections.unmodifiableMap(copiedDependencies);
            this.taskReferences = Collections.unmodifiableList(new ArrayList<>(taskReferences));
            this.taskContracts = Collections.unmodifiableList(new ArrayList<>(taskContracts));
        }

        private static Snapshot notPopulated() {
            return new Snapshot(false, Collections.emptyList(), Collections.emptyMap(), Collections.emptyList(), Collections.emptyList());
        }

        private static Snapshot populated(
            List<String> taskPaths,
            Map<String, List<String>> taskDependencies,
            List<Task> taskReferences,
            List<BuildPlanTask> taskContracts
        ) {
            return new Snapshot(true, taskPaths, taskDependencies, taskReferences, taskContracts);
        }

        public boolean isPopulated() {
            return populated;
        }

        public List<String> getTaskPaths() {
            return taskPaths;
        }

        public List<String> getDependencies(String taskPath) {
            return taskDependencies.getOrDefault(taskPath, Collections.emptyList());
        }

        public List<Task> getTaskReferences() {
            return taskReferences;
        }

        public List<BuildPlanTask> getTaskContracts() {
            return taskContracts;
        }
    }
}
