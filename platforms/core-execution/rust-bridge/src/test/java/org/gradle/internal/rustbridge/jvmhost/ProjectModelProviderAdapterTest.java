package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlanTask;
import gradle.substrate.v1.BuildPlanTaskInputSpec;
import gradle.substrate.v1.BuildPlanTaskOutputSpec;

import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.artifacts.Configuration;
import org.gradle.api.artifacts.ConfigurationContainer;
import org.gradle.api.artifacts.VersionConstraint;
import org.gradle.api.file.FileCollection;
import org.gradle.api.tasks.TaskDependency;
import org.junit.Rule;
import org.junit.rules.TemporaryFolder;

import java.io.File;
import java.io.IOException;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.Arrays;
import java.util.Base64;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assume.assumeTrue;

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
        File javaHome = temporaryFolder.newFolder("jdks", "jdk-17");

        Task compileJava = javaCompileTask(sourceFile, outputDir, classpathEntry, javaHome);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(compileJava, JavaCompile.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":compileJava", task.getPath());
        assertEquals("compile", task.getActionKind());
        assertEquals("process", task.getWorkerIsolation());
        assertEquals("JavaCompile", inputs.get("taskType"));
        assertEquals(javaHome.getAbsolutePath(), inputs.get("java_home"));
        assertEquals("17", inputs.get("source_version"));
        assertEquals("17", inputs.get("target_version"));
        assertEquals("17", inputs.get("release"));
        assertEquals("UTF-8", inputs.get("encoding"));
        assertEquals(classpathEntry.getAbsolutePath(), inputs.get("classpath"));
        assertEquals("true", inputs.get("debug"));
        assertEquals("-parameters", inputs.get("compiler_args"));
        assertEquals("[\"-parameters\"]", inputs.get("compiler_args_json"));
        assertTrue(task.getInputSpecsList().stream()
            .anyMatch(input -> input.getKind().equals("source") && input.getValue().equals(sourceFile.getAbsolutePath())));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(outputDir.getAbsolutePath())));
    }

    @org.junit.Test
    public void augmentsStandardJavaCompileClasspathFromResolvableConfiguration() throws IOException {
        File sourceDir = temporaryFolder.newFolder("src", "test", "java");
        File sourceFile = new File(sourceDir, "AppTest.java");
        assertTrue(sourceFile.createNewFile());
        File outputDir = temporaryFolder.newFolder("build/classes/java/test");
        File mainClasses = temporaryFolder.newFolder("build/classes/java/main");
        File junitJar = temporaryFolder.newFile("junit-jupiter-api.jar");
        File javaHome = temporaryFolder.newFolder("jdks", "jdk-17");

        Task compileTestJava = javaCompileTask(
            ":compileTestJava",
            "compileTestJava",
            sourceFile,
            outputDir,
            fileCollection(mainClasses),
            javaHome,
            configurationContainer("testCompileClasspath", fileCollection(mainClasses, junitJar))
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(compileTestJava, JavaCompile.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(
            fileCollectionPathStringForTest(fileCollection(mainClasses, junitJar)),
            inputs.get("classpath")
        );
    }

    @org.junit.Test
    public void capturesSourceFilesForJvmLanguageCompileTasks() throws IOException {
        File sourceDir = temporaryFolder.newFolder("src", "main", "kotlin");
        File sourceFile = new File(sourceDir, "App.kt");
        assertTrue(sourceFile.createNewFile());
        File outputDir = temporaryFolder.newFolder("build/classes/kotlin/main");
        File classpathEntry = temporaryFolder.newFolder("build/classes/java/main");
        File javaHome = temporaryFolder.newFolder("jdks", "jdk-17");

        Task compileKotlin = javaCompileTask(":compileKotlin", "compileKotlin", sourceFile, outputDir, fileCollection(classpathEntry), javaHome, null);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(compileKotlin, KotlinCompile.class);

        assertEquals("compile", task.getActionKind());
        assertTrue(task.getInputSpecsList().stream()
            .anyMatch(input -> input.getKind().equals("source") && input.getValue().equals(sourceFile.getAbsolutePath())));
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
        assertEquals("com.example.Main", inputs.get("main_class"));
        assertEquals("com.example.Main", inputs.get("manifest.Main-Class"));
        assertEquals("sample", inputs.get("manifest.Implementation-Title"));
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
        assertEquals("[\"-ea\",\"-Dcustom=true\"]", inputs.get("jvm_args_json"));
        assertEquals("env=test", inputs.get("system_properties"));
        assertEquals(reportsDir.getAbsolutePath(), inputs.get("xml_report_dir"));
        assertEquals("example.*Test", inputs.get("test_filter"));
        assertEquals("example.*Test", inputs.get("test_filter_includes"));
        assertEquals("fast,integration", inputs.get("include_tags"));
        assertEquals("slow", inputs.get("exclude_tags"));
        assertEquals("false", inputs.get("test_unsupported_filters"));
        assertEquals("true", inputs.get("scan_classpath"));
    }

    @org.junit.Test
    public void capturesClassNameTestIncludeAndExcludeFiltersAsNativeReady() throws IOException {
        File testClassesDir = temporaryFolder.newFolder("build/classes/java/test");
        File runtimeJar = temporaryFolder.newFile("junit-platform-console-standalone.jar");
        File reportsDir = temporaryFolder.newFolder("build/test-results/test");
        File workingDir = temporaryFolder.newFolder("work");

        Task test = testTask(
            testClassesDir,
            runtimeJar,
            reportsDir,
            workingDir,
            null,
            new TestFilterSpec(
                new LinkedHashSet<>(Arrays.asList("com.example.*Test")),
                new LinkedHashSet<>(Arrays.asList("com.example.Legacy*"))
            )
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(test, Test.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("com.example.*Test", inputs.get("test_filter_includes"));
        assertEquals("com.example.Legacy*", inputs.get("test_filter_excludes"));
        assertFalse(inputs.containsKey("test_filter"));
        assertEquals("false", inputs.get("test_unsupported_filters"));
    }

    @org.junit.Test
    public void augmentsStandardTestClasspathFromResolvableConfiguration() throws IOException {
        File testClassesDir = temporaryFolder.newFolder("build/classes/java/test");
        File runtimeJar = temporaryFolder.newFile("junit-platform-console-standalone.jar");
        File engineJar = temporaryFolder.newFile("junit-jupiter-engine.jar");
        File reportsDir = temporaryFolder.newFolder("build/test-results/test");
        File workingDir = temporaryFolder.newFolder("work");

        Task test = testTask(
            testClassesDir,
            runtimeJar,
            reportsDir,
            workingDir,
            configurationContainer("testRuntimeClasspath", fileCollection(testClassesDir, runtimeJar, engineJar))
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(test, Test.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(
            fileCollectionPathStringForTest(fileCollection(testClassesDir, runtimeJar, engineJar)),
            inputs.get("classpath")
        );
    }

    @org.junit.Test
    public void extractsOnlyStaticMavenConstraintVersionsForBuildPlanConstraints() {
        assertEquals(
            "3.14.0",
            ProjectModelProviderAdapter.staticMavenConstraintVersion(versionConstraint("", "3.14.0", "", null, Collections.emptyList()))
        );
        assertEquals(
            "3.14.0",
            ProjectModelProviderAdapter.staticMavenConstraintVersion(versionConstraint("3.14.0", "", "", null, Collections.emptyList()))
        );
        assertEquals(
            "3.14.0",
            ProjectModelProviderAdapter.staticMavenConstraintVersion(versionConstraint("", "", "3.14.0", null, Collections.emptyList()))
        );
        assertEquals(
            null,
            ProjectModelProviderAdapter.staticMavenConstraintVersion(versionConstraint("", "3.+", "", null, Collections.emptyList()))
        );
        assertEquals(
            null,
            ProjectModelProviderAdapter.staticMavenConstraintVersion(versionConstraint("", "3.14.0", "", "main", Collections.emptyList()))
        );
        assertEquals(
            null,
            ProjectModelProviderAdapter.staticMavenConstraintVersion(versionConstraint("", "3.14.0", "", null, Collections.singletonList("3.13.0")))
        );
    }

    @org.junit.Test
    public void capturesSimpleExecContractFromTaskModel() throws IOException {
        File outputDir = temporaryFolder.newFolder("build/exec");
        File workingDir = temporaryFolder.newFolder("work-exec");

        Task exec = execTask(outputDir, workingDir);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(exec, Exec.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":generateFile", task.getPath());
        assertEquals("external-process", task.getActionKind());
        assertEquals("process", task.getWorkerIsolation());
        assertEquals("Exec", inputs.get("taskType"));
        assertEquals("/usr/bin/touch", inputs.get("executable"));
        assertEquals("generated file.txt", inputs.get("args"));
        assertEquals("[\"generated file.txt\"]", inputs.get("args_json"));
        assertEquals("NATIVE_EXEC_ENV=from-task", inputs.get("environment"));
        assertEquals("{\"NATIVE_EXEC_ENV\":\"from-task\"}", inputs.get("environment_json"));
        assertEquals(workingDir.getAbsolutePath(), inputs.get("working_dir"));
        assertEquals("false", inputs.get("ignore_exit_value"));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(outputDir.getAbsolutePath())));
    }

    @org.junit.Test
    public void capturesNativeReadyJavaExecContractFromTaskModel() throws IOException {
        File classesDir = temporaryFolder.newFolder("build/classes/java/main");
        File outputDir = temporaryFolder.newFolder("build/resources");
        File outputFile = new File(outputDir, "javaexec-result.txt");
        assertTrue(outputFile.createNewFile());
        File workingDir = temporaryFolder.newFolder("work-javaexec");

        Task javaExec = javaExecTask(classesDir, outputFile, workingDir);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(javaExec, JavaExec.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":runTool", task.getPath());
        assertEquals("external-process", task.getActionKind());
        assertEquals("process", task.getWorkerIsolation());
        assertEquals("JavaExec", inputs.get("taskType"));
        assertFalse(inputs.get("java_home").isEmpty());
        assertEquals(classesDir.getAbsolutePath(), inputs.get("classpath"));
        assertEquals("example.Tool", inputs.get("main_class"));
        assertEquals(outputFile.getAbsolutePath() + " expected token", inputs.get("args"));
        assertEquals("[\"" + outputFile.getAbsolutePath().replace("\\", "\\\\") + "\",\"expected token\"]", inputs.get("args_json"));
        assertEquals("-Dnative=true -Xmx128m", inputs.get("jvm_args"));
        assertEquals("[\"-Dnative=true\",\"-Xmx128m\"]", inputs.get("jvm_args_json"));
        assertEquals("256m", inputs.get("max_heap_size"));
        assertEquals("native.prop=from-task", inputs.get("system_properties"));
        assertEquals("NATIVE_JAVA_EXEC_ENV=from-task", inputs.get("environment"));
        assertEquals("{\"NATIVE_JAVA_EXEC_ENV\":\"from-task\"}", inputs.get("environment_json"));
        assertEquals(workingDir.getAbsolutePath(), inputs.get("working_dir"));
        assertEquals("false", inputs.get("ignore_exit_value"));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(outputFile.getAbsolutePath())));
    }

    @org.junit.Test
    public void capturesNativeReadyStartScriptsContractFromTaskModel() throws IOException {
        File jarFile = temporaryFolder.newFile("corpus-app-1.0.jar");
        File outputDir = temporaryFolder.newFolder("build/scripts");

        Task startScripts = startScriptsTask(jarFile, outputDir);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(startScripts, CreateStartScripts.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":startScripts", task.getPath());
        assertEquals("start-scripts", task.getActionKind());
        assertEquals("in-process", task.getWorkerIsolation());
        assertEquals("CreateStartScripts", inputs.get("taskType"));
        assertEquals("corpus-app", inputs.get("application_name"));
        assertEquals("example.App", inputs.get("main_class"));
        assertEquals(jarFile.getAbsolutePath(), inputs.get("classpath"));
        assertEquals(outputDir.getAbsolutePath(), inputs.get("output_dir"));
        assertEquals("bin", inputs.get("executable_dir"));
        assertEquals("-Ddemo=true -Xmx128m", inputs.get("default_jvm_opts"));
        assertEquals("CORPUS_APP_OPTS", inputs.get("opts_environment_var"));
        assertEquals("HEAD", inputs.get("git_ref"));
        assertEquals(new File(outputDir, "corpus-app").getAbsolutePath(), inputs.get("unix_script"));
        assertEquals(new File(outputDir, "corpus-app.bat").getAbsolutePath(), inputs.get("windows_script"));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(outputDir.getAbsolutePath())));
    }

    @org.junit.Test
    public void capturesNativeReadyJavadocContractFromTaskModel() throws IOException {
        File sourceDir = temporaryFolder.newFolder("src", "main", "java");
        File sourceFile = new File(sourceDir, "App.java");
        assertTrue(sourceFile.createNewFile());
        File classesDir = temporaryFolder.newFolder("build/classes/java/main");
        File docsDir = temporaryFolder.newFolder("build/docs/javadoc");

        Task javadoc = javadocTask(sourceFile, classesDir, docsDir);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(javadoc, Javadoc.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":javadoc", task.getPath());
        assertEquals("documentation", task.getActionKind());
        assertEquals("process", task.getWorkerIsolation());
        assertEquals("Javadoc", inputs.get("taskType"));
        assertFalse(inputs.get("java_home").isEmpty());
        assertEquals(classesDir.getAbsolutePath(), inputs.get("classpath"));
        assertEquals(docsDir.getAbsolutePath(), inputs.get("destination_dir"));
        assertEquals("API", inputs.get("title"));
        assertEquals("UTF-8", inputs.get("encoding"));
        assertEquals("true", inputs.get("no_timestamp"));
        assertTrue(task.getInputSpecsList().stream()
            .anyMatch(input -> input.getKind().equals("source") && input.getValue().equals(sourceFile.getAbsolutePath())));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(docsDir.getAbsolutePath())));
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
            true,
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
        assertEquals("false", inputs.get("copy_contains_symlinks"));
        assertEquals("true", inputs.get("copy_has_custom_actions"));
        assertEquals("false", inputs.get("copy_unsupported_custom_actions"));
        assertTrue(inputs.get("copy_custom_action_types").endsWith("MapBackedExpandAction"));
        assertTrue(task.getInputSpecsList().stream()
            .anyMatch(input -> input.getKind().equals("path") && input.getValue().equals(resourceFile.getAbsolutePath())));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(outputDir.getAbsolutePath())));
    }

    @org.junit.Test
    public void marksUnsupportedCopyActionsAsNotNativeReady() throws IOException {
        File resourceFile = temporaryFolder.newFile("custom.txt");
        File outputDir = temporaryFolder.newFolder("build/custom");

        Task copy = basicFileTransformTask(
            ":copyCustom",
            "copyCustom",
            fileCollection(resourceFile),
            fileCollection(outputDir),
            Collections.emptyMap(),
            true,
            false
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(copy, Copy.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("copy_has_custom_actions"));
        assertEquals("true", inputs.get("copy_unsupported_custom_actions"));
        assertTrue(inputs.get("copy_custom_action_types").endsWith("ArbitraryCopyAction"));
    }

    @org.junit.Test
    public void capturesStaticEachFileRelativePathRewriteAsNativeReadyMappings() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Arrays.asList(
            "plugins { base }",
            "tasks.register<Copy>(\"copyWithAction\") {",
            "    from(\"src/raw\")",
            "    into(layout.buildDirectory.dir(\"rewritten\"))",
            "    eachFile {",
            "        relativePath = RelativePath(true, \"renamed\", name)",
            "    }",
            "}"
        ), StandardCharsets.UTF_8);
        File resourceFile = temporaryFolder.newFile("message.txt");
        File outputDir = temporaryFolder.newFolder("build/rewritten");

        Task copy = basicFileTransformTask(
            ":copyWithAction",
            "copyWithAction",
            fileCollection(resourceFile),
            fileCollection(outputDir),
            Collections.emptyMap(),
            true,
            false,
            buildFile
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(copy, Copy.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("copy_has_custom_actions"));
        assertEquals("false", inputs.get("copy_unsupported_custom_actions"));
        assertTrue(inputs.get("copy_custom_action_types").endsWith("ArbitraryCopyAction"));
        String[] mapping = inputs.get("copy_file_mappings").split(">");
        assertEquals(resourceFile.getAbsolutePath(), decodeMapping(mapping[0]));
        assertEquals("renamed/message.txt", decodeMapping(mapping[1]));
        assertEquals("F", mapping[2]);
    }

    @org.junit.Test
    public void capturesStaticLineReplaceFilterAsNativeReady() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Arrays.asList(
            "plugins { base }",
            "tasks.register<Copy>(\"copyFiltered\") {",
            "    from(\"src/raw\")",
            "    into(layout.buildDirectory.dir(\"filtered\"))",
            "    filter { line: String -> line.replace(\"TOKEN\", \"native-copy-filter\") }",
            "}"
        ), StandardCharsets.UTF_8);
        File resourceFile = temporaryFolder.newFile("message.txt");
        File outputDir = temporaryFolder.newFolder("build/filtered");

        Task copy = basicFileTransformTask(
            ":copyFiltered",
            "copyFiltered",
            fileCollection(resourceFile),
            fileCollection(outputDir),
            Collections.emptyMap(),
            true,
            false,
            buildFile
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(copy, Copy.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("copy_has_custom_actions"));
        assertEquals("false", inputs.get("copy_unsupported_custom_actions"));
        assertTrue(inputs.get("copy_custom_action_types").endsWith("ArbitraryCopyAction"));
        String[] replacement = inputs.get("copy_line_replace_filter").split(">");
        assertEquals("TOKEN", decodeMapping(replacement[0]));
        assertEquals("native-copy-filter", decodeMapping(replacement[1]));
    }

    @org.junit.Test
    public void marksFileTransformInputsContainingSymlinksAsNotNativeReady() throws IOException {
        File target = temporaryFolder.newFile("target.txt");
        File link = new File(temporaryFolder.getRoot(), "link.txt");
        try {
            Files.createSymbolicLink(link.toPath(), target.toPath());
        } catch (UnsupportedOperationException | SecurityException | IOException e) {
            assumeTrue("symlink creation unavailable: " + e, false);
        }
        File outputDir = temporaryFolder.newFolder("build/symlink-copy");

        Task copy = basicFileTransformTask(
            ":copySymlink",
            "copySymlink",
            fileCollection(link),
            fileCollection(outputDir),
            Collections.emptyMap(),
            false,
            true
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(copy, Copy.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("copy_contains_symlinks"));
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

    @org.junit.Test
    public void capturesStaticWriteFileDefaultTaskContract() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "tasks.register(\"apiContractReport\") { doLast { output.writeText(\"oss-style api contract\\n\") } }"
        ), StandardCharsets.UTF_8);
        File outputFile = new File(temporaryFolder.getRoot(), "build/reports/api-contract.txt");
        Task report = staticWriteFileTask(buildFile, outputFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(report, DefaultTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("jvm-task", task.getActionKind());
        assertEquals("1", inputs.get("action_count"));
        assertEquals("b3NzLXN0eWxlIGFwaSBjb250cmFjdAo=", inputs.get("static_output_text_b64"));
        assertTrue(task.getOutputSpecsList().stream()
            .map(BuildPlanTaskOutputSpec::getPath)
            .anyMatch(path -> path.equals(outputFile.getAbsolutePath())));
    }

    @org.junit.Test
    public void marksComponentMetadataRulesAsUnsupportedDependencySemantics() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "dependencies { components { all { status = \"release\" } } }"
        ), StandardCharsets.UTF_8);
        Task task = basicFileTransformTask(
            ":classes",
            "classes",
            fileCollection(),
            fileCollection(),
            Collections.emptyMap(),
            false,
            false,
            buildFile
        );

        BuildPlanTask planTask = ProjectModelProviderAdapter.toBuildPlanTask(task, DefaultTask.class);
        Map<String, String> inputs = planTask.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("unsupported_dependency_semantics"));
        assertTrue(inputs.get("unsupported_repository_features").contains("component-metadata-rule:build-script"));
    }

    @org.junit.Test
    public void marksArtifactTransformsAsUnsupportedDependencySemantics() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "dependencies { registerTransform(MarkerTransform::class) { from.attribute(kind, \"jar\"); to.attribute(kind, \"marker\") } }"
        ), StandardCharsets.UTF_8);
        Task task = basicFileTransformTask(
            ":classes",
            "classes",
            fileCollection(),
            fileCollection(),
            Collections.emptyMap(),
            false,
            false,
            buildFile
        );

        BuildPlanTask planTask = ProjectModelProviderAdapter.toBuildPlanTask(task, DefaultTask.class);
        Map<String, String> inputs = planTask.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("unsupported_dependency_semantics"));
        assertTrue(inputs.get("unsupported_repository_features").contains("artifact-transform:build-script"));
    }

    private static Task javaCompileTask(File sourceFile, File outputDir, File classpathEntry, File javaHome) {
        return javaCompileTask(
            ":compileJava",
            "compileJava",
            sourceFile,
            outputDir,
            fileCollection(classpathEntry),
            javaHome,
            null
        );
    }

    private static Task javaCompileTask(
        String path,
        String name,
        File sourceFile,
        File outputDir,
        FileCollection classpath,
        File javaHome,
        ConfigurationContainer configurations
    ) {
        FileCollection source = fileCollection(sourceFile);
        FileCollection outputs = fileCollection(outputDir);
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            if (method.getName().equals("getConfigurations")) {
                return configurations;
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
                case "getJavaCompiler":
                    return new JavaCompilerProvider(javaHome);
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
        return testTask(testClassesDir, runtimeJar, reportsDir, workingDir, null);
    }

    private static Task testTask(
        File testClassesDir,
        File runtimeJar,
        File reportsDir,
        File workingDir,
        ConfigurationContainer configurations
    ) {
        return testTask(testClassesDir, runtimeJar, reportsDir, workingDir, configurations, new TestFilterSpec());
    }

    private static Task testTask(
        File testClassesDir,
        File runtimeJar,
        File reportsDir,
        File workingDir,
        ConfigurationContainer configurations,
        TestFilterSpec filterSpec
    ) {
        FileCollection testClasses = fileCollection(testClassesDir);
        FileCollection classpath = fileCollection(testClassesDir, runtimeJar);
        FileCollection outputs = fileCollection(reportsDir);
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            if (method.getName().equals("getConfigurations")) {
                return configurations;
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
                case "getFilter":
                    return filterSpec;
                case "getOptions":
                    return new JUnitPlatformOptionsSpec();
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Task execTask(File outputDir, File workingDir) {
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
        return proxy(new Class<?>[] {Task.class, ExecContract.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return ":generateFile";
                case "getProject":
                    return project;
                case "getName":
                    return "generateFile";
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
                    return filesOwner(method.getReturnType(), fileCollection());
                case "getOutputs":
                    return filesOwner(method.getReturnType(), outputs);
                case "getLocalState":
                case "getDestroyables":
                    return registeredFilesOwner(method.getReturnType());
                case "getExecutable":
                    return "/usr/bin/touch";
                case "getArgs":
                    return Collections.singletonList("generated file.txt");
                case "getEnvironment":
                    return Collections.singletonMap("NATIVE_EXEC_ENV", "from-task");
                case "getWorkingDir":
                    return workingDir;
                case "isIgnoreExitValue":
                    return false;
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Task javaExecTask(File classesDir, File outputFile, File workingDir) {
        FileCollection classpath = fileCollection(classesDir);
        FileCollection outputs = fileCollection(outputFile);
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
        return proxy(new Class<?>[] {Task.class, JavaExecContract.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return ":runTool";
                case "getProject":
                    return project;
                case "getName":
                    return "runTool";
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
                case "getMainClass":
                    return new ValueProvider("example.Tool");
                case "getArgs":
                    return Arrays.asList(outputFile.getAbsolutePath(), "expected token");
                case "getJvmArgs":
                    return Arrays.asList("-Dnative=true", "-Xmx128m");
                case "getMaxHeapSize":
                    return "256m";
                case "getSystemProperties":
                    return Collections.singletonMap("native.prop", "from-task");
                case "getEnvironment":
                    return Collections.singletonMap("NATIVE_JAVA_EXEC_ENV", "from-task");
                case "getWorkingDir":
                    return workingDir;
                case "isIgnoreExitValue":
                    return false;
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Task startScriptsTask(File jarFile, File outputDir) {
        FileCollection classpath = fileCollection(jarFile);
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
        return proxy(new Class<?>[] {Task.class, CreateStartScriptsContract.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return ":startScripts";
                case "getProject":
                    return project;
                case "getName":
                    return "startScripts";
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
                case "getApplicationName":
                    return "corpus-app";
                case "getMainClass":
                    return new ValueProvider("example.App");
                case "getMainModule":
                    return new ValueProvider("");
                case "getClasspath":
                    return classpath;
                case "getOutputDir":
                    return outputDir;
                case "getExecutableDir":
                    return "bin";
                case "getDefaultJvmOpts":
                    return Arrays.asList("-Ddemo=true", "-Xmx128m");
                case "getOptsEnvironmentVar":
                    return "CORPUS_APP_OPTS";
                case "getGitRef":
                    return new ValueProvider("HEAD");
                case "getUnixScript":
                    return new File(outputDir, "corpus-app");
                case "getWindowsScript":
                    return new File(outputDir, "corpus-app.bat");
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Task javadocTask(File sourceFile, File classesDir, File docsDir) {
        FileCollection source = fileCollection(sourceFile);
        FileCollection classpath = fileCollection(classesDir);
        FileCollection outputs = fileCollection(docsDir);
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
        return proxy(new Class<?>[] {Task.class, JavadocContract.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return ":javadoc";
                case "getProject":
                    return project;
                case "getName":
                    return "javadoc";
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
                case "getSource":
                    return source;
                case "getClasspath":
                    return classpath;
                case "getDestinationDir":
                    return docsDir;
                case "getTitle":
                    return "API";
                case "getMaxMemory":
                    return "256m";
                case "getOptions":
                    return new JavadocOptionsContract();
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
                case "getManifest":
                    return new ManifestSpec();
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

    private static Task staticWriteFileTask(File buildFile, File outputFile) {
        FileCollection outputs = fileCollection(outputFile);
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            if (method.getName().equals("getBuildFile")) {
                return buildFile;
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
                    return ":apiContractReport";
                case "getProject":
                    return project;
                case "getName":
                    return "apiContractReport";
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
                    return filesOwner(method.getReturnType(), fileCollection());
                case "getOutputs":
                    return filesOwner(method.getReturnType(), outputs);
                case "getLocalState":
                case "getDestroyables":
                    return registeredFilesOwner(method.getReturnType());
                case "getActions":
                    return Collections.singletonList(new Object());
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
        boolean customActions,
        boolean supportedCustomActions
    ) {
        return basicFileTransformTask(path, name, inputs, outputs, inputProperties, customActions, supportedCustomActions, null);
    }

    private static Task basicFileTransformTask(
        String path,
        String name,
        FileCollection inputs,
        FileCollection outputs,
        Map<String, String> inputProperties,
        boolean customActions,
        boolean supportedCustomActions,
        File buildFile
    ) {
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            if (method.getName().equals("getBuildFile")) {
                return buildFile;
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
                    return copySpec(customActions, supportedCustomActions);
                case "getActions":
                    return customActions ? Collections.singletonList(new Object()) : Collections.emptyList();
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static String decodeMapping(String value) {
        return new String(Base64.getUrlDecoder().decode(value), StandardCharsets.UTF_8);
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
        return copySpec(customActions, true);
    }

    private static Object copySpec(boolean customActions, boolean supportedCustomActions) {
        return proxy(CopySpecContract.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "hasCustomActions":
                    return customActions;
                case "buildRootResolver":
                    Object copyAction = supportedCustomActions ? new MapBackedExpandAction() : new ArbitraryCopyAction();
                    return copySpecResolver(customActions ? Collections.singletonList(copyAction) : Collections.emptyList());
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

    private static Object copySpecResolver(Iterable<Object> actions) {
        return proxy(CopySpecResolverContract.class, (proxy, method, args) -> {
            if (method.getName().equals("getAllCopyActions")) {
                return actions;
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
            if (method.getName().equals("getAsPath")) {
                return fileSet.stream()
                    .map(File::getAbsolutePath)
                    .collect(Collectors.joining(File.pathSeparator));
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static ConfigurationContainer configurationContainer(String name, FileCollection files) {
        Configuration configuration = proxy(Configuration.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "isCanBeResolved":
                    return true;
                case "getFiles":
                    return files.getFiles();
                case "getAsPath":
                    return files.getAsPath();
                default:
                    return defaultValue(method.getReturnType());
            }
        });
        return proxy(ConfigurationContainer.class, (proxy, method, args) -> {
            if (method.getName().equals("findByName") && args != null && args.length == 1 && name.equals(args[0])) {
                return configuration;
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static VersionConstraint versionConstraint(
        String strict,
        String required,
        String preferred,
        String branch,
        java.util.List<String> rejected
    ) {
        return proxy(VersionConstraint.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getStrictVersion":
                    return strict;
                case "getRequiredVersion":
                    return required;
                case "getPreferredVersion":
                    return preferred;
                case "getBranch":
                    return branch;
                case "getRejectedVersions":
                    return rejected;
                default:
                    return defaultValue(method.getReturnType());
            }
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
        JavaCompilerProvider getJavaCompiler();
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
        Object getManifest();
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
        Object getFilter();
        Object getOptions();
    }

    public interface ExecContract {
        String getExecutable();
        Iterable<String> getArgs();
        Map<String, String> getEnvironment();
        File getWorkingDir();
        boolean isIgnoreExitValue();
    }

    public interface JavaExecContract {
        FileCollection getClasspath();
        ValueProvider getMainClass();
        Iterable<String> getArgs();
        Iterable<String> getJvmArgs();
        String getMaxHeapSize();
        Map<String, String> getSystemProperties();
        Map<String, String> getEnvironment();
        File getWorkingDir();
        boolean isIgnoreExitValue();
    }

    public interface CreateStartScriptsContract {
        String getApplicationName();
        ValueProvider getMainClass();
        ValueProvider getMainModule();
        FileCollection getClasspath();
        File getOutputDir();
        String getExecutableDir();
        Iterable<String> getDefaultJvmOpts();
        String getOptsEnvironmentVar();
        ValueProvider getGitRef();
        File getUnixScript();
        File getWindowsScript();
    }

    public interface JavadocContract {
        FileCollection getSource();
        FileCollection getClasspath();
        File getDestinationDir();
        String getTitle();
        String getMaxMemory();
        JavadocOptionsContract getOptions();
    }

    public interface CopySpecContract {
        boolean hasCustomActions();
        Object buildRootResolver();
        Object getDuplicatesStrategy();
        String getFilteringCharset();
        Set<String> getIncludes();
        Set<String> getExcludes();
        boolean isCaseSensitive();
        boolean isIncludeEmptyDirs();
        Object getFilePermissions();
        Object getDirPermissions();
    }

    public interface CopySpecResolverContract {
        Iterable<Object> getAllCopyActions();
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

        public boolean isDebug() {
            return true;
        }

        public FileCollection getAnnotationProcessorPath() {
            return fileCollection();
        }

        public Iterable<String> getCompilerArgs() {
            return Collections.singletonList("-parameters");
        }
    }

    public static class JavadocOptionsContract {
        public String getEncoding() {
            return "UTF-8";
        }

        public boolean isNoTimestamp() {
            return true;
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

    public static class ManifestSpec {
        public Map<String, String> getAttributes() {
            Map<String, String> attributes = new LinkedHashMap<>();
            attributes.put("Main-Class", "com.example.Main");
            attributes.put("Implementation-Title", "sample");
            return attributes;
        }
    }

    public static class MapBackedExpandAction {
    }

    public static class ArbitraryCopyAction {
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

    public static class JavaCompilerProvider {
        private final File javaHome;

        JavaCompilerProvider(File javaHome) {
            this.javaHome = javaHome;
        }

        public JavaCompiler getOrNull() {
            return new JavaCompiler(javaHome);
        }
    }

    public static class JavaCompiler {
        private final File javaHome;

        JavaCompiler(File javaHome) {
            this.javaHome = javaHome;
        }

        public JavaToolchainMetadata getMetadata() {
            return new JavaToolchainMetadata(javaHome);
        }

        public FileProvider getExecutablePath() {
            return new FileProvider(new File(javaHome, "bin/javac"));
        }
    }

    public static class JavaToolchainMetadata {
        private final File javaHome;

        JavaToolchainMetadata(File javaHome) {
            this.javaHome = javaHome;
        }

        public FileProvider getInstallationPath() {
            return new FileProvider(javaHome);
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

    public static class TestFilterSpec {
        private final Set<String> includePatterns;
        private final Set<String> excludePatterns;

        public TestFilterSpec() {
            this(Collections.singleton("example.*Test"), Collections.emptySet());
        }

        public TestFilterSpec(Set<String> includePatterns, Set<String> excludePatterns) {
            this.includePatterns = includePatterns;
            this.excludePatterns = excludePatterns;
        }

        public Set<String> getIncludePatterns() {
            return includePatterns;
        }

        public Set<String> getExcludePatterns() {
            return excludePatterns;
        }
    }

    public static class JUnitPlatformOptionsSpec {
        public Set<String> getIncludeTags() {
            Set<String> tags = new LinkedHashSet<>();
            tags.add("fast");
            tags.add("integration");
            return tags;
        }

        public Set<String> getExcludeTags() {
            return Collections.singleton("slow");
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

    public static class KotlinCompile {
    }

    public static class Jar {
    }

    public static class Zip {
    }

    public static class Tar {
    }

    public static class Javadoc {
    }

    public static class Test {
    }

    public static class Exec {
    }

    public static class JavaExec {
    }

    public static class CreateStartScripts {
    }

    public static class Copy {
    }

    public static class ProcessResources {
    }

    public static class DefaultTask {
    }
}
