package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlanTask;
import gradle.substrate.v1.BuildPlanTaskInputSpec;
import gradle.substrate.v1.BuildPlanTaskOutputSpec;

import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.file.FileCollection;
import org.gradle.api.tasks.TaskDependency;
import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

import java.io.File;
import java.io.IOException;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Proxy;
import java.util.Arrays;
import java.util.Collections;
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

    @Test
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

    @Test
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
        assertTrue(task.getInputSpecsList().stream()
            .anyMatch(input -> input.getKind().equals("path") && input.getValue().equals(classesDir.getAbsolutePath())));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(archiveFile.getAbsolutePath())));
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
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Object filesOwner(Class<?> type, FileCollection files) {
        return proxy(type, (proxy, method, args) -> {
            if (method.getName().equals("getFiles")) {
                return files;
            }
            if (method.getName().equals("getHasOutput")) {
                return true;
            }
            return defaultValue(method.getReturnType());
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

    public static class JavaCompile {
    }

    public static class Jar {
    }
}
