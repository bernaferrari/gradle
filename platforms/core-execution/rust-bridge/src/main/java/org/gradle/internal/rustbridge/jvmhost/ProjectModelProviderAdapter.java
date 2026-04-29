package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlanTask;
import gradle.substrate.v1.BuildPlanTaskDiagnostic;
import gradle.substrate.v1.BuildPlanTaskInputSpec;
import gradle.substrate.v1.BuildPlanTaskOutputSpec;

import org.gradle.api.Action;
import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.file.FileCollection;
import org.gradle.api.file.FileTree;
import org.gradle.api.file.RelativePath;
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
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Base64;
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
        return toBuildPlanTask(task, (List<String>) null);
    }

    private static BuildPlanTask toBuildPlanTask(Task task, @Nullable List<String> selectedDependencyPaths) {
        Class<?> taskType = GeneratedSubclasses.unpackType(task);
        return toBuildPlanTask(task, selectedDependencyPaths, taskType);
    }

    static BuildPlanTask toBuildPlanTask(Task task, Class<?> taskType) {
        return toBuildPlanTask(task, null, taskType);
    }

    private static BuildPlanTask toBuildPlanTask(
            Task task,
            @Nullable List<String> selectedDependencyPaths,
            Class<?> taskType
    ) {
        String taskTypeName = taskType.getName();
        String shortTaskTypeName = taskType.getSimpleName();
        Map<String, String> inputs = new LinkedHashMap<>();
        inputs.put("source", "jvm-task-model");
        inputs.put("enabled", Boolean.toString(task.getEnabled()));
        inputs.put("taskName", task.getName());
        inputs.put("taskType", shortTaskTypeName.isEmpty() ? taskTypeName : shortTaskTypeName);
        inputs.put("action_count", Integer.toString(taskActionCount(task)));
        String group = task.getGroup();
        if (group != null && !group.isEmpty()) {
            inputs.put("group", group);
        }
        String description = task.getDescription();
        if (description != null && !description.isEmpty()) {
            inputs.put("description", description);
        }
        if ("JavaCompile".equals(shortTaskTypeName)) {
            captureJavaCompileInputs(task, inputs);
        }
        if (isArchiveTask(shortTaskTypeName)) {
            captureJarInputs(task, inputs);
        }
        if (isFileTransformTask(shortTaskTypeName)) {
            captureFileTransformInputs(task, inputs);
        }
        if ("Test".equals(shortTaskTypeName)) {
            captureTestInputs(task, inputs);
        }
        if ("Exec".equals(shortTaskTypeName)) {
            captureExecInputs(task, inputs);
        }

        BuildPlanTask.Builder builder = BuildPlanTask.newBuilder()
            .setPath(task.getPath())
            .setProjectPath(task.getProject().getPath())
            .setImplementationId(taskTypeName)
            .setWorkerIsolation(workerIsolation(taskType))
            .setCacheability(cacheability(task, taskType))
            .setActionKind(actionKind(task, taskType))
            .putAllInputs(inputs);

        List<String> inputPaths = fileCollectionPaths(safeInputFiles(task));
        List<String> sourcePaths = "JavaCompile".equals(shortTaskTypeName)
            ? fileCollectionPaths(safeJavaCompileSource(task))
            : new ArrayList<>();
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
        builder.addAllInputSpecs(inputSpecs(inputs, inputPaths, sourcePaths));
        builder.addAllOutputSpecs(outputSpecs(outputPaths));
        builder.addDiagnostics(BuildPlanTaskDiagnostic.newBuilder()
            .setSeverity("info")
            .setCode("jvm-host-task-contract")
            .setMessage("Task contract captured from Gradle JVM task model")
            .setSource("jvm-host")
            .build());

        return builder.build();
    }

    private static void captureJavaCompileInputs(Task task, Map<String, String> inputs) {
        putIfPresent(inputs, "java_home", System.getProperty("java.home"));
        putIfPresent(inputs, "source_version", stringOrEmpty(invokeOptional(task, "getSourceCompatibility")));
        putIfPresent(inputs, "target_version", stringOrEmpty(invokeOptional(task, "getTargetCompatibility")));
        putIfPresent(inputs, "classpath", fileCollectionPathString(invokeOptional(task, "getClasspath")));

        Object options = invokeOptional(task, "getOptions");
        if (options != null) {
            putIfPresent(inputs, "release", providerValue(invokeOptional(options, "getRelease")));
            putIfPresent(inputs, "encoding", stringOrEmpty(invokeOptional(options, "getEncoding")));
            putIfPresent(inputs, "parameters", booleanString(invokeOptional(options, "isParameters")));
            putIfPresent(inputs, "processor_path", fileCollectionPathString(invokeOptional(options, "getAnnotationProcessorPath")));
        }
    }

    private static void captureJarInputs(Task task, Map<String, String> inputs) {
        putIfPresent(inputs, "archive_file_name", providerValue(invokeOptional(task, "getArchiveFileName")));
        putIfPresent(inputs, "archive_base_name", providerValue(invokeOptional(task, "getArchiveBaseName")));
        putIfPresent(inputs, "archive_appendix", providerValue(invokeOptional(task, "getArchiveAppendix")));
        putIfPresent(inputs, "archive_version", providerValue(invokeOptional(task, "getArchiveVersion")));
        putIfPresent(inputs, "archive_classifier", providerValue(invokeOptional(task, "getArchiveClassifier")));
        putIfPresent(inputs, "archive_extension", providerValue(invokeOptional(task, "getArchiveExtension")));
        putIfPresent(inputs, "archive_destination_directory", providerFilePath(invokeOptional(task, "getDestinationDirectory")));
        putIfPresent(inputs, "archive_file", providerFilePath(invokeOptional(task, "getArchiveFile")));
        putIfPresent(inputs, "archive_compression", stringOrEmpty(invokeOptional(task, "getCompression")));
        captureManifestInputs(task, inputs);
        Object rootSpec = invokeOptional(task, "getRootSpec");
        putIfPresent(inputs, "duplicates_strategy", stringOrEmpty(invokeOptional(rootSpec, "getDuplicatesStrategy")));
        putIfPresent(inputs, "include_empty_dirs", booleanString(invokeOptional(rootSpec, "isIncludeEmptyDirs")));
        putIfPresent(inputs, "file_permissions", permissionUnixMode(invokeOptional(rootSpec, "getFilePermissions")));
        putIfPresent(inputs, "dir_permissions", permissionUnixMode(invokeOptional(rootSpec, "getDirPermissions")));
        putIfPresent(inputs, "copy_file_mappings", nestedCopyFileMappings(rootSpec));
        inputs.put("copy_contains_symlinks", Boolean.toString(containsSymbolicLinks(safeInputFiles(task))));
    }

    private static String nestedCopyFileMappings(@Nullable Object rootSpec) {
        Object resolver = rootSpec == null ? null : invokeOptional(rootSpec, "buildRootResolver");
        if (resolver == null) {
            return "";
        }
        List<String> mappings = new ArrayList<>();
        boolean[] hasNestedDestination = new boolean[] { false };
        Action<Object> visitor = specResolver -> {
            Object destPathValue = invokeOptional(specResolver, "getDestPath");
            Object sourceValue = invokeOptional(specResolver, "getSource");
            if (!(destPathValue instanceof RelativePath) || !(sourceValue instanceof FileTree)) {
                return;
            }
            RelativePath destPath = (RelativePath) destPathValue;
            if (!destPath.getPathString().isEmpty()) {
                hasNestedDestination[0] = true;
            }
            ((FileTree) sourceValue).visit(details -> {
                RelativePath sourcePath = details.getRelativePath();
                RelativePath outputPath = destPath.append(!details.isDirectory(), sourcePath.getSegments());
                mappings.add(encodeMapping(details.getFile().getAbsolutePath())
                    + ">"
                    + encodeMapping(outputPath.getPathString())
                    + ">"
                    + (details.isDirectory() ? "D" : "F"));
            });
        };
        try {
            invoke(resolver, "walk", Action.class, visitor);
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to capture nested CopySpec mappings", e);
            return "";
        }
        return hasNestedDestination[0] ? String.join(",", mappings) : "";
    }

    private static String encodeMapping(String value) {
        return Base64.getUrlEncoder()
            .withoutPadding()
            .encodeToString(value.getBytes(StandardCharsets.UTF_8));
    }

    private static void captureManifestInputs(Task task, Map<String, String> inputs) {
        Object manifest = invokeOptional(task, "getManifest");
        Object attributes = manifest == null ? null : invokeOptional(manifest, "getAttributes");
        if (!(attributes instanceof Map)) {
            return;
        }
        for (Map.Entry<?, ?> entry : ((Map<?, ?>) attributes).entrySet()) {
            if (entry.getKey() == null || entry.getValue() == null) {
                continue;
            }
            String name = entry.getKey().toString();
            String value = entry.getValue().toString();
            if (name.isEmpty() || value.isEmpty()) {
                continue;
            }
            inputs.put("manifest." + name, value);
            if ("Main-Class".equalsIgnoreCase(name)) {
                inputs.put("main_class", value);
            }
        }
    }

    private static boolean isArchiveTask(String simpleName) {
        return "Jar".equals(simpleName)
            || "War".equals(simpleName)
            || "Ear".equals(simpleName)
            || "Zip".equals(simpleName)
            || "Tar".equals(simpleName);
    }

    private static boolean isFileTransformTask(String simpleName) {
        return "Copy".equals(simpleName) || "Sync".equals(simpleName) || "ProcessResources".equals(simpleName);
    }

    private static void captureFileTransformInputs(Task task, Map<String, String> inputs) {
        putIfPresent(inputs, "expand_properties", stringMap(safeInputProperties(task)));
        Object rootSpec = invokeOptional(task, "getRootSpec");
        boolean hasCustomActions = Boolean.TRUE.equals(invokeOptional(rootSpec, "hasCustomActions"));
        List<String> copyActionClasses = copyActionClassNames(rootSpec);
        inputs.put("copy_has_custom_actions", Boolean.toString(hasCustomActions));
        putIfPresent(inputs, "copy_custom_action_types", String.join(",", copyActionClasses));
        inputs.put("copy_unsupported_custom_actions", Boolean.toString(hasUnsupportedCopyActions(hasCustomActions, copyActionClasses)));
        putIfPresent(inputs, "duplicates_strategy", stringOrEmpty(invokeOptional(rootSpec, "getDuplicatesStrategy")));
        putIfPresent(inputs, "filtering_charset", stringOrEmpty(invokeOptional(rootSpec, "getFilteringCharset")));
        putIfPresent(inputs, "include_patterns", stringCollection(invokeOptional(rootSpec, "getIncludes")));
        putIfPresent(inputs, "exclude_patterns", stringCollection(invokeOptional(rootSpec, "getExcludes")));
        putIfPresent(inputs, "case_sensitive", booleanString(invokeOptional(rootSpec, "isCaseSensitive")));
        putIfPresent(inputs, "include_empty_dirs", booleanString(invokeOptional(rootSpec, "isIncludeEmptyDirs")));
        putIfPresent(inputs, "file_permissions", permissionUnixMode(invokeOptional(rootSpec, "getFilePermissions")));
        putIfPresent(inputs, "dir_permissions", permissionUnixMode(invokeOptional(rootSpec, "getDirPermissions")));
        putIfPresent(inputs, "copy_file_mappings", nestedCopyFileMappings(rootSpec));
        inputs.put("copy_contains_symlinks", Boolean.toString(containsSymbolicLinks(safeInputFiles(task))));
    }

    private static boolean containsSymbolicLinks(@Nullable FileCollection files) {
        if (files == null) {
            return false;
        }
        try {
            for (File file : files.getFiles()) {
                if (Files.isSymbolicLink(file.toPath())) {
                    return true;
                }
            }
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to inspect task inputs for symlinks", e);
        }
        return false;
    }

    private static List<String> copyActionClassNames(@Nullable Object rootSpec) {
        Object resolver = rootSpec == null ? null : invokeOptional(rootSpec, "buildRootResolver");
        Object actions = resolver == null ? null : invokeOptional(resolver, "getAllCopyActions");
        List<String> classes = new ArrayList<>();
        if (!(actions instanceof Iterable)) {
            return classes;
        }
        for (Object action : (Iterable<?>) actions) {
            if (action != null) {
                classes.add(action.getClass().getName());
            }
        }
        Collections.sort(classes);
        return classes;
    }

    private static boolean hasUnsupportedCopyActions(boolean hasCustomActions, List<String> copyActionClasses) {
        if (!hasCustomActions) {
            return false;
        }
        if (copyActionClasses.isEmpty()) {
            return true;
        }
        for (String className : copyActionClasses) {
            if (!className.endsWith("MapBackedExpandAction")) {
                return true;
            }
        }
        return false;
    }

    private static void captureTestInputs(Task task, Map<String, String> inputs) {
        putIfPresent(inputs, "java_home", System.getProperty("java.home"));
        putIfPresent(inputs, "classpath", fileCollectionPathString(invokeOptional(task, "getClasspath")));
        putIfPresent(inputs, "test_classes_dirs", fileCollectionPathString(invokeOptional(task, "getTestClassesDirs")));
        putIfPresent(inputs, "working_dir", filePath(invokeOptional(task, "getWorkingDir")));
        putIfPresent(inputs, "max_heap_size", stringOrEmpty(invokeOptional(task, "getMaxHeapSize")));
        putIfPresent(inputs, "jvm_args", stringList(invokeOptional(task, "getJvmArgs")));
        putIfPresent(inputs, "system_properties", stringMap(invokeOptional(task, "getSystemProperties")));
        putIfPresent(inputs, "xml_report_dir", testXmlReportDirectory(task));
        captureTestFilterInputs(task, inputs);
        Object options = invokeOptional(task, "getOptions");
        putIfPresent(inputs, "include_tags", stringCollection(invokeOptional(options, "getIncludeTags")));
        putIfPresent(inputs, "exclude_tags", stringCollection(invokeOptional(options, "getExcludeTags")));
        inputs.put("scan_classpath", "true");
    }

    private static void captureTestFilterInputs(Task task, Map<String, String> inputs) {
        Object filter = invokeOptional(task, "getFilter");
        List<String> includes = stringValues(invokeOptional(filter, "getIncludePatterns"));
        List<String> excludes = stringValues(invokeOptional(filter, "getExcludePatterns"));
        if (includes.size() == 1 && excludes.isEmpty()) {
            inputs.put("test_filter", includes.get(0));
            inputs.put("test_unsupported_filters", "false");
        } else if (includes.isEmpty() && excludes.isEmpty()) {
            inputs.put("test_unsupported_filters", "false");
        } else {
            putIfPresent(inputs, "test_filter_includes", String.join(",", includes));
            putIfPresent(inputs, "test_filter_excludes", String.join(",", excludes));
            inputs.put("test_unsupported_filters", "true");
        }
    }

    private static void captureExecInputs(Task task, Map<String, String> inputs) {
        putIfPresent(inputs, "executable", stringOrEmpty(invokeOptional(task, "getExecutable")));
        putIfPresent(inputs, "args", stringList(invokeOptional(task, "getArgs")));
        putIfPresent(inputs, "working_dir", filePath(invokeOptional(task, "getWorkingDir")));
        putIfPresent(inputs, "ignore_exit_value", booleanString(invokeOptional(task, "isIgnoreExitValue")));
    }

    private static void putIfPresent(Map<String, String> inputs, String key, String value) {
        if (value != null && !value.isEmpty()) {
            inputs.put(key, value);
        }
    }

    private static String booleanString(@Nullable Object value) {
        return value instanceof Boolean ? value.toString() : "";
    }

    private static String providerValue(@Nullable Object value) {
        if (value == null) {
            return "";
        }
        Object providerValue = invokeOptional(value, "getOrNull");
        return providerValue == null ? "" : providerValue.toString();
    }

    private static String permissionUnixMode(@Nullable Object value) {
        if (value == null) {
            return "";
        }
        Object permissions = invokeOptional(value, "getOrNull");
        if (permissions == null) {
            permissions = value;
        }
        Object unixNumeric = invokeOptional(permissions, "toUnixNumeric");
        return unixNumeric instanceof Number ? unixNumeric.toString() : "";
    }

    private static String providerFilePath(@Nullable Object value) {
        if (value == null) {
            return "";
        }
        Object providerValue = invokeOptional(value, "getOrNull");
        if (providerValue instanceof File) {
            return ((File) providerValue).getAbsolutePath();
        }
        Object file = invokeOptional(providerValue, "getAsFile");
        if (file instanceof File) {
            return ((File) file).getAbsolutePath();
        }
        return "";
    }

    private static String stringOrEmpty(@Nullable Object value) {
        return value == null ? "" : value.toString();
    }

    private static String fileCollectionPathString(@Nullable Object files) {
        if (!(files instanceof FileCollection)) {
            return "";
        }
        return String.join(File.pathSeparator, fileCollectionPaths((FileCollection) files));
    }

    private static String filePath(@Nullable Object value) {
        return value instanceof File ? ((File) value).getAbsolutePath() : "";
    }

    private static String stringList(@Nullable Object value) {
        if (!(value instanceof Iterable)) {
            return "";
        }
        List<String> values = new ArrayList<>();
        for (Object item : (Iterable<?>) value) {
            if (item != null) {
                values.add(item.toString());
            }
        }
        return String.join(" ", values);
    }

    private static String stringCollection(@Nullable Object value) {
        List<String> values = stringValues(value);
        return String.join(",", values);
    }

    private static List<String> stringValues(@Nullable Object value) {
        List<String> values = new ArrayList<>();
        if (!(value instanceof Iterable)) {
            return values;
        }
        for (Object item : (Iterable<?>) value) {
            if (item != null) {
                values.add(item.toString());
            }
        }
        Collections.sort(values);
        return values;
    }

    private static String stringMap(@Nullable Object value) {
        if (!(value instanceof Map)) {
            return "";
        }
        List<String> entries = new ArrayList<>();
        for (Map.Entry<?, ?> entry : ((Map<?, ?>) value).entrySet()) {
            if (entry.getKey() != null && entry.getValue() != null) {
                entries.add(entry.getKey() + "=" + entry.getValue());
            }
        }
        Collections.sort(entries);
        return String.join(",", entries);
    }

    private static String testXmlReportDirectory(Task task) {
        Object reports = invokeOptional(task, "getReports");
        Object junitXml = reports == null ? null : invokeOptional(reports, "getJunitXml");
        Object outputLocation = junitXml == null ? null : invokeOptional(junitXml, "getOutputLocation");
        return providerFilePath(outputLocation);
    }

    private static List<BuildPlanTaskInputSpec> inputSpecs(
            Map<String, String> inputs,
            List<String> inputPaths,
            List<String> sourcePaths
    ) {
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
        for (int index = 0; index < inputPaths.size(); index++) {
            specs.add(BuildPlanTaskInputSpec.newBuilder()
                .setName("input" + index)
                .setKind("path")
                .setValue(inputPaths.get(index))
                .setNormalization("absolute-path")
                .setOptionalInput(false)
                .build());
        }
        for (int index = 0; index < sourcePaths.size(); index++) {
            specs.add(BuildPlanTaskInputSpec.newBuilder()
                .setName("source" + index)
                .setKind("source")
                .setValue(sourcePaths.get(index))
                .setNormalization("absolute-path")
                .setOptionalInput(false)
                .build());
        }
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
    private static FileCollection safeInputFiles(Task task) {
        try {
            return task.getInputs().getFiles();
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to resolve task inputs for {}", task.getPath(), e);
            return null;
        }
    }

    @Nullable
    private static Map<?, ?> safeInputProperties(Task task) {
        try {
            Object properties = invoke(task.getInputs(), "getProperties");
            return properties instanceof Map ? (Map<?, ?>) properties : null;
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to resolve task input properties for {}", task.getPath(), e);
            return null;
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
    private static FileCollection safeJavaCompileSource(Task task) {
        Object source = invokeOptional(task, "getSource");
        return source instanceof FileCollection ? (FileCollection) source : null;
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

    private static int taskActionCount(Task task) {
        try {
            return task.getActions().size();
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Failed to inspect task actions for {}", task.getPath(), e);
            return -1;
        }
    }

    private static String actionKind(Task task, Class<?> taskType) {
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
        if ("Copy".equals(simpleName) || "Sync".equals(simpleName) || "ProcessResources".equals(simpleName)) {
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
        if (("DefaultTask".equals(simpleName) || "Task".equals(simpleName)) && taskActionCount(task) == 0) {
            return "lifecycle";
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
            || "ProcessResources".equals(simpleName)
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

    @Nullable
    private static Object invokeOptional(@Nullable Object target, String methodName) {
        if (target == null) {
            return null;
        }
        try {
            Method method = target.getClass().getMethod(methodName);
            return method.invoke(target);
        } catch (IllegalAccessException | InvocationTargetException | NoSuchMethodException | RuntimeException e) {
            LOGGER.debug("[substrate-jvmhost] Optional method unavailable: {}.{}", target.getClass().getName(), methodName, e);
            return null;
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
