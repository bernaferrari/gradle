package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlan;
import gradle.substrate.v1.BuildPlanTask;

import org.junit.Test;

import java.io.File;
import java.util.Collections;
import java.util.List;

import static org.junit.Assert.assertEquals;

public class JvmHostServiceImplTest {

    @Test
    public void buildPlanPrefersSelectedTaskSnapshotOverLiveModelRefresh() {
        BuildPlanTaskSelectionSnapshot selected = new BuildPlanTaskSelectionSnapshot();
        selected.recordSelectedTasks(
            Collections.singletonList(":compileJava"),
            Collections.singletonMap(":compileJava", Collections.emptyList()),
            Collections.singletonList(task(":compileJava", "stale-classpath"))
        );

        JvmHostServiceImpl service = new JvmHostServiceImpl();
        service.setTaskSelectionSnapshot(selected);
        service.setProjectModelProvider(new StubProjectModelProvider(
            Collections.singletonList(task(":compileJava", "fresh-classpath"))
        ));

        BuildPlan plan = service.getBuildPlan("build");

        assertEquals("jvm-host-selected-task-graph-snapshot", plan.getMetadataOrThrow("taskSource"));
        assertEquals("stale-classpath", plan.getTasks(0).getInputsOrThrow("classpath"));
    }

    @Test
    public void buildPlanFallsBackToSelectedModelRefreshWhenSnapshotContractIsUnavailable() {
        BuildPlanTaskSelectionSnapshot selected = new BuildPlanTaskSelectionSnapshot();
        selected.recordSelectedTasks(
            Collections.singletonList(":compileJava"),
            Collections.singletonMap(":compileJava", Collections.emptyList()),
            Collections.emptyList()
        );

        JvmHostServiceImpl service = new JvmHostServiceImpl();
        service.setTaskSelectionSnapshot(selected);
        service.setProjectModelProvider(new StubProjectModelProvider(
            Collections.singletonList(task(":compileJava", "fresh-classpath"))
        ));

        BuildPlan plan = service.getBuildPlan("build");

        assertEquals("jvm-host-selected-task-graph-model-refreshed", plan.getMetadataOrThrow("taskSource"));
        assertEquals("fresh-classpath", plan.getTasks(0).getInputsOrThrow("classpath"));
    }

    private static BuildPlanTask task(String path, String classpath) {
        return BuildPlanTask.newBuilder()
            .setPath(path)
            .setProjectPath(":")
            .setImplementationId("org.gradle.api.tasks.compile.JavaCompile")
            .putInputs("classpath", classpath)
            .build();
    }

    private static class StubProjectModelProvider implements JvmHostServiceImpl.ProjectModelProvider {
        private final List<BuildPlanTask> selectedTasks;

        private StubProjectModelProvider(List<BuildPlanTask> selectedTasks) {
            this.selectedTasks = selectedTasks;
        }

        @Override
        public List<JvmHostServiceImpl.ProjectModelEntry> getProjectModels() {
            return Collections.singletonList(new JvmHostServiceImpl.ProjectModelEntry(
                ":",
                "root",
                new File("build.gradle").getAbsolutePath(),
                Collections.emptyList()
            ));
        }

        @Override
        public List<BuildPlanTask> getBuildPlanTasks() {
            return Collections.emptyList();
        }

        @Override
        public List<BuildPlanTask> getSelectedBuildPlanTasks(BuildPlanTaskSelectionSnapshot.Snapshot selectedGraph) {
            return selectedTasks;
        }

        @Override
        public List<JvmHostServiceImpl.ResolvedArtifactEntry> resolveArtifacts(String projectPath, String configurationName) {
            return Collections.emptyList();
        }
    }
}
