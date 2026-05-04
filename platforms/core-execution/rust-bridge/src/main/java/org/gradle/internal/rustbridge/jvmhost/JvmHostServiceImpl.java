package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlan;
import gradle.substrate.v1.BuildPlanProject;
import gradle.substrate.v1.BuildPlanTask;
import gradle.substrate.v1.ExecuteTaskRequest;
import gradle.substrate.v1.ExecuteTaskResponse;

import org.gradle.api.logging.Logging;
import org.jspecify.annotations.Nullable;

import java.util.List;

/**
 * Implementation of the JVM Compatibility Host service.
 *
 * Allows the Rust daemon to call back into the JVM for operations that
 * require the JVM runtime (script evaluation, build model access, etc.).
 */
public class JvmHostServiceImpl {

    private static final org.slf4j.Logger LOGGER =
        Logging.getLogger(JvmHostServiceImpl.class);

    @Nullable
    private ProjectModelProvider projectModelProvider;

    @Nullable
    private BuildPlanTaskSelectionSnapshot taskSelectionSnapshot;

    @Nullable
    private TaskExecutionProvider taskExecutionProvider;

    public JvmHostServiceImpl() {
    }

    /**
     * Set the project model provider. Called during build initialization
     * to give the JVM host access to Gradle's project model.
     */
    public void setProjectModelProvider(@Nullable ProjectModelProvider provider) {
        this.projectModelProvider = provider;
    }

    public void setTaskSelectionSnapshot(@Nullable BuildPlanTaskSelectionSnapshot snapshot) {
        this.taskSelectionSnapshot = snapshot;
    }

    public void setTaskExecutionProvider(@Nullable TaskExecutionProvider provider) {
        this.taskExecutionProvider = provider;
    }

    /**
     * Returns build environment information from the JVM.
     */
    public String getJavaVersion() {
        return System.getProperty("java.version", "unknown");
    }

    public String getJavaHome() {
        return System.getProperty("java.home", "");
    }

    public String getGradleVersion() {
        try {
            Class<?> versionClass = Class.forName("org.gradle.util.GradleVersion");
            Object current = versionClass.getMethod("current").invoke(null);
            return current.toString();
        } catch (ReflectiveOperationException e) {
            LOGGER.debug("[substrate-jvmhost] Could not determine Gradle version", e);
            return "unknown";
        }
    }

    public String getOsName() {
        return System.getProperty("os.name", "unknown");
    }

    public String getOsArch() {
        return System.getProperty("os.arch", "unknown");
    }

    public int getAvailableProcessors() {
        return Runtime.getRuntime().availableProcessors();
    }

    public long getMaxMemoryBytes() {
        return Runtime.getRuntime().maxMemory();
    }

    // --- Build Model ---

    /**
     * Get the project tree as a list of ProjectModel entries.
     * Returns an empty list if no project model provider is set.
     */
    public List<ProjectModelEntry> getProjectModels() {
        if (projectModelProvider == null) {
            return java.util.Collections.emptyList();
        }
        return projectModelProvider.getProjectModels();
    }

    // --- Configuration Resolution ---

    /**
     * Resolve artifacts for a given project and configuration name.
     * Returns resolved artifact entries, or an empty list if not available.
     */
    public List<ResolvedArtifactEntry> resolveArtifacts(String projectPath, String configurationName) {
        if (projectModelProvider == null) {
            return java.util.Collections.emptyList();
        }
        return projectModelProvider.resolveArtifacts(projectPath, configurationName);
    }

    /**
     * Return a JVM-host build-plan envelope when the compatibility side can provide one.
     *
     * <p>This first implementation is deliberately conservative: it exposes the authoritative
     * project model from Gradle and marks task data as not yet available instead of fabricating
     * realized task nodes.</p>
     */
    public BuildPlan getBuildPlan(String buildId) {
        BuildPlan.Builder plan = BuildPlan.newBuilder()
            .setSchemaVersion(2)
            .setBuildId(buildId)
            .putMetadata("source", "jvm-host");

        for (ProjectModelEntry entry : getProjectModels()) {
            plan.addProjects(BuildPlanProject.newBuilder()
                .setPath(entry.getPath())
                .setName(entry.getName())
                .setProjectDir(projectDirFromBuildFile(entry.getBuildFile()))
                .build());
        }

        String taskSource = "jvm-host-realized-tasks";
        List<BuildPlanTask> tasks = getBuildPlanTasks();
        int selectedTaskReferenceCount = 0;
        int selectedTaskContractCount = 0;
        if (taskSelectionSnapshot != null) {
            BuildPlanTaskSelectionSnapshot.Snapshot selectedGraph = taskSelectionSnapshot.snapshot();
            if (selectedGraph.isPopulated()) {
                selectedTaskReferenceCount = selectedGraph.getTaskReferences().size();
                selectedTaskContractCount = selectedGraph.getTaskContracts().size();
                if (!selectedGraph.getTaskContracts().isEmpty()) {
                    tasks = selectedGraph.getTaskContracts();
                    taskSource = "jvm-host-selected-task-graph-snapshot";
                } else {
                    List<BuildPlanTask> selectedTasks = getSelectedBuildPlanTasks(selectedGraph);
                    tasks = selectedTasks;
                    taskSource = selectedTasks.isEmpty()
                        ? "jvm-host-selected-task-graph-empty"
                        : "jvm-host-selected-task-graph-model-refreshed";
                }
            }
        }
        plan.addAllTasks(tasks);
        plan.putMetadata("taskSource", tasks.isEmpty() ? taskSource + "-empty" : taskSource);
        plan.putMetadata("jvmHostTaskCount", Integer.toString(tasks.size()));
        plan.putMetadata("selectedTaskReferenceCount", Integer.toString(selectedTaskReferenceCount));
        plan.putMetadata("selectedTaskContractCount", Integer.toString(selectedTaskContractCount));

        return plan.build();
    }

