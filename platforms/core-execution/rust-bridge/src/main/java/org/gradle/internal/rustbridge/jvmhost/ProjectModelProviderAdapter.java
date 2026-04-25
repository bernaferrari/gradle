package org.gradle.internal.rustbridge.jvmhost;

import org.gradle.api.logging.Logging;
import org.gradle.internal.service.ServiceRegistry;
import org.slf4j.Logger;
import org.jspecify.annotations.Nullable;

import java.io.File;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.List;

/**
 * Adapter that reads Gradle's project model reflectively from the build service registry
 * and provides it to the JVM host service.
 */
public class ProjectModelProviderAdapter implements JvmHostServiceImpl.ProjectModelProvider {

    private static final Logger LOGGER = Logging.getLogger(ProjectModelProviderAdapter.class);
    private static final String PROJECT_STATE_REGISTRY_CLASS = "org.gradle.api.internal.project.ProjectStateRegistry";

    @Nullable
    private final Object projectStateRegistry;

    public ProjectModelProviderAdapter(@Nullable Object projectStateRegistry) {
        this.projectStateRegistry = projectStateRegistry;
    }

    public static ProjectModelProviderAdapter fromServiceRegistry(ServiceRegistry services) {
        return new ProjectModelProviderAdapter(findProjectStateRegistry(services));
    }

    @Override
    public List<JvmHostServiceImpl.ProjectModelEntry> getProjectModels() {
        if (projectStateRegistry == null) {
            return new ArrayList<>();
        }

        List<JvmHostServiceImpl.ProjectModelEntry> entries = new ArrayList<>();
        try {
            for (Object projectState : getAllProjects(projectStateRegistry)) {
                String path = nestedString(projectState, "getProjectPath", "getPath");
                String name = stringValue(projectState, "getName");
                File projectDir = (File) invoke(projectState, "getProjectDir");
                String buildFile = resolveBuildFile(projectDir);

                List<String> subprojects = new ArrayList<>();
                for (Object child : asCollection(invoke(projectState, "getChildProjects"))) {
                    subprojects.add(nestedString(child, "getProjectPath", "getPath"));
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
            return new ArrayList<>();
        }

        try {
            Object targetProject = null;
            for (Object ps : getAllProjects(projectStateRegistry)) {
                if (projectPath.equals(nestedString(ps, "getProjectPath", "getPath"))) {
                    targetProject = ps;
                    break;
                }
            }

            if (targetProject == null) {
                    LOGGER.debug("[substrate-jvmhost] Project not found: {}", projectPath);
                    return new ArrayList<>();
            }

            if (!booleanValue(targetProject, "isCreated")) {
                LOGGER.debug("[substrate-jvmhost] Project not yet configured: {}", projectPath);
                return new ArrayList<>();
            }

            Object project = invoke(targetProject, "getMutableModel");

            Object configurations = invoke(project, "getConfigurations");
            Object configuration = invoke(configurations, "findByName", String.class, configurationName);
            if (configuration == null) {
                LOGGER.debug("[substrate-jvmhost] Configuration not found: {} in project {}",
                    configurationName, projectPath);
                return new ArrayList<>();
            }

            if (!booleanValue(configuration, "isCanBeResolved")) {
                LOGGER.debug("[substrate-jvmhost] Configuration cannot be resolved: {} in project {}",
                    configurationName, projectPath);
                return new ArrayList<>();
            }

            List<JvmHostServiceImpl.ResolvedArtifactEntry> artifacts = new ArrayList<>();
            Object resolvedConfiguration = invoke(configuration, "getResolvedConfiguration");
            for (Object resolvedArtifact : asCollection(invoke(resolvedConfiguration, "getResolvedArtifacts"))) {
                Object moduleVersion = invoke(resolvedArtifact, "getModuleVersion");
                Object id = invoke(moduleVersion, "getId");
                artifacts.add(new JvmHostServiceImpl.ResolvedArtifactEntry(
                    stringValue(id, "getGroup"),
                    stringValue(id, "getName"),
                    stringValue(id, "getVersion"),
                    configurationName
                ));
            }
            return artifacts;
        } catch (Exception e) {
            LOGGER.debug("[substrate-jvmhost] Failed to resolve artifacts for {}:{}",
                projectPath, configurationName, e);
            return new ArrayList<>();
        }
    }

    @Nullable
    private static Object findProjectStateRegistry(ServiceRegistry services) {
        try {
            Class<?> registryType = Class.forName(PROJECT_STATE_REGISTRY_CLASS);
            return services.find(registryType);
        } catch (ClassNotFoundException e) {
            LOGGER.debug("[substrate-jvmhost] ProjectStateRegistry unavailable", e);
            return null;
        }
    }

    @SuppressWarnings("unchecked")
    private static Collection<Object> getAllProjects(Object projectStateRegistry) {
        return asCollection(invoke(projectStateRegistry, "getAllProjects"));
    }

    @SuppressWarnings("unchecked")
    private static Collection<Object> asCollection(@Nullable Object value) {
        if (value instanceof Collection) {
            return (Collection<Object>) value;
        }
        return Collections.emptyList();
    }

    private static boolean booleanValue(Object target, String methodName) {
        Object value = invoke(target, methodName);
        return value instanceof Boolean && (Boolean) value;
    }

    private static String stringValue(Object target, String methodName) {
        Object value = invoke(target, methodName);
        return value == null ? "" : value.toString();
    }

    private static String nestedString(Object target, String outerMethod, String innerMethod) {
        Object outer = invoke(target, outerMethod);
        return outer == null ? "" : stringValue(outer, innerMethod);
    }

    @Nullable
    private static Object invoke(Object target, String methodName) {
        try {
            Method method = target.getClass().getMethod(methodName);
            return method.invoke(target);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException("Cannot access " + target.getClass().getName() + "." + methodName, e);
        } catch (InvocationTargetException e) {
            Throwable cause = e.getCause();
            throw new IllegalStateException("Invocation failed for " + target.getClass().getName() + "." + methodName, cause != null ? cause : e);
        } catch (NoSuchMethodException e) {
            throw new IllegalStateException("Missing method " + target.getClass().getName() + "." + methodName, e);
        }
    }

    @Nullable
    private static Object invoke(Object target, String methodName, Class<?> argumentType, Object argument) {
        try {
            Method method = target.getClass().getMethod(methodName, argumentType);
            return method.invoke(target, argument);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException("Cannot access " + target.getClass().getName() + "." + methodName, e);
        } catch (InvocationTargetException e) {
            Throwable cause = e.getCause();
            throw new IllegalStateException("Invocation failed for " + target.getClass().getName() + "." + methodName, cause != null ? cause : e);
        } catch (NoSuchMethodException e) {
            throw new IllegalStateException("Missing method " + target.getClass().getName() + "." + methodName, e);
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
