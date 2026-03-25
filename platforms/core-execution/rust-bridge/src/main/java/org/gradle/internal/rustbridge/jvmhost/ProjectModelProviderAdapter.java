package org.gradle.internal.rustbridge.jvmhost;

import org.gradle.api.logging.Logging;
import org.gradle.api.internal.project.ProjectState;
import org.gradle.api.internal.project.ProjectStateRegistry;
import org.slf4j.Logger;
import org.jspecify.annotations.Nullable;

import java.io.File;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;

/**
 * Adapter that reads Gradle's project model from {@link ProjectStateRegistry}
 * and provides it to the JVM host service.
 */
public class ProjectModelProviderAdapter implements JvmHostServiceImpl.ProjectModelProvider {

    private static final Logger LOGGER = Logging.getLogger(ProjectModelProviderAdapter.class);

    @Nullable
    private final ProjectStateRegistry projectStateRegistry;

    public ProjectModelProviderAdapter(@Nullable ProjectStateRegistry projectStateRegistry) {
        this.projectStateRegistry = projectStateRegistry;
    }

    @Override
    public List<JvmHostServiceImpl.ProjectModelEntry> getProjectModels() {
        if (projectStateRegistry == null) {
            return java.util.Collections.emptyList();
        }

        List<JvmHostServiceImpl.ProjectModelEntry> entries = new ArrayList<>();
        try {
            Collection<? extends ProjectState> allProjects = projectStateRegistry.getAllProjects();
            for (ProjectState projectState : allProjects) {
                String path = projectState.getProjectPath().getPath();
                String name = projectState.getName();
                File projectDir = projectState.getProjectDir();
                String buildFile = resolveBuildFile(projectDir);

                List<String> subprojects = new ArrayList<>();
                for (ProjectState child : projectState.getChildProjects()) {
                    subprojects.add(child.getProjectPath().getPath());
                }

                entries.add(new JvmHostServiceImpl.ProjectModelEntry(path, name, buildFile, subprojects));
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate-jvmhost] Failed to read project models", e);
        }
        return entries;
    }

    @Override
    public List<JvmHostServiceImpl.ResolvedArtifactEntry> resolveArtifacts(
            String projectPath, String configurationName) {
        if (projectStateRegistry == null) {
            return java.util.Collections.emptyList();
        }

        try {
            ProjectState targetProject = null;
            for (ProjectState ps : projectStateRegistry.getAllProjects()) {
                if (ps.getProjectPath().getPath().equals(projectPath)) {
                    targetProject = ps;
                    break;
                }
            }

            if (targetProject == null) {
                LOGGER.debug("[substrate-jvmhost] Project not found: {}", projectPath);
                return java.util.Collections.emptyList();
            }

            if (!targetProject.isCreated()) {
                LOGGER.debug("[substrate-jvmhost] Project not yet configured: {}", projectPath);
                return java.util.Collections.emptyList();
            }

            org.gradle.api.internal.project.ProjectInternal project =
                targetProject.getMutableModel();

            org.gradle.api.artifacts.Configuration configuration =
                project.getConfigurations().findByName(configurationName);
            if (configuration == null) {
                LOGGER.debug("[substrate-jvmhost] Configuration not found: {} in project {}",
                    configurationName, projectPath);
                return java.util.Collections.emptyList();
            }

            if (!configuration.isCanBeResolved()) {
                LOGGER.debug("[substrate-jvmhost] Configuration cannot be resolved: {} in project {}",
                    configurationName, projectPath);
                return java.util.Collections.emptyList();
            }

            List<JvmHostServiceImpl.ResolvedArtifactEntry> artifacts = new ArrayList<>();
            for (org.gradle.api.artifacts.ResolvedArtifact resolvedArtifact :
                    configuration.getResolvedConfiguration().getResolvedArtifacts()) {
                org.gradle.api.artifacts.ModuleVersionIdentifier id =
                    resolvedArtifact.getModuleVersion().getId();
                artifacts.add(new JvmHostServiceImpl.ResolvedArtifactEntry(
                    id.getGroup(),
                    id.getName(),
                    id.getVersion(),
                    configurationName
                ));
            }
            return artifacts;
        } catch (Exception e) {
            LOGGER.debug("[substrate-jvmhost] Failed to resolve artifacts for {}:{}",
                projectPath, configurationName, e);
            return java.util.Collections.emptyList();
        }
    }

    private static String resolveBuildFile(File projectDir) {
        File kotlinBuildFile = new File(projectDir, "build.gradle.kts");
        if (kotlinBuildFile.exists()) {
            return kotlinBuildFile.getAbsolutePath();
        }
        File groovyBuildFile = new File(projectDir, "build.gradle");
        if (groovyBuildFile.exists()) {
            return groovyBuildFile.getAbsolutePath();
        }
        return "";
    }
}