    public List<BuildPlanTask> getBuildPlanTasks() {
        if (projectModelProvider == null) {
            return java.util.Collections.emptyList();
        }
        return projectModelProvider.getBuildPlanTasks();
    }

    public List<BuildPlanTask> getSelectedBuildPlanTasks(BuildPlanTaskSelectionSnapshot.Snapshot selectedGraph) {
        if (projectModelProvider == null) {
            return java.util.Collections.emptyList();
        }
        return projectModelProvider.getSelectedBuildPlanTasks(selectedGraph);
    }

    public ExecuteTaskResponse executeTask(ExecuteTaskRequest request) {
        if (taskExecutionProvider == null) {
            return ExecuteTaskResponse.newBuilder()
                .setSuccess(false)
                .setOutcome("UNSUPPORTED")
                .setExecutionMode("jvm_unsupported")
                .setErrorMessage("No JVM task execution provider is registered for " + request.getTaskPath())
                .build();
        }
        return taskExecutionProvider.executeTask(request);
    }

    /**
     * Interface for providing project model data to the JVM host.
     * Implemented by a BuildSession-scoped adapter that reads from Gradle's model.
     */
    public interface ProjectModelProvider {
        List<ProjectModelEntry> getProjectModels();
        List<BuildPlanTask> getBuildPlanTasks();
        List<BuildPlanTask> getSelectedBuildPlanTasks(BuildPlanTaskSelectionSnapshot.Snapshot selectedGraph);
        List<ResolvedArtifactEntry> resolveArtifacts(String projectPath, String configurationName);
    }

    public interface TaskExecutionProvider {
        ExecuteTaskResponse executeTask(ExecuteTaskRequest request);
    }

    public static class ProjectModelEntry {
        private final String path;
        private final String name;
        private final String buildFile;
        private final List<String> subprojects;

        public ProjectModelEntry(String path, String name, String buildFile, List<String> subprojects) {
            this.path = path;
            this.name = name;
            this.buildFile = buildFile;
            this.subprojects = subprojects;
        }

        public String getPath() { return path; }
        public String getName() { return name; }
        public String getBuildFile() { return buildFile; }
        public List<String> getSubprojects() { return subprojects; }
    }

    public static class ResolvedArtifactEntry {
        private final String group;
        private final String name;
        private final String version;
        private final String configuration;
        private final String classifier;
        private final String extension;
        private final String kind;

        public ResolvedArtifactEntry(String group, String name, String version, String configuration) {
            this(group, name, version, configuration, "", "jar", "dependency");
        }

        public ResolvedArtifactEntry(String group, String name, String version, String configuration, String classifier, String extension) {
            this(group, name, version, configuration, classifier, extension, "dependency");
        }

        public ResolvedArtifactEntry(String group, String name, String version, String configuration, String classifier, String extension, String kind) {
            this.group = group;
            this.name = name;
            this.version = version;
            this.configuration = configuration;
            this.classifier = classifier == null ? "" : classifier;
            this.extension = extension == null || extension.isEmpty() ? "jar" : extension;
            this.kind = kind == null || kind.trim().isEmpty() ? "dependency" : kind;
        }

        public String getGroup() { return group; }
        public String getName() { return name; }
        public String getVersion() { return version; }
        public String getConfiguration() { return configuration; }
        public String getClassifier() { return classifier; }
        public String getExtension() { return extension; }
        public String getKind() { return kind; }
    }

    private static String projectDirFromBuildFile(String buildFile) {
        if (buildFile == null || buildFile.isEmpty()) {
            return "";
        }
        java.io.File parent = new java.io.File(buildFile).getParentFile();
        return parent == null ? "" : parent.getAbsolutePath();
    }
}
