package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlanTask;
import gradle.substrate.v1.BuildPlanTaskInputSpec;
import gradle.substrate.v1.BuildPlanTaskOutputSpec;

import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.file.FileCollection;
import org.gradle.api.tasks.TaskDependency;
import org.junit.Rule;
import org.junit.rules.TemporaryFolder;

import java.io.File;
import java.io.IOException;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Proxy;
import java.util.Arrays;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

public class ProjectModelProviderAdapterTest {
    @Rule
    public final TemporaryFolder temporaryFolder = new TemporaryFolder();

    @org.junit.Test
    public void capturesNativeReadyJavaCompileContractFromTaskModel() throws IOException {
        File sourceDir = temporaryFolder.newFolder("src", "main", "java");
        File sourceFile = new File(sourceDir, "App.java");
        assertTrue(sourceFile.createNewFile());
        File outputDir = temporaryFolder.newFolder("build/classes/java/main");
        File classpathEntry = temporaryFolder.newFolder("build/classes/java/dependency");

        Task compileJava = javaCompileTask(sourceFile, outputDir, classpathEntry);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(compileJava, JavaCompile.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":compileJava", task.getPath());
        assertEquals("compile", task.getActionKind());
        assertEquals("process", task.getWorkerIsolation());
        assertEquals("JavaCompile", inputs.get("taskType"));
        assertFalse(inputs.get("java_home").isEmpty());
        assertEquals("17", inputs.get("source_version"));
        assertEquals("17", inputs.get("target_version"));
        assertEquals("17", inputs.get("release"));
        assertEquals("UTF-8", inputs.get("encoding"));
        assertEquals(classpathEntry.getAbsolutePath(), inputs.get("classpath"));
        assertTrue(task.getInputSpecsList().stream()
            .anyMatch(input -> input.getKind().equals("source") && input.getValue().equals(sourceFile.getAbsolutePath())));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(outputDir.getAbsolutePath())));
    }

    @org.junit.Test
    public void capturesNativeReadyJarContractFromTaskModel() throws IOException {
        File classesDir = temporaryFolder.newFolder("build/classes/java/main");
        File classFile = new File(classesDir, "App.class");
        assertTrue(classFile.createNewFile());
        File archiveDir = temporaryFolder.newFolder("build/libs");
        File archiveFile = new File(archiveDir, "sample-1.0.jar");

        Task jar = jarTask(classesDir, archiveDir, archiveFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(jar, Jar.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":jar", task.getPath());
        assertEquals("archive", task.getActionKind());
        assertEquals("in-process", task.getWorkerIsolation());
        assertEquals("Jar", inputs.get("taskType"));
        assertEquals("sample-1.0.jar", inputs.get("archive_file_name"));
        assertEquals("sample", inputs.get("archive_base_name"));
        assertEquals("1.0", inputs.get("archive_version"));
        assertEquals("jar", inputs.get("archive_extension"));
        assertEquals(archiveDir.getAbsolutePath(), inputs.get("archive_destination_directory"));
        assertEquals(archiveFile.getAbsolutePath(), inputs.get("archive_file"));
        assertEquals("INCLUDE", inputs.get("duplicates_strategy"));
        assertEquals("true", inputs.get("include_empty_dirs"));
        assertEquals("420", inputs.get("file_permissions"));
        assertEquals("493", inputs.get("dir_permissions"));
        assertTrue(task.getInputSpecsList().stream()
            .anyMatch(input -> input.getKind().equals("path") && input.getValue().equals(classesDir.getAbsolutePath())));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(archiveFile.getAbsolutePath())));
    }

    @org.junit.Test
    public void capturesNativeReadyZipContractFromTaskModel() throws IOException {
        File inputDir = temporaryFolder.newFolder("build/install/app");
        File archiveDir = temporaryFolder.newFolder("build/distributions");
        File archiveFile = new File(archiveDir, "app.zip");

        Task zip = basicJarTask(fileCollection(inputDir), fileCollection(archiveFile), archiveDir, archiveFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(zip, Zip.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":jar", task.getPath());
        assertEquals("archive", task.getActionKind());
        assertEquals("in-process", task.getWorkerIsolation());
        assertEquals("Zip", inputs.get("taskType"));
        assertEquals("sample-1.0.jar", inputs.get("archive_file_name"));
        assertEquals(archiveDir.getAbsolutePath(), inputs.get("archive_destination_directory"));
        assertEquals(archiveFile.getAbsolutePath(), inputs.get("archive_file"));
    }

    @org.junit.Test
    public void capturesNativeReadyTarContractFromTaskModel() throws IOException {
        File inputDir = temporaryFolder.newFolder("build/install/app");
        File archiveDir = temporaryFolder.newFolder("build/distributions");
        File archiveFile = new File(archiveDir, "app.tar");

        Task tar = basicJarTask(fileCollection(inputDir), fileCollection(archiveFile), archiveDir, archiveFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(tar, Tar.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":jar", task.getPath());
        assertEquals("archive", task.getActionKind());
        assertEquals("in-process", task.getWorkerIsolation());
        assertEquals("Tar", inputs.get("taskType"));
        assertEquals("sample-1.0.jar", inputs.get("archive_file_name"));
        assertEquals("GZIP", inputs.get("archive_compression"));
        assertEquals(archiveDir.getAbsolutePath(), inputs.get("archive_destination_directory"));
        assertEquals(archiveFile.getAbsolutePath(), inputs.get("archive_file"));
    }

    @org.junit.Test
    public void capturesNativeReadyTestExecContractFromTaskModel() throws IOException {
        File testClassesDir = temporaryFolder.newFolder("build/classes/java/test");
        File runtimeJar = temporaryFolder.newFile("junit-platform-console-standalone.jar");
        File reportsDir = temporaryFolder.newFolder("build/test-results/test");
        File workingDir = temporaryFolder.newFolder("work");
        FileCollection classpath = fileCollection(testClassesDir, runtimeJar);

        Task test = testTask(testClassesDir, runtimeJar, reportsDir, workingDir);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(test, Test.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":test", task.getPath());
        assertEquals("test", task.getActionKind());
        assertEquals("process", task.getWorkerIsolation());
        assertEquals("Test", inputs.get("taskType"));
        assertEquals(fileCollectionPathStringForTest(classpath), inputs.get("classpath"));
        assertEquals(testClassesDir.getAbsolutePath(), inputs.get("test_classes_dirs"));
        assertEquals(workingDir.getAbsolutePath(), inputs.get("working_dir"));
        assertEquals("-ea -Dcustom=true", inputs.get("jvm_args"));
        assertEquals("env=test", inputs.get("system_properties"));
        assertEquals(reportsDir.getAbsolutePath(), inputs.get("xml_report_dir"));
        assertEquals("true", inputs.get("scan_classpath"));
    }

    @org.junit.Test
    public void capturesProcessResourcesAsNativeFileTransform() throws IOException {
        File resourceFile = temporaryFolder.newFile("application.properties");
        File outputDir = temporaryFolder.newFolder("build/resources/main");
        Map<String, String> properties = new LinkedHashMap<>();
        properties.put("appName", "corpus");
        properties.put("appVersion", "1.0");

        Task processResources = basicFileTransformTask(
            ":processResources",
            "processResources",
            fileCollection(resourceFile),
            fileCollection(outputDir),
            properties,
            true
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(processResources, ProcessResources.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("file-transform", task.getActionKind());
        assertEquals("in-process", task.getWorkerIsolation());
        assertEquals("appName=corpus,appVersion=1.0", inputs.get("expand_properties"));
        assertEquals("INCLUDE", inputs.get("duplicates_strategy"));
        assertEquals("**/*.properties", inputs.get("include_patterns"));
        assertEquals("**/secret.*", inputs.get("exclude_patterns"));
        assertEquals("false", inputs.get("case_sensitive"));
        assertEquals("true", inputs.get("include_empty_dirs"));
        assertEquals("420", inputs.get("file_permissions"));
        assertEquals("493", inputs.get("dir_permissions"));
        assertTrue(task.getInputSpecsList().stream()
            .anyMatch(input -> input.getKind().equals("path") && input.getValue().equals(resourceFile.getAbsolutePath())));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(outputDir.getAbsolutePath())));
    }

    @org.junit.Test
    public void capturesNoActionDefaultTaskAsLifecycleNoop() {
        Task classes = basicTask(":classes", "classes", fileCollection(), fileCollection(), true);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(classes, DefaultTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("lifecycle", task.getActionKind());
        assertEquals("0", inputs.get("action_count"));
    }

    private static Task javaCompileTask(File sourceFile, File outputDir, File classpathEntry) {
        FileCollection source = fileCollection(sourceFile);
        FileCollection classpath = fileCollection(classpathEntry);
        FileCollection outputs = fileCollection(outputDir);
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            return defaultValue(method.getReturnType());
        });
        TaskDependency noDependencies = proxy(TaskDependency.class, (proxy, method, args) -> {
            if (method.getName().equals("getDependencies")) {
                return Collections.emptySet();
            }
            return defaultValue(method.getReturnType());
        });
        return proxy(new Class<?>[] {Task.class, JavaCompileContract.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return ":compileJava";
                case "getProject":
                    return project;
                case "getName":
                    return "compileJava";
                case "getEnabled":
                    return true;
                case "getGroup":
                case "getDescription":
                    return "";
                case "getTaskDependencies":
                case "getShouldRunAfter":
                case "getMustRunAfter":
                case "getFinalizedBy":
                    return noDependencies;
                case "getInputs":
                    return filesOwner(method.getReturnType(), source);
                case "getOutputs":
                    return filesOwner(method.getReturnType(), outputs);
                case "getLocalState":
                case "getDestroyables":
                    return registeredFilesOwner(method.getReturnType());
                case "getSourceCompatibility":
                case "getTargetCompatibility":
                    return "17";
                case "getClasspath":
                    return classpath;
                case "getSource":
                    return source;
                case "getOptions":
                    return new CompileOptionsContract();
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Task jarTask(File classesDir, File archiveDir, File archiveFile) {
        FileCollection inputs = fileCollection(classesDir);
        FileCollection outputs = fileCollection(archiveFile);
        return basicJarTask(inputs, outputs, archiveDir, archiveFile);
    }

    private static Task testTask(File testClassesDir, File runtimeJar, File reportsDir, File workingDir) {
        FileCollection testClasses = fileCollection(testClassesDir);
        FileCollection classpath = fileCollection(testClassesDir, runtimeJar);
        FileCollection outputs = fileCollection(reportsDir);
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            return defaultValue(method.getReturnType());
        });
        TaskDependency noDependencies = proxy(TaskDependency.class, (proxy, method, args) -> {
            if (method.getName().equals("getDependencies")) {
                return Collections.emptySet();
            }
            return defaultValue(method.getReturnType());
        });
        return proxy(new Class<?>[] {Task.class, TestContract.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return ":test";
                case "getProject":
                    return project;
                case "getName":
                    return "test";
                case "getEnabled":
                    return true;
                case "getGroup":
                case "getDescription":
                    return "";
                case "getTaskDependencies":
                case "getShouldRunAfter":
                case "getMustRunAfter":
                case "getFinalizedBy":
                    return noDependencies;
                case "getInputs":
                    return filesOwner(method.getReturnType(), classpath);
                case "getOutputs":
                    return filesOwner(method.getReturnType(), outputs);
                case "getLocalState":
                case "getDestroyables":
                    return registeredFilesOwner(method.getReturnType());
                case "getClasspath":
                    return classpath;
                case "getTestClassesDirs":
                    return testClasses;
                case "getWorkingDir":
                    return workingDir;
                case "getMaxHeapSize":
                    return "768m";
                case "getJvmArgs":
                    return Arrays.asList("-ea", "-Dcustom=true");
                case "getSystemProperties":
                    return Collections.singletonMap("env", "test");
                case "getReports":
                    return new TestReports(reportsDir);
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Task basicJarTask(FileCollection inputs, FileCollection outputs, File archiveDir, File archiveFile) {
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            return defaultValue(method.getReturnType());
        });
        TaskDependency noDependencies = proxy(TaskDependency.class, (proxy, method, args) -> {
            if (method.getName().equals("getDependencies")) {
                return Collections.emptySet();
            }
            return defaultValue(method.getReturnType());
        });
        return proxy(new Class<?>[] {Task.class, JarContract.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return ":jar";
                case "getProject":
                    return project;
                case "getName":
                    return "jar";
                case "getEnabled":
                    return true;
                case "getGroup":
                case "getDescription":
                    return "";
                case "getTaskDependencies":
                case "getShouldRunAfter":
                case "getMustRunAfter":
                case "getFinalizedBy":
                    return noDependencies;
                case "getInputs":
                    return filesOwner(method.getReturnType(), inputs);
                case "getOutputs":
                    return filesOwner(method.getReturnType(), outputs);
                case "getLocalState":
                case "getDestroyables":
                    return registeredFilesOwner(method.getReturnType());
                case "getArchiveFileName":
                    return new ValueProvider("sample-1.0.jar");
                case "getArchiveBaseName":
                    return new ValueProvider("sample");
                case "getArchiveAppendix":
                case "getArchiveClassifier":
                    return new ValueProvider("");
                case "getArchiveVersion":
                    return new ValueProvider("1.0");
                case "getArchiveExtension":
                    return new ValueProvider("jar");
                case "getDestinationDirectory":
                    return new FileProvider(archiveDir);
                case "getArchiveFile":
                    return new FileProvider(archiveFile);
                case "getCompression":
                    return "GZIP";
                case "getRootSpec":
                    return copySpec(false);
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Task basicTask(
        String path,
        String name,
        FileCollection inputs,
        FileCollection outputs,
        boolean noActions
    ) {
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            return defaultValue(method.getReturnType());
        });
        TaskDependency noDependencies = proxy(TaskDependency.class, (proxy, method, args) -> {
            if (method.getName().equals("getDependencies")) {
                return Collections.emptySet();
            }
            return defaultValue(method.getReturnType());
        });
        return proxy(new Class<?>[] {Task.class, FileTransformTaskContract.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return path;
                case "getProject":
                    return project;
                case "getName":
                    return name;
                case "getEnabled":
                    return true;
                case "getGroup":
                case "getDescription":
                    return "";
                case "getTaskDependencies":
                case "getShouldRunAfter":
                case "getMustRunAfter":
                case "getFinalizedBy":
                    return noDependencies;
                case "getInputs":
                    return filesOwner(method.getReturnType(), inputs);
                case "getOutputs":
                    return filesOwner(method.getReturnType(), outputs);
                case "getLocalState":
                case "getDestroyables":
                    return registeredFilesOwner(method.getReturnType());
                case "getActions":
                    return noActions ? Collections.emptyList() : Collections.singletonList(new Object());
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Task basicFileTransformTask(
        String path,
        String name,
        FileCollection inputs,
        FileCollection outputs,
        Map<String, String> inputProperties,
        boolean customActions
    ) {
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            return defaultValue(method.getReturnType());
        });
        TaskDependency noDependencies = proxy(TaskDependency.class, (proxy, method, args) -> {
            if (method.getName().equals("getDependencies")) {
                return Collections.emptySet();
            }
            return defaultValue(method.getReturnType());
        });
        return proxy(new Class<?>[] {Task.class, FileTransformTaskContract.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return path;
                case "getProject":
                    return project;
                case "getName":
                    return name;
                case "getEnabled":
                    return true;
                case "getGroup":
                case "getDescription":
                    return "";
                case "getTaskDependencies":
                case "getShouldRunAfter":
                case "getMustRunAfter":
                case "getFinalizedBy":
                    return noDependencies;
                case "getInputs":
                    return filesOwner(method.getReturnType(), inputs, inputProperties);
                case "getOutputs":
                    return filesOwner(method.getReturnType(), outputs);
                case "getLocalState":
                case "getDestroyables":
                    return registeredFilesOwner(method.getReturnType());
                case "getRootSpec":
                    return copySpec(customActions);
                case "getActions":
                    return customActions ? Collections.singletonList(new Object()) : Collections.emptyList();
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Object filesOwner(Class<?> type, FileCollection files) {
        return filesOwner(type, files, Collections.emptyMap());
    }

    private static Object filesOwner(Class<?> type, FileCollection files, Map<String, String> properties) {
        return proxy(type, (proxy, method, args) -> {
            if (method.getName().equals("getFiles")) {
                return files;
            }
            if (method.getName().equals("getProperties")) {
                return properties;
            }
            if (method.getName().equals("getHasOutput")) {
                return true;
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static Object copySpec(boolean customActions) {
        return proxy(CopySpecContract.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "hasCustomActions":
                    return customActions;
                case "getDuplicatesStrategy":
                    return "INCLUDE";
                case "getFilteringCharset":
                    return "UTF-8";
                case "getIncludes":
                    return Collections.singleton("**/*.properties");
                case "getExcludes":
                    return Collections.singleton("**/secret.*");
                case "isCaseSensitive":
                    return false;
                case "isIncludeEmptyDirs":
                    return true;
                case "getFilePermissions":
                    return new PermissionProvider(0644);
                case "getDirPermissions":
                    return new PermissionProvider(0755);
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Object registeredFilesOwner(Class<?> type) {
        return proxy(type, (proxy, method, args) -> {
            if (method.getName().equals("getRegisteredFiles")) {
                return fileCollection();
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static FileCollection fileCollection(File... files) {
        Set<File> fileSet = new LinkedHashSet<>(Arrays.asList(files));
        return proxy(FileCollection.class, (proxy, method, args) -> {
            if (method.getName().equals("getFiles")) {
                return fileSet;
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static String fileCollectionPathStringForTest(FileCollection files) {
        return files.getFiles().stream()
            .map(File::getAbsolutePath)
            .collect(Collectors.joining(File.pathSeparator));
    }

    @SuppressWarnings("unchecked")
    private static <T> T proxy(Class<T> type, InvocationHandler handler) {
        return (T) proxy(new Class<?>[] {type}, handler);
    }

    @SuppressWarnings("unchecked")
    private static <T> T proxy(Class<?>[] types, InvocationHandler handler) {
        return (T) Proxy.newProxyInstance(
            ProjectModelProviderAdapterTest.class.getClassLoader(),
            types,
            (proxy, method, args) -> {
                if (method.getDeclaringClass().equals(Object.class)) {
                    switch (method.getName()) {
                        case "toString":
                            return types[0].getSimpleName() + "Proxy";
                        case "hashCode":
                            return System.identityHashCode(proxy);
                        case "equals":
                            return proxy == args[0];
                        default:
                            return defaultValue(method.getReturnType());
                    }
                }
                return handler.invoke(proxy, method, args);
            }
        );
    }

    private static Object defaultValue(Class<?> returnType) {
        if (!returnType.isPrimitive()) {
            return null;
        }
        if (returnType.equals(boolean.class)) {
            return false;
        }
        if (returnType.equals(char.class)) {
            return '\0';
        }
        if (returnType.equals(void.class)) {
            return null;
        }
        return 0;
    }

    public interface JavaCompileContract {
        String getSourceCompatibility();
        String getTargetCompatibility();
        FileCollection getClasspath();
        FileCollection getSource();
        CompileOptionsContract getOptions();
    }

    public interface JarContract {
        ValueProvider getArchiveFileName();
        ValueProvider getArchiveBaseName();
        ValueProvider getArchiveAppendix();
        ValueProvider getArchiveVersion();
        ValueProvider getArchiveClassifier();
        ValueProvider getArchiveExtension();
        FileProvider getDestinationDirectory();
        FileProvider getArchiveFile();
        Object getCompression();
        Object getRootSpec();
    }

    public interface TestContract {
        FileCollection getClasspath();
        FileCollection getTestClassesDirs();
        File getWorkingDir();
        String getMaxHeapSize();
        Iterable<String> getJvmArgs();
        Map<String, String> getSystemProperties();
        TestReports getReports();
    }

    public interface CopySpecContract {
        boolean hasCustomActions();
        Object getDuplicatesStrategy();
        String getFilteringCharset();
        Set<String> getIncludes();
        Set<String> getExcludes();
        boolean isCaseSensitive();
        boolean isIncludeEmptyDirs();
        Object getFilePermissions();
        Object getDirPermissions();
    }

    public interface FileTransformTaskContract {
        Object getRootSpec();
    }

    public static class CompileOptionsContract {
        public ReleaseProvider getRelease() {
            return new ReleaseProvider();
        }

        public String getEncoding() {
            return "UTF-8";
        }

        public FileCollection getAnnotationProcessorPath() {
            return fileCollection();
        }
    }

    public static class ReleaseProvider {
        public Integer getOrNull() {
            return 17;
        }
    }

    public static class ValueProvider {
        private final String value;

        ValueProvider(String value) {
            this.value = value;
        }

        public String getOrNull() {
            return value;
        }
    }

    public static class PermissionProvider {
        private final int value;

        PermissionProvider(int value) {
            this.value = value;
        }

        public Permission getOrNull() {
            return new Permission(value);
        }
    }

    public static class Permission {
        private final int value;

        Permission(int value) {
            this.value = value;
        }

        public int toUnixNumeric() {
            return value;
        }
    }

    public static class FileProvider {
        private final File file;

        FileProvider(File file) {
            this.file = file;
        }

        public RegularFile getOrNull() {
            return new RegularFile(file);
        }
    }

    public static class RegularFile {
        private final File file;

        RegularFile(File file) {
            this.file = file;
        }

        public File getAsFile() {
            return file;
        }
    }

    public static class TestReports {
        private final File reportDir;

        TestReports(File reportDir) {
            this.reportDir = reportDir;
        }

        public JunitXmlReport getJunitXml() {
            return new JunitXmlReport(reportDir);
        }
    }

    public static class JunitXmlReport {
        private final File reportDir;

        JunitXmlReport(File reportDir) {
            this.reportDir = reportDir;
        }

        public FileProvider getOutputLocation() {
            return new FileProvider(reportDir);
        }
    }

    public static class JavaCompile {
    }

    public static class Jar {
    }

    public static class Zip {
    }

    public static class Tar {
    }

    public static class Test {
    }

    public static class ProcessResources {
    }

    public static class DefaultTask {
    }
}
