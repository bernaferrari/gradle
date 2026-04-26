package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlan;
import gradle.substrate.v1.BuildPlanProject;
import gradle.substrate.v1.BuildPlanTask;

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
            .setSchemaVersion(1)
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
        if (taskSelectionSnapshot != null) {
            BuildPlanTaskSelectionSnapshot.Snapshot selectedGraph = taskSelectionSnapshot.snapshot();
            if (selectedGraph.isPopulated()) {
                tasks = getSelectedBuildPlanTasks(selectedGraph);
                taskSource = "jvm-host-selected-task-graph";
            }
        }
        plan.addAllTasks(tasks);
        plan.putMetadata("taskSource", tasks.isEmpty() ? taskSource + "-empty" : taskSource);
        plan.putMetadata("jvmHostTaskCount", Integer.toString(tasks.size()));

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

        public ResolvedArtifactEntry(String group, String name, String version, String configuration) {
            this.group = group;
            this.name = name;
            this.version = version;
            this.configuration = configuration;
        }

        public String getGroup() { return group; }
        public String getName() { return name; }
        public String getVersion() { return version; }
        public String getConfiguration() { return configuration; }
    }

    private static String projectDirFromBuildFile(String buildFile) {
        if (buildFile == null || buildFile.isEmpty()) {
            return "";
        }
        java.io.File parent = new java.io.File(buildFile).getParentFile();
        return parent == null ? "" : parent.getAbsolutePath();
    }
}
