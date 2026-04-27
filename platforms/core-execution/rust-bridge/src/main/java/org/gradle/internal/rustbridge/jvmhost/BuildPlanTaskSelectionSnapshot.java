package org.gradle.internal.rustbridge.jvmhost;

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
        snapshot = Snapshot.populated(taskPaths, taskDependencies);
    }

    public Snapshot snapshot() {
        return snapshot;
    }

    public static class Snapshot {
        private final boolean populated;
        private final List<String> taskPaths;
        private final Map<String, List<String>> taskDependencies;

        private Snapshot(boolean populated, List<String> taskPaths, Map<String, List<String>> taskDependencies) {
            this.populated = populated;
            this.taskPaths = Collections.unmodifiableList(new ArrayList<>(taskPaths));
            Map<String, List<String>> copiedDependencies = new LinkedHashMap<>();
            for (Map.Entry<String, List<String>> entry : taskDependencies.entrySet()) {
                copiedDependencies.put(entry.getKey(), Collections.unmodifiableList(new ArrayList<>(entry.getValue())));
            }
            this.taskDependencies = Collections.unmodifiableMap(copiedDependencies);
        }

        private static Snapshot notPopulated() {
            return new Snapshot(false, Collections.emptyList(), Collections.emptyMap());
        }

        private static Snapshot populated(List<String> taskPaths, Map<String, List<String>> taskDependencies) {
            return new Snapshot(true, taskPaths, taskDependencies);
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
    }
}
