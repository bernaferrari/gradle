package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlanTask;
import gradle.substrate.v1.BuildPlanTaskDiagnostic;
import gradle.substrate.v1.BuildPlanTaskInputSpec;
import gradle.substrate.v1.BuildPlanTaskOutputSpec;

import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.file.FileCollection;
import org.gradle.api.internal.GeneratedSubclasses;
import org.gradle.api.internal.tasks.TaskDependencyUtil;
import org.gradle.api.logging.Logging;
import org.gradle.api.tasks.CacheableTask;
import org.gradle.api.tasks.TaskContainer;
import org.gradle.internal.service.ServiceRegistry;
import org.slf4j.Logger;
import org.jspecify.annotations.Nullable;

import java.io.File;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;

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
    public List<BuildPlanTask> getBuildPlanTasks() {
        if (projectStateRegistry == null) {
            return new ArrayList<>();
        }

        List<BuildPlanTask> tasks = new ArrayList<>();
        try {
            for (Object projectState : getAllProjects(projectStateRegistry)) {
                if (!booleanValue(projectState, "isCreated")) {
                    continue;
                }
                Object mutableModel = invoke(projectState, "getMutableModel");
                if (!(mutableModel instanceof Project)) {
                    continue;
                }
                tasks.addAll(collectProjectTasks((Project) mutableModel));
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate-jvmhost] Failed to read realized task model", e);
        }
        tasks.sort(Comparator.comparing(BuildPlanTask::getPath));
        return tasks;
    }

    @Override
    public List<BuildPlanTask> getSelectedBuildPlanTasks(BuildPlanTaskSelectionSnapshot.Snapshot selectedGraph) {
        if (projectStateRegistry == null) {
            return new ArrayList<>();
        }

        Map<String, Task> tasksByPath = new LinkedHashMap<>();
        try {
            for (Object projectState : getAllProjects(projectStateRegistry)) {
                if (!booleanValue(projectState, "isCreated")) {
                    continue;
                }
                Object mutableModel = invoke(projectState, "getMutableModel");
                if (!(mutableModel instanceof Project)) {
                    continue;
                }
                for (Task task : ((Project) mutableModel).getTasks()) {
                    tasksByPath.put(task.getPath(), task);
                }
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate-jvmhost] Failed to index realized tasks for selected graph", e);
            return new ArrayList<>();
        }

        List<BuildPlanTask> tasks = new ArrayList<>();
        for (String taskPath : selectedGraph.getTaskPaths()) {
            Task task = tasksByPath.get(taskPath);
            if (task != null) {
                tasks.add(toBuildPlanTask(task, selectedGraph.getDependencies(taskPath)));
            }
        }
        return tasks;
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

    private static List<BuildPlanTask> collectProjectTasks(Project project) {
        List<BuildPlanTask> tasks = new ArrayList<>();
        TaskContainer taskContainer = project.getTasks();
        for (Task task : taskContainer) {
            tasks.add(toBuildPlanTask(task));
        }
        return tasks;
    }

    public static BuildPlanTask toBuildPlanTask(Task task) {
        return toBuildPlanTask(task, null);
    }

    private static BuildPlanTask toBuildPlanTask(Task task, @Nullable List<String> selectedDependencyPaths) {
        Class<?> taskType = GeneratedSubclasses.unpackType(task);
        String taskTypeName = taskType.getName();
        String shortTaskTypeName = taskType.getSimpleName();

        Map<String, String> inputs = new LinkedHashMap<>();
        inputs.put("source", "jvm-task-model");
        inputs.put("enabled", Boolean.toString(task.getEnabled()));
        inputs.put("taskName", task.getName());
        inputs.put("taskType", shortTaskTypeName.isEmpty() ? taskTypeName : shortTaskTypeName);
        String group = task.getGroup();
        if (group != null && !group.isEmpty()) {
            inputs.put("group", group);
        }
        String description = task.getDescription();
        if (description != null && !description.isEmpty()) {
            inputs.put("description", description);
        }

        BuildPlanTask.Builder builder = BuildPlanTask.newBuilder()
            .setPath(task.getPath())
            .setProjectPath(task.getProject().getPath())
            .setImplementationId(taskTypeName)
            .setWorkerIsolation(workerIsolation(taskType))
            .setCacheability(cacheability(task, taskType))
            .setActionKind(actionKind(taskType))
            .putAllInputs(inputs);

        List<String> outputPaths = fileCollectionPaths(safeOutputFiles(task));
        builder.addAllDependsOn(selectedDependencyPaths == null
            ? taskDependencyPaths(task, task.getTaskDependencies())
            : selectedDependencyPaths);
        builder.addAllShouldRunAfter(taskDependencyPaths(task, task.getShouldRunAfter()));
        builder.addAllMustRunAfter(taskDependencyPaths(task, task.getMustRunAfter()));
        builder.addAllFinalizedBy(taskDependencyPaths(task, task.getFinalizedBy()));
        builder.addAllOutputs(outputPaths);
        builder.addAllLocalState(fileCollectionPaths(registeredFiles(task.getLocalState())));
        builder.addAllDestroyables(fileCollectionPaths(registeredFiles(task.getDestroyables())));
        builder.addAllInputSpecs(inputSpecs(inputs));
        builder.addAllOutputSpecs(outputSpecs(outputPaths));
        builder.addDiagnostics(BuildPlanTaskDiagnostic.newBuilder()
            .setSeverity("info")
            .setCode("jvm-host-task-contract")
            .setMessage("Task contract captured from Gradle JVM task model")
            .setSource("jvm-host")
            .build());

        return builder.build();
    }

    private static List<BuildPlanTaskInputSpec> inputSpecs(Map<String, String> inputs) {
        List<BuildPlanTaskInputSpec> specs = new ArrayList<>();
        inputs.entrySet().stream()
            .sorted(Map.Entry.comparingByKey())
            .forEach(entry -> specs.add(BuildPlanTaskInputSpec.newBuilder()
                .setName(entry.getKey())
                .setKind("value")
                .setValue(entry.getValue())
                .setNormalization("scalar")
                .setOptionalInput(false)
                .build()));
        return specs;
    }

    private static List<BuildPlanTaskOutputSpec> outputSpecs(List<String> outputs) {
        List<BuildPlanTaskOutputSpec> specs = new ArrayList<>();
        for (int index = 0; index < outputs.size(); index++) {
            specs.add(BuildPlanTaskOutputSpec.newBuilder()
                .setName("output" + index)
                .setKind("path")
                .setPath(outputs.get(index))
                .build());
        }
        return specs;
    }

    private static List<String> taskDependencyPaths(Task task, org.gradle.api.tasks.TaskDependency dependency) {
        try {
            Set<? extends Task> dependencies = TaskDependencyUtil.getDependenciesForInternalUse(dependency, task);
            TreeSet<String> paths = new TreeSet<>();
            for (Task dependencyTask : dependencies) {
                paths.add(dependencyTask.getPath());
            }
            return new ArrayList<>(paths);
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to resolve task dependency paths for {}", task.getPath(), e);
            return new ArrayList<>();
        }
    }

    @Nullable
    private static FileCollection safeOutputFiles(Task task) {
        try {
            return task.getOutputs().getFiles();
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to resolve task outputs for {}", task.getPath(), e);
            return null;
        }
    }

    @Nullable
    private static FileCollection registeredFiles(Object value) {
        try {
            Object files = invoke(value, "getRegisteredFiles");
            return files instanceof FileCollection ? (FileCollection) files : null;
        } catch (RuntimeException e) {
            return null;
        }
    }

    private static List<String> fileCollectionPaths(@Nullable FileCollection files) {
        if (files == null) {
            return new ArrayList<>();
        }
        try {
            TreeSet<String> paths = new TreeSet<>();
            for (File file : files.getFiles()) {
                paths.add(file.getAbsolutePath());
            }
            return new ArrayList<>(paths);
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to resolve file collection paths", e);
            return new ArrayList<>();
        }
    }

    private static String cacheability(Task task, Class<?> taskType) {
        if (taskType.isAnnotationPresent(CacheableTask.class)) {
            return "cacheable-annotation";
        }
        try {
            if (task.getOutputs().getHasOutput()) {
                return "declared-outputs";
            }
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to inspect cacheability for {}", task.getPath(), e);
        }
        return "unknown";
    }

    private static String actionKind(Class<?> taskType) {
        String simpleName = taskType.getSimpleName();
        if ("JavaCompile".equals(simpleName)
            || "GroovyCompile".equals(simpleName)
            || "ScalaCompile".equals(simpleName)
            || "KotlinCompile".equals(simpleName)) {
            return "compile";
        }
        if ("Test".equals(simpleName)) {
            return "test";
        }
        if ("Copy".equals(simpleName) || "Sync".equals(simpleName)) {
            return "file-transform";
        }
        if ("Delete".equals(simpleName)) {
            return "delete";
        }
        if ("Jar".equals(simpleName)
            || "War".equals(simpleName)
            || "Ear".equals(simpleName)
            || "Zip".equals(simpleName)
            || "Tar".equals(simpleName)) {
            return "archive";
        }
        if ("Exec".equals(simpleName) || "JavaExec".equals(simpleName)) {
            return "external-process";
        }
        return "jvm-task";
    }

    private static String workerIsolation(Class<?> taskType) {
        String simpleName = taskType.getSimpleName();
        if ("JavaCompile".equals(simpleName)
            || "GroovyCompile".equals(simpleName)
            || "ScalaCompile".equals(simpleName)
            || "KotlinCompile".equals(simpleName)
            || "Test".equals(simpleName)
            || "Exec".equals(simpleName)
            || "JavaExec".equals(simpleName)
            || "Javadoc".equals(simpleName)
            || "Groovydoc".equals(simpleName)
            || "Scaladoc".equals(simpleName)) {
            return "process";
        }
        if ("Copy".equals(simpleName)
            || "Sync".equals(simpleName)
            || "Delete".equals(simpleName)
            || "Jar".equals(simpleName)
            || "War".equals(simpleName)
            || "Ear".equals(simpleName)
            || "Zip".equals(simpleName)
            || "Tar".equals(simpleName)) {
            return "in-process";
        }
        return "compat-jvm";
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
