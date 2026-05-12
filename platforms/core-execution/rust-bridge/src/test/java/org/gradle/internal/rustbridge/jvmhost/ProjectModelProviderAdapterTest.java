package org.gradle.internal.rustbridge.jvmhost;

import gradle.substrate.v1.BuildPlanTask;
import gradle.substrate.v1.BuildPlanTaskInputSpec;
import gradle.substrate.v1.BuildPlanTaskOutputSpec;

import org.gradle.StartParameter;
import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.artifacts.Configuration;
import org.gradle.api.artifacts.ConfigurationContainer;
import org.gradle.api.artifacts.ModuleVersionIdentifier;
import org.gradle.api.artifacts.ResolvableDependencies;
import org.gradle.api.artifacts.ResolvedArtifact;
import org.gradle.api.artifacts.ResolvedConfiguration;
import org.gradle.api.artifacts.ResolvedModuleVersion;
import org.gradle.api.artifacts.VersionConstraint;
import org.gradle.api.artifacts.component.ComponentSelector;
import org.gradle.api.artifacts.component.ModuleComponentIdentifier;
import org.gradle.api.file.FileCollection;
import org.gradle.api.invocation.Gradle;
import org.gradle.api.artifacts.result.ResolutionResult;
import org.gradle.api.artifacts.result.ResolvedComponentResult;
import org.gradle.api.artifacts.result.ResolvedDependencyResult;
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
    public void capturesExactMethodTestIncludeFilterAsNativeReady() throws IOException {
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
                new LinkedHashSet<>(Arrays.asList("com.example.AppTest.someMethod")),
                Collections.emptySet()
            )
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(test, Test.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("com.example.AppTest.someMethod", inputs.get("test_method_includes"));
        assertFalse(inputs.containsKey("test_filter"));
        assertFalse(inputs.containsKey("test_filter_includes"));
        assertEquals("false", inputs.get("test_unsupported_filters"));
    }

    @org.junit.Test
    public void marksWildcardMethodTestFilterAsUnsupported() throws IOException {
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
                new LinkedHashSet<>(Arrays.asList("com.example.AppTest.some*")),
                Collections.emptySet()
            )
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(test, Test.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("test_unsupported_filters"));
        assertEquals("com.example.AppTest.some*", inputs.get("test_filter_includes"));
        assertFalse(inputs.containsKey("test_method_includes"));
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
    public void marksCycloneDxTaskAsMissingSchemaBackedSbomContract() throws IOException {
        File inputJar = temporaryFolder.newFile("runtime.jar");
        File outputJson = temporaryFolder.newFile("bom.json");
        Task cyclonedx = cyclonedxDirectTask(inputJar, outputJson);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(cyclonedx, CyclonedxDirectTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":cyclonedxDirectBom", task.getPath());
        assertEquals("sbom", task.getActionKind());
        assertEquals("process", task.getWorkerIsolation());
        assertEquals("CyclonedxDirectTask", inputs.get("taskType"));
        assertEquals(":cyclonedxDirectBom", inputs.get("cyclonedx_identity_task_path"));
        assertEquals(":", inputs.get("cyclonedx_identity_project_path"));
        assertEquals(temporaryFolder.getRoot().getAbsolutePath(), inputs.get("cyclonedx_identity_project_dir"));
        assertEquals("org.example", inputs.get("cyclonedx_component_group"));
        assertEquals("demo", inputs.get("cyclonedx_component_name"));
        assertEquals("1.0", inputs.get("cyclonedx_component_version"));
        assertEquals("LIBRARY", inputs.get("cyclonedx_project_type"));
        assertEquals("VERSION_16", inputs.get("cyclonedx_schema_version"));
        assertEquals("true", inputs.get("cyclonedx_include_bom_serial_number"));
        assertEquals("cyclonedx-core-metadata-constructor-now", inputs.get("cyclonedx_timestamp_source_policy"));
        assertEquals("cyclonedx-gradle-random-uuid", inputs.get("cyclonedx_serial_source_policy"));
        assertEquals("false", inputs.get("cyclonedx_include_build_system"));
        assertEquals("false", inputs.get("cyclonedx_include_build_environment"));
        assertEquals("true", inputs.get("cyclonedx_include_license_text"));
        assertEquals("false", inputs.get("cyclonedx_include_metadata_resolution"));
        assertEquals("true", inputs.get("cyclonedx_organizational_entity_present"));
        String organizationalEntityJson = new String(Base64.getDecoder().decode(inputs.get("cyclonedx_organizational_entity_json_b64")), StandardCharsets.UTF_8);
        assertTrue(organizationalEntityJson.contains("\"name\":\"Acme Security\""));
        assertTrue(organizationalEntityJson.contains("\"url\":[\"https://security.example.invalid\"]"));
        assertTrue(organizationalEntityJson.contains("\"email\":\"security@example.invalid\""));
        assertEquals("SPDX", inputs.get("cyclonedx_license_choice"));
        String licenseChoiceJson = new String(Base64.getDecoder().decode(inputs.get("cyclonedx_license_choice_json_b64")), StandardCharsets.UTF_8);
        assertTrue(licenseChoiceJson.contains("\"id\":\"Apache-2.0\""));
        assertTrue(licenseChoiceJson.contains("\"url\":\"https://www.apache.org/licenses/LICENSE-2.0\""));
        assertTrue(licenseChoiceJson.contains("\"contentType\":\"text/plain\""));
        assertTrue(licenseChoiceJson.contains("\"content\":\"Apache License text\""));
        assertEquals("CI", inputs.get("cyclonedx_build_system_environment_variable"));
        assertTrue(inputs.get("cyclonedx_external_references").contains("https://example.invalid/sbom"));
        String externalReferencesJson = new String(Base64.getDecoder().decode(inputs.get("cyclonedx_external_references_json_b64")), StandardCharsets.UTF_8);
        assertTrue(externalReferencesJson.contains("\"type\":\"website\""));
        assertTrue(externalReferencesJson.contains("\"url\":\"https://example.invalid/sbom\""));
        assertTrue(externalReferencesJson.contains("\"comment\":\"SBOM docs\""));
        assertTrue(externalReferencesJson.contains("\"alg\":\"SHA-256\""));
        assertTrue(externalReferencesJson.contains("\"content\":\"abc123\""));
        assertEquals("runtimeClasspath", inputs.get("cyclonedx_include_configs"));
        assertEquals(".*[Tt]est.*", inputs.get("cyclonedx_skip_configs"));
        assertEquals(outputJson.getAbsolutePath(), inputs.get("cyclonedx_json_output"));
        assertEquals(inputJar.getAbsolutePath(), inputs.get("cyclonedx_resolved_dependencies"));
        assertEquals("missing", inputs.get("cyclonedx_sbom_contract_status"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("resolution-result-edges"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("component-metadata"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("artifact-hash-policy"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("timestamp-source-policy"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("serial-source-policy"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("license-text-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("build-system-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("build-environment-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("organizational-entity-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("license-choice-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("build-system-environment-variable"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("external-reference-shape"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("aggregate-merge-policy"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("aggregate-input-contracts"));
        assertEquals("true", inputs.get("requires_jvm_task_execution"));
        assertFalse(inputs.containsKey("sbom_contract_json_b64"));
    }

    @org.junit.Test
    public void marksCycloneDxAggregateTaskAsMissingSchemaBackedInputContracts() throws IOException {
        File inputJson = temporaryFolder.newFile("direct-bom.json");
        File outputJson = temporaryFolder.newFile("aggregate-bom.json");
        Task cyclonedx = cyclonedxAggregateTask(inputJson, outputJson);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(cyclonedx, CyclonedxAggregateTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals(":cyclonedxBom", task.getPath());
        assertEquals("sbom", task.getActionKind());
        assertEquals("process", task.getWorkerIsolation());
        assertEquals("CyclonedxAggregateTask", inputs.get("taskType"));
        assertEquals(inputJson.getAbsolutePath(), inputs.get("cyclonedx_input_sboms"));
        assertEquals(outputJson.getAbsolutePath(), inputs.get("cyclonedx_json_output"));
        assertEquals("cyclonedx-core-metadata-constructor-now", inputs.get("cyclonedx_timestamp_source_policy"));
        assertEquals("omitted", inputs.get("cyclonedx_serial_source_policy"));
        assertEquals("missing", inputs.get("cyclonedx_sbom_contract_status"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("aggregate-input-contracts"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("aggregate-merge-policy"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("component-metadata"));
        assertEquals("true", inputs.get("requires_jvm_task_execution"));
        assertFalse(inputs.containsKey("aggregate_input_contracts_json_b64"));
        assertFalse(inputs.containsKey("sbom_contract_json_b64"));
    }

    @org.junit.Test
    public void marksCycloneDxAggregateUnknownPoliciesAsMissingContractFields() throws IOException {
        File inputJson = temporaryFolder.newFile("direct-bom.json");
        File outputJson = temporaryFolder.newFile("aggregate-bom.json");
        Task cyclonedx = cyclonedxAggregateTask(inputJson, outputJson, null, null, null, null, null);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(cyclonedx, CyclonedxAggregateTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        String missing = inputs.get("cyclonedx_missing_contract_fields");
        assertTrue(missing.contains("aggregate-input-contracts"));
        assertTrue(missing.contains("aggregate-merge-policy"));
        assertTrue(missing.contains("serial-source-policy"));
        assertTrue(missing.contains("build-system-policy"));
        assertTrue(missing.contains("build-environment-policy"));
        assertTrue(missing.contains("license-text-rendering"));
        assertTrue(missing.contains("metadata-resolution-policy"));
        assertFalse(inputs.containsKey("aggregate_input_contracts_json_b64"));
        assertFalse(inputs.containsKey("sbom_contract_json_b64"));
    }

    @org.junit.Test
    public void capturesCycloneDxResolutionGraphEvidenceWhenConfigurationGraphIsAvailable() throws IOException {
        File inputJar = temporaryFolder.newFile("lib-1.1.jar");
        File outputJson = temporaryFolder.newFile("bom.json");
        Task cyclonedx = cyclonedxDirectTask(
            inputJar,
            outputJson,
            cyclonedxConfigurationContainer("runtimeClasspath", inputJar)
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(cyclonedx, CyclonedxDirectTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        String graphJson = new String(
            Base64.getDecoder().decode(inputs.get("cyclonedx_resolution_graph_json_b64")),
            StandardCharsets.UTF_8
        );
        assertTrue(graphJson.contains("\"schema\":\"gradle-substrate.cyclonedx-resolution-graph.v1\""));
        assertTrue(graphJson.contains("\"name\":\"runtimeClasspath\""));
        assertTrue(graphJson, graphJson.contains("\"id\":\"org.example:app:1.0\""));
        assertTrue(graphJson, graphJson.contains("\"id\":\"org.example:lib:1.1\""));
        assertTrue(graphJson, graphJson.contains("\"artifactPath\":\"" + inputJar.getAbsolutePath().replace("\\", "\\\\") + "\""));
        assertTrue(graphJson, graphJson.contains("\"artifactType\":\"jar\""));
        assertTrue(graphJson, graphJson.contains("\"artifactExtension\":\"jar\""));
        assertTrue(graphJson, graphJson.contains("\"artifactClassifier\":\"\""));
        assertTrue(graphJson, graphJson.contains("\"inScopeConfigurations\":[\"runtimeClasspath\"]"));
        assertTrue(graphJson, graphJson.contains("\"from\":\"org.example:app:1.0\""));
        assertTrue(graphJson, graphJson.contains("\"to\":\"org.example:lib:1.1\""));
        assertTrue(graphJson, graphJson.contains("\"requested\":\"org.example:lib:1.+\""));
        assertEquals("partial", inputs.get("cyclonedx_sbom_contract_status"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("resolution-result-edges"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("component-metadata"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("license-metadata"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("artifact-hash-policy"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("timestamp-source-policy"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("serial-source-policy"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("license-text-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("build-system-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("build-environment-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("organizational-entity-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("license-choice-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("build-system-environment-variable"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("external-reference-shape"));
        assertFalse(inputs.containsKey("sbom_contract_json_b64"));
    }

    @org.junit.Test
    public void marksCycloneDxUnknownSerialPolicyAsMissingContractField() throws IOException {
        File inputJar = temporaryFolder.newFile("runtime.jar");
        File outputJson = temporaryFolder.newFile("bom.json");
        Task cyclonedx = cyclonedxDirectTask(inputJar, outputJson, null, null);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(cyclonedx, CyclonedxDirectTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertFalse(inputs.containsKey("cyclonedx_include_bom_serial_number"));
        assertFalse(inputs.containsKey("cyclonedx_serial_source_policy"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("serial-source-policy"));
        assertEquals("true", inputs.get("requires_jvm_task_execution"));
        assertFalse(inputs.containsKey("sbom_contract_json_b64"));
    }

    @org.junit.Test
    public void marksCycloneDxUnknownLicenseTextPolicyAsMissingContractField() throws IOException {
        File inputJar = temporaryFolder.newFile("runtime.jar");
        File outputJson = temporaryFolder.newFile("bom.json");
        Task cyclonedx = cyclonedxDirectTask(inputJar, outputJson, null, Boolean.FALSE, null);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(cyclonedx, CyclonedxDirectTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("false", inputs.get("cyclonedx_include_bom_serial_number"));
        assertEquals("omitted", inputs.get("cyclonedx_serial_source_policy"));
        assertFalse(inputs.containsKey("cyclonedx_include_license_text"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("license-text-rendering"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("serial-source-policy"));
        assertEquals("true", inputs.get("requires_jvm_task_execution"));
        assertFalse(inputs.containsKey("sbom_contract_json_b64"));
    }

    @org.junit.Test
    public void marksRawCycloneDxExternalReferencesAsUnsupportedShapeOnce() throws IOException {
        File inputJar = temporaryFolder.newFile("runtime.jar");
        File outputJson = temporaryFolder.newFile("bom.json");
        Task cyclonedx = cyclonedxDirectTask(
            inputJar,
            outputJson,
            null,
            Boolean.FALSE,
            Boolean.FALSE,
            Collections.singletonList(new RawExternalReference())
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(cyclonedx, CyclonedxDirectTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("https://example.invalid/raw", inputs.get("cyclonedx_external_references"));
        assertFalse(inputs.containsKey("cyclonedx_external_references_json_b64"));
        String missing = inputs.get("cyclonedx_missing_contract_fields");
        assertTrue(missing.contains("external-reference-shape"));
        assertEquals(missing.indexOf("external-reference-shape"), missing.lastIndexOf("external-reference-shape"));
        assertFalse(missing.contains("serial-source-policy"));
        assertFalse(missing.contains("license-text-rendering"));
        assertEquals("true", inputs.get("requires_jvm_task_execution"));
        assertFalse(inputs.containsKey("sbom_contract_json_b64"));
    }

    @org.junit.Test
    public void marksCycloneDxUnknownMetadataResolutionPolicyAsMissingContractField() throws IOException {
        File inputJar = temporaryFolder.newFile("runtime.jar");
        File outputJson = temporaryFolder.newFile("bom.json");
        Task cyclonedx = cyclonedxDirectTask(
            inputJar,
            outputJson,
            cyclonedxConfigurationContainer("runtimeClasspath", inputJar),
            Boolean.FALSE,
            Boolean.FALSE,
            null,
            Collections.singletonList(new TestExternalReference())
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(cyclonedx, CyclonedxDirectTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertFalse(inputs.containsKey("cyclonedx_include_metadata_resolution"));
        assertEquals("partial", inputs.get("cyclonedx_sbom_contract_status"));
        assertTrue(inputs.get("cyclonedx_missing_contract_fields").contains("metadata-resolution-policy"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("serial-source-policy"));
        assertFalse(inputs.get("cyclonedx_missing_contract_fields").contains("license-text-rendering"));
        assertEquals("true", inputs.get("requires_jvm_task_execution"));
        assertFalse(inputs.containsKey("sbom_contract_json_b64"));
    }

    @org.junit.Test
    public void marksCycloneDxUnknownBuildSystemPoliciesAsMissingContractFields() throws IOException {
        File inputJar = temporaryFolder.newFile("runtime.jar");
        File outputJson = temporaryFolder.newFile("bom.json");
        Task cyclonedx = cyclonedxDirectTask(
            inputJar,
            outputJson,
            cyclonedxConfigurationContainer("runtimeClasspath", inputJar),
            Boolean.FALSE,
            null,
            null,
            Boolean.FALSE,
            Boolean.FALSE,
            Collections.singletonList(new TestExternalReference())
        );

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(cyclonedx, CyclonedxDirectTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertFalse(inputs.containsKey("cyclonedx_include_build_system"));
        assertFalse(inputs.containsKey("cyclonedx_include_build_environment"));
        String missing = inputs.get("cyclonedx_missing_contract_fields");
        assertTrue(missing.contains("build-system-policy"));
        assertTrue(missing.contains("build-environment-policy"));
        assertFalse(missing.contains("serial-source-policy"));
        assertFalse(missing.contains("license-text-rendering"));
        assertFalse(missing.contains("metadata-resolution-policy"));
        assertEquals("true", inputs.get("requires_jvm_task_execution"));
        assertFalse(inputs.containsKey("sbom_contract_json_b64"));
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
    public void capturesValBackedStaticWriteFileDefaultTaskContract() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "tasks.register(\"customJvmTask\") {\n" +
                "  val message = \"custom JVM task requires compatibility execution\\n\"\n" +
                "  doLast { output.writeText(message) }\n" +
                "}\n"
        ), StandardCharsets.UTF_8);
        File outputFile = new File(temporaryFolder.getRoot(), "build/custom/task.txt");
        Task report = staticWriteFileTask("customJvmTask", buildFile, outputFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(report, DefaultTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("jvm-task", task.getActionKind());
        assertEquals("1", inputs.get("action_count"));
        assertEquals("Y3VzdG9tIEpWTSB0YXNrIHJlcXVpcmVzIGNvbXBhdGliaWxpdHkgZXhlY3V0aW9uCg==", inputs.get("static_output_text_b64"));
    }

    @org.junit.Test
    public void dynamicWriteFileDefaultTaskDoesNotCaptureStaticContract() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "tasks.register(\"customJvmTask\") {\n" +
                "  val message = System.getenv(\"MESSAGE\") ?: \"fallback\"\n" +
                "  doLast { output.writeText(message) }\n" +
                "}\n"
        ), StandardCharsets.UTF_8);
        File outputFile = new File(temporaryFolder.getRoot(), "build/custom/task.txt");
        Task report = staticWriteFileTask("customJvmTask", buildFile, outputFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(report, DefaultTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("jvm-task", task.getActionKind());
        assertFalse(inputs.containsKey("static_output_text_b64"));
    }

    @org.junit.Test
    public void capturesExactDetachedConfigurationReportAsStaticWriteFile() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "val detachedDependency = dependencies.create(\"org.example:detached:1.0\")\n" +
                "tasks.register(\"apiContractReport\") {\n" +
                "  val detached = configurations.detachedConfiguration(detachedDependency)\n" +
                "  doLast {\n" +
                "    val files = detached.resolve().map { it.name }.sorted()\n" +
                "    file(\"build/detached/resolved.txt\").writeText(files.joinToString(separator = \"\\n\", postfix = \"\\n\"))\n" +
                "  }\n" +
                "}\n"
        ), StandardCharsets.UTF_8);
        File outputFile = new File(temporaryFolder.getRoot(), "build/detached/resolved.txt");
        Task report = staticWriteFileTask(buildFile, outputFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(report, DefaultTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("jvm-task", task.getActionKind());
        assertEquals("ZGV0YWNoZWQtMS4wLmphcgo=", inputs.get("static_output_text_b64"));
        assertFalse(inputs.containsKey("unsupported_dependency_semantics"));
    }

    @org.junit.Test
    public void dynamicDetachedConfigurationRemainsUnsupportedDependencySemantics() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "val detachedDependency = dependencies.create(\"org.example:detached:1.+\")\n" +
                "tasks.register(\"apiContractReport\") {\n" +
                "  val detached = configurations.detachedConfiguration(detachedDependency)\n" +
                "  doLast { file(\"build/detached/resolved.txt\").writeText(detached.resolve().joinToString()) }\n" +
                "}\n"
        ), StandardCharsets.UTF_8);
        File outputFile = new File(temporaryFolder.getRoot(), "build/detached/resolved.txt");
        Task report = staticWriteFileTask(buildFile, outputFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(report, DefaultTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("unsupported_dependency_semantics"));
        assertTrue(inputs.get("unsupported_repository_features").contains("detached-configuration:build-script"));
        assertFalse(inputs.containsKey("static_output_text_b64"));
    }

    @org.junit.Test
    public void capturesExactArtifactViewReportAsStaticWriteFile() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "dependencies { implementation(\"org.gradle.substrate:artifact-view:1.0\") }\n" +
                "tasks.register(\"resolveArtifactView\") {\n" +
                "  doLast {\n" +
                "    val view = configurations.runtimeClasspath.get().incoming.artifactView { lenient(true) }\n" +
                "    val files = view.files.files.map { it.name }.sorted()\n" +
                "    file(\"build/artifact-view/resolved.txt\").writeText(files.joinToString(separator = \"\\n\", postfix = \"\\n\"))\n" +
                "  }\n" +
                "}\n"
        ), StandardCharsets.UTF_8);
        File outputFile = new File(temporaryFolder.getRoot(), "build/artifact-view/resolved.txt");
        Task report = staticWriteFileTask("resolveArtifactView", buildFile, outputFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(report, DefaultTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("jvm-task", task.getActionKind());
        assertEquals("YXJ0aWZhY3Qtdmlldy0xLjAuamFyCg==", inputs.get("static_output_text_b64"));
        assertFalse(inputs.containsKey("unsupported_dependency_semantics"));
    }

    @org.junit.Test
    public void nontrivialArtifactViewRemainsUnsupportedDependencySemantics() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "dependencies { implementation(\"org.gradle.substrate:artifact-view:1.0\") }\n" +
                "tasks.register(\"resolveArtifactView\") {\n" +
                "  doLast {\n" +
                "    val view = configurations.runtimeClasspath.get().incoming.artifactView { componentFilter { false } }\n" +
                "    file(\"build/artifact-view/resolved.txt\").writeText(view.files.files.joinToString())\n" +
                "  }\n" +
                "}\n"
        ), StandardCharsets.UTF_8);
        File outputFile = new File(temporaryFolder.getRoot(), "build/artifact-view/resolved.txt");
        Task report = staticWriteFileTask("resolveArtifactView", buildFile, outputFile);

        BuildPlanTask task = ProjectModelProviderAdapter.toBuildPlanTask(report, DefaultTask.class);
        Map<String, String> inputs = task.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("unsupported_dependency_semantics"));
        assertTrue(inputs.get("unsupported_repository_features").contains("artifact-view:build-script"));
        assertFalse(inputs.containsKey("static_output_text_b64"));
    }

    @org.junit.Test
    public void exactReleaseComponentMetadataStatusRuleIsNativeReady() throws IOException {
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

        assertFalse(inputs.containsKey("unsupported_dependency_semantics"));
        assertFalse(inputs.getOrDefault("unsupported_repository_features", "").contains("component-metadata-rule"));
    }

    @org.junit.Test
    public void nontrivialComponentMetadataRuleRemainsUnsupportedDependencySemantics() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "dependencies { components { all { status = \"integration\" } } }"
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
    public void exactModuleDependencySubstitutionIsNativeReady() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "configurations.configureEach { resolutionStrategy.dependencySubstitution { substitute(module(\"org.example:original\")).using(module(\"org.example:replacement:1.0\")) } }"
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

        assertFalse(inputs.containsKey("unsupported_dependency_semantics"));
        assertFalse(inputs.getOrDefault("unsupported_repository_features", "").contains("dependency-substitution"));
    }

    @org.junit.Test
    public void projectDependencySubstitutionRemainsUnsupportedDependencySemantics() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "configurations.configureEach { resolutionStrategy.dependencySubstitution { substitute(module(\"org.example:original\")).using(project(\":replacement\")) } }"
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
        assertTrue(inputs.get("unsupported_repository_features").contains("dependency-substitution:build-script"));
    }

    @org.junit.Test
    public void capturesExactArtifactTransformReportAsStaticWriteFile() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "val artifactKind = Attribute.of(\"org.gradle.substrate.artifact-kind\", String::class.java)\n" +
                "abstract class MarkerTransform : TransformAction<org.gradle.api.artifacts.transform.TransformParameters.None> {\n" +
                "  override fun transform(outputs: TransformOutputs) {\n" +
                "    val input = inputArtifact.get().asFile\n" +
                "    val output = outputs.file(input.nameWithoutExtension + \".marker\")\n" +
                "    output.writeText(input.name + \"\\n\")\n" +
                "  }\n" +
                "}\n" +
                "dependencies {\n" +
                "  registerTransform(MarkerTransform::class) {\n" +
                "    from.attribute(artifactKind, \"jar\")\n" +
                "    to.attribute(artifactKind, \"marker\")\n" +
                "  }\n" +
                "  implementation(\"org.gradle.substrate:artifact-transform:1.0\")\n" +
                "}\n" +
                "configurations.runtimeClasspath { attributes.attribute(artifactKind, \"marker\") }\n" +
                "tasks.register(\"resolveTransformedArtifact\") {\n" +
                "  doLast {\n" +
                "    val files = configurations.runtimeClasspath.get().files.map { it.name }.sorted()\n" +
                "    file(\"build/artifact-transform/resolved.txt\").writeText(files.joinToString(separator = \"\\n\", postfix = \"\\n\"))\n" +
                "  }\n" +
                "}\n"
        ), StandardCharsets.UTF_8);
        File outputFile = new File(temporaryFolder.getRoot(), "build/artifact-transform/resolved.txt");
        Task task = staticWriteFileTask("resolveTransformedArtifact", buildFile, outputFile);

        BuildPlanTask planTask = ProjectModelProviderAdapter.toBuildPlanTask(task, DefaultTask.class);
        Map<String, String> inputs = planTask.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("YXJ0aWZhY3QtdHJhbnNmb3JtLTEuMC5tYXJrZXIK", inputs.get("static_output_text_b64"));
        assertFalse(inputs.containsKey("unsupported_dependency_semantics"));
    }

    @org.junit.Test
    public void nontrivialArtifactTransformsRemainUnsupportedDependencySemantics() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "dependencies { registerTransform(OtherTransform::class) { from.attribute(kind, \"jar\"); to.attribute(kind, \"marker\") } }"
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

    @org.junit.Test
    public void exactStaticEnforcedPlatformIsNativeReady() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "dependencies { implementation(enforcedPlatform(\"org.example:platform:1.0\")) }"
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

        assertFalse(inputs.containsKey("unsupported_dependency_semantics"));
        assertFalse(inputs.getOrDefault("unsupported_repository_features", "").contains("enforced-platform"));
    }

    @org.junit.Test
    public void dynamicEnforcedPlatformRemainsUnsupportedDependencySemantics() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "dependencies { implementation(enforcedPlatform(\"org.example:platform:1.+\")) }"
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
        assertTrue(inputs.get("unsupported_repository_features").contains("enforced-platform:build-script"));
    }

    @org.junit.Test
    public void marksExplicitSettingsIncludeBuildAsUnsupportedDependencySemantics() throws IOException {
        File rootDir = temporaryFolder.newFolder("explicit-composite");
        File buildFile = new File(rootDir, "build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList("plugins { java }"), StandardCharsets.UTF_8);
        Files.write(new File(rootDir, "settings.gradle.kts").toPath(), Collections.singletonList("includeBuild(\"included\")"), StandardCharsets.UTF_8);
        Task task = basicFileTransformTask(
            ":classes",
            "classes",
            fileCollection(),
            fileCollection(),
            Collections.emptyMap(),
            false,
            false,
            buildFile,
            null,
            rootDir
        );

        BuildPlanTask planTask = ProjectModelProviderAdapter.toBuildPlanTask(task, DefaultTask.class);
        Map<String, String> inputs = planTask.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("unsupported_dependency_semantics"));
        assertTrue(inputs.get("unsupported_repository_features").contains("composite-substitution:settings"));
    }

    @org.junit.Test
    public void doesNotMarkIncludedBuildApiWithoutExplicitSettingsIncludeBuild() throws IOException {
        File rootDir = temporaryFolder.newFolder("configuration-only-build-logic");
        File buildFile = new File(rootDir, "build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList("plugins { java }"), StandardCharsets.UTF_8);
        Files.write(new File(rootDir, "settings.gradle.kts").toPath(), Collections.singletonList("rootProject.name = \"configuration-only\""), StandardCharsets.UTF_8);
        Task task = basicFileTransformTask(
            ":classes",
            "classes",
            fileCollection(),
            fileCollection(),
            Collections.emptyMap(),
            false,
            false,
            buildFile,
            gradleWithSyntheticIncludedBuild(),
            rootDir
        );

        BuildPlanTask planTask = ProjectModelProviderAdapter.toBuildPlanTask(task, DefaultTask.class);
        Map<String, String> inputs = planTask.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertFalse(inputs.containsKey("unsupported_dependency_semantics"));
        assertFalse(inputs.getOrDefault("unsupported_repository_features", "").contains("composite-substitution:settings"));
    }

    @org.junit.Test
    public void mavenLocalRepositoryIsNotBlanketUnsupportedDependencySemantics() throws IOException {
        File buildFile = temporaryFolder.newFile("build.gradle.kts");
        Files.write(buildFile.toPath(), Collections.singletonList(
            "repositories { mavenLocal() }"
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

        assertFalse(inputs.containsKey("unsupported_dependency_semantics"));
        assertFalse(inputs.getOrDefault("unsupported_repository_features", "").contains("maven-local:build-script"));
    }

    @org.junit.Test
    public void marksOfflineStartParameterAsUnsupportedDependencySemantics() {
        Task task = basicFileTransformTask(
            ":classes",
            "classes",
            fileCollection(),
            fileCollection(),
            Collections.emptyMap(),
            false,
            false,
            null,
            startParameter(true, false)
        );

        BuildPlanTask planTask = ProjectModelProviderAdapter.toBuildPlanTask(task, DefaultTask.class);
        Map<String, String> inputs = planTask.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("unsupported_dependency_semantics"));
        assertTrue(inputs.get("unsupported_repository_features").contains("dependency-offline-mode:start-parameter"));
    }

    @org.junit.Test
    public void marksRefreshDependenciesStartParameterAsUnsupportedDependencySemantics() {
        Task task = basicFileTransformTask(
            ":classes",
            "classes",
            fileCollection(),
            fileCollection(),
            Collections.emptyMap(),
            false,
            false,
            null,
            startParameter(false, true)
        );

        BuildPlanTask planTask = ProjectModelProviderAdapter.toBuildPlanTask(task, DefaultTask.class);
        Map<String, String> inputs = planTask.getInputSpecsList().stream()
            .filter(input -> input.getKind().equals("value"))
            .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

        assertEquals("true", inputs.get("unsupported_dependency_semantics"));
        assertTrue(inputs.get("unsupported_repository_features").contains("dependency-refresh:start-parameter"));
    }

    @org.junit.Test
    public void marksProxySystemPropertiesAsUnsupportedDependencySemantics() {
        String previousHttpProxy = System.getProperty("http.proxyHost");
        String previousHttpsProxy = System.getProperty("https.proxyHost");
        String previousSocksProxy = System.getProperty("socksProxyHost");
        try {
            System.setProperty("http.proxyHost", "proxy.local");
            System.setProperty("https.proxyHost", "secure-proxy.local");
            System.setProperty("socksProxyHost", "socks-proxy.local");
            Task task = basicFileTransformTask(
                ":classes",
                "classes",
                fileCollection(),
                fileCollection(),
                Collections.emptyMap(),
                false,
                false
            );

            BuildPlanTask planTask = ProjectModelProviderAdapter.toBuildPlanTask(task, DefaultTask.class);
            Map<String, String> inputs = planTask.getInputSpecsList().stream()
                .filter(input -> input.getKind().equals("value"))
                .collect(Collectors.toMap(BuildPlanTaskInputSpec::getName, BuildPlanTaskInputSpec::getValue));

            assertEquals("true", inputs.get("unsupported_dependency_semantics"));
            assertTrue(inputs.get("unsupported_repository_features").contains("dependency-proxy:http"));
            assertTrue(inputs.get("unsupported_repository_features").contains("dependency-proxy:https"));
            assertTrue(inputs.get("unsupported_repository_features").contains("dependency-proxy:socks"));
        } finally {
            restoreProperty("http.proxyHost", previousHttpProxy);
            restoreProperty("https.proxyHost", previousHttpsProxy);
            restoreProperty("socksProxyHost", previousSocksProxy);
        }
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

    private static Task cyclonedxDirectTask(File runtimeJar, File outputJson) {
        return cyclonedxDirectTask(runtimeJar, outputJson, null);
    }

    private static Task cyclonedxDirectTask(
        File runtimeJar,
        File outputJson,
        ConfigurationContainer configurations
    ) {
        return cyclonedxDirectTask(runtimeJar, outputJson, configurations, Boolean.TRUE);
    }

    private static Task cyclonedxDirectTask(
        File runtimeJar,
        File outputJson,
        ConfigurationContainer configurations,
        Boolean includeBomSerialNumber
    ) {
        return cyclonedxDirectTask(runtimeJar, outputJson, configurations, includeBomSerialNumber, Boolean.TRUE);
    }

    private static Task cyclonedxDirectTask(
        File runtimeJar,
        File outputJson,
        ConfigurationContainer configurations,
        Boolean includeBomSerialNumber,
        Boolean includeLicenseText
    ) {
        return cyclonedxDirectTask(runtimeJar, outputJson, configurations, includeBomSerialNumber, includeLicenseText, Collections.singletonList(new TestExternalReference()));
    }

    private static Task cyclonedxDirectTask(
        File runtimeJar,
        File outputJson,
        ConfigurationContainer configurations,
        Boolean includeBomSerialNumber,
        Boolean includeLicenseText,
        Iterable<?> externalReferences
    ) {
        return cyclonedxDirectTask(runtimeJar, outputJson, configurations, includeBomSerialNumber, includeLicenseText, Boolean.FALSE, externalReferences);
    }

    private static Task cyclonedxDirectTask(
        File runtimeJar,
        File outputJson,
        ConfigurationContainer configurations,
        Boolean includeBomSerialNumber,
        Boolean includeLicenseText,
        Boolean includeMetadataResolution,
        Iterable<?> externalReferences
    ) {
        return cyclonedxDirectTask(runtimeJar, outputJson, configurations, includeBomSerialNumber, Boolean.FALSE, Boolean.FALSE, includeLicenseText, includeMetadataResolution, externalReferences);
    }

    private static Task cyclonedxDirectTask(
        File runtimeJar,
        File outputJson,
        ConfigurationContainer configurations,
        Boolean includeBomSerialNumber,
        Boolean includeBuildSystem,
        Boolean includeBuildEnvironment,
        Boolean includeLicenseText,
        Boolean includeMetadataResolution,
        Iterable<?> externalReferences
    ) {
        FileCollection inputs = fileCollection(runtimeJar);
        FileCollection outputs = fileCollection(outputJson);
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            if (method.getName().equals("getProjectDir")) {
                return runtimeJar.getParentFile();
            }
            if (method.getName().equals("getConfigurations") && configurations != null) {
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
        return proxy(new Class<?>[] {Task.class, CyclonedxDirectTask.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return ":cyclonedxDirectBom";
                case "getProject":
                    return project;
                case "getName":
                    return "cyclonedxDirectBom";
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
                case "getComponentGroup":
                    return new ObjectProvider("org.example");
                case "getComponentName":
                    return new ObjectProvider("demo");
                case "getComponentVersion":
                    return new ObjectProvider("1.0");
                case "getProjectType":
                    return new ObjectProvider("LIBRARY");
                case "getSchemaVersion":
                    return new ObjectProvider("VERSION_16");
                case "getIncludeBomSerialNumber":
                    return includeBomSerialNumber == null ? null : new ObjectProvider(includeBomSerialNumber);
                case "getIncludeBuildSystem":
                    return includeBuildSystem == null ? null : new ObjectProvider(includeBuildSystem);
                case "getIncludeBuildEnvironment":
                    return includeBuildEnvironment == null ? null : new ObjectProvider(includeBuildEnvironment);
                case "getIncludeMetadataResolution":
                    return includeMetadataResolution == null ? null : new ObjectProvider(includeMetadataResolution);
                case "getIncludeLicenseText":
                    return includeLicenseText == null ? null : new ObjectProvider(includeLicenseText);
                case "getOrganizationalEntity":
                    return new ObjectProvider(new TestOrganizationalEntity());
                case "getLicenseChoice":
                    return new ObjectProvider(new TestLicenseChoice());
                case "getBuildSystemEnvironmentVariable":
                    return new ObjectProvider("CI");
                case "getExternalReferences":
                    return new ObjectProvider(externalReferences);
                case "getIncludeConfigs":
                    return new ObjectProvider(Collections.singletonList("runtimeClasspath"));
                case "getSkipConfigs":
                    return new ObjectProvider(Collections.singletonList(".*[Tt]est.*"));
                case "getJsonOutput":
                    return new FileProvider(outputJson);
                case "getXmlOutput":
                    return null;
                case "getResolvedDependencies":
                    return inputs;
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Task cyclonedxAggregateTask(File inputJson, File outputJson) {
        return cyclonedxAggregateTask(inputJson, outputJson, Boolean.FALSE, Boolean.FALSE, Boolean.FALSE, Boolean.FALSE, Boolean.FALSE);
    }

    private static Task cyclonedxAggregateTask(
        File inputJson,
        File outputJson,
        Boolean includeBomSerialNumber,
        Boolean includeBuildSystem,
        Boolean includeBuildEnvironment,
        Boolean includeLicenseText,
        Boolean includeMetadataResolution
    ) {
        FileCollection inputs = fileCollection(inputJson);
        FileCollection outputs = fileCollection(outputJson);
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            if (method.getName().equals("getProjectDir")) {
                return outputJson.getParentFile();
            }
            return defaultValue(method.getReturnType());
        });
        TaskDependency noDependencies = proxy(TaskDependency.class, (proxy, method, args) -> {
            if (method.getName().equals("getDependencies")) {
                return Collections.emptySet();
            }
            return defaultValue(method.getReturnType());
        });
        return proxy(new Class<?>[] {Task.class, CyclonedxAggregateTask.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getPath":
                    return ":cyclonedxBom";
                case "getProject":
                    return project;
                case "getName":
                    return "cyclonedxBom";
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
                case "getComponentGroup":
                    return new ObjectProvider("org.example");
                case "getComponentName":
                    return new ObjectProvider("aggregate");
                case "getComponentVersion":
                    return new ObjectProvider("1.0");
                case "getProjectType":
                    return new ObjectProvider("APPLICATION");
                case "getSchemaVersion":
                    return new ObjectProvider("VERSION_16");
                case "getIncludeBomSerialNumber":
                    return includeBomSerialNumber == null ? null : new ObjectProvider(includeBomSerialNumber);
                case "getIncludeBuildSystem":
                    return includeBuildSystem == null ? null : new ObjectProvider(includeBuildSystem);
                case "getIncludeBuildEnvironment":
                    return includeBuildEnvironment == null ? null : new ObjectProvider(includeBuildEnvironment);
                case "getIncludeLicenseText":
                    return includeLicenseText == null ? null : new ObjectProvider(includeLicenseText);
                case "getIncludeMetadataResolution":
                    return includeMetadataResolution == null ? null : new ObjectProvider(includeMetadataResolution);
                case "getIncludeConfigs":
                case "getSkipConfigs":
                case "getExternalReferences":
                    return new ObjectProvider(Collections.emptyList());
                case "getJsonOutput":
                    return new FileProvider(outputJson);
                case "getXmlOutput":
                    return null;
                case "getInputSboms":
                    return inputs;
                case "compareTo":
                    return 0;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static ConfigurationContainer cyclonedxConfigurationContainer(String name, File artifactFile) {
        ResolvedComponentResult child = resolvedComponent("org.example", "lib", "1.1");
        ResolvedDependencyResult edge = proxy(ResolvedDependencyResult.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getSelected":
                    return child;
                case "getRequested":
                    return componentSelector("org.example:lib:1.+");
                default:
                    return defaultValue(method.getReturnType());
            }
        });
        ResolvedComponentResult root = resolvedComponent("org.example", "app", "1.0", Collections.singleton(edge));
        Set<ResolvedComponentResult> components = new LinkedHashSet<>(Arrays.asList(root, child));
        ResolutionResult result = proxy(ResolutionResult.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getRoot":
                    return root;
                case "getAllComponents":
                    return components;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
        ResolvableDependencies incoming = proxy(ResolvableDependencies.class, (proxy, method, args) -> {
            if (method.getName().equals("getResolutionResult")) {
                return result;
            }
            return defaultValue(method.getReturnType());
        });
        Configuration configuration = proxy(Configuration.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "isCanBeResolved":
                    return true;
                case "getName":
                    return name;
                case "getIncoming":
                    return incoming;
                case "getResolvedConfiguration":
                    return resolvedConfiguration(artifactFile);
                case "getFiles":
                    return Collections.singleton(artifactFile);
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

    private static Object resolvedConfiguration(File artifactFile) {
        Object artifact = proxy(ResolvedArtifact.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getModuleVersion":
                    return resolvedModuleVersion("org.example", "lib", "1.1");
                case "getFile":
                    return artifactFile;
                case "getType":
                case "getExtension":
                    return "jar";
                case "getClassifier":
                    return "";
                default:
                    return defaultValue(method.getReturnType());
            }
        });
        return proxy(ResolvedConfiguration.class, (proxy, method, args) -> {
            if (method.getName().equals("getResolvedArtifacts")) {
                return Collections.singleton(artifact);
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static Object moduleVersionIdentifier(String group, String name, String version) {
        return proxy(ModuleVersionIdentifier.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getGroup":
                    return group;
                case "getName":
                    return name;
                case "getVersion":
                    return version;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Object resolvedModuleVersion(String group, String name, String version) {
        return proxy(ResolvedModuleVersion.class, (proxy, method, args) -> {
            if (method.getName().equals("getId")) {
                return moduleVersionIdentifier(group, name, version);
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static ResolvedComponentResult resolvedComponent(String group, String module, String version) {
        return resolvedComponent(group, module, version, Collections.emptySet());
    }

    private static ResolvedComponentResult resolvedComponent(
        String group,
        String module,
        String version,
        Set<ResolvedDependencyResult> dependencies
    ) {
        ModuleComponentIdentifier id = moduleComponentIdentifier(group, module, version);
        return proxy(ResolvedComponentResult.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getId":
                    return id;
                case "getDependencies":
                    return dependencies;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static ModuleComponentIdentifier moduleComponentIdentifier(String group, String module, String version) {
        return proxy(ModuleComponentIdentifier.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getGroup":
                    return group;
                case "getModule":
                    return module;
                case "getVersion":
                    return version;
                case "getDisplayName":
                case "toString":
                    return group + ":" + module + ":" + version;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static ComponentSelector componentSelector(String displayName) {
        return proxy(ComponentSelector.class, (proxy, method, args) -> {
            if (method.getName().equals("getDisplayName") || method.getName().equals("toString")) {
                return displayName;
            }
            return defaultValue(method.getReturnType());
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
        return staticWriteFileTask("apiContractReport", buildFile, outputFile);
    }

    private static Task staticWriteFileTask(String taskName, File buildFile, File outputFile) {
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
                    return ":" + taskName;
                case "getProject":
                    return project;
                case "getName":
                    return taskName;
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
        return basicFileTransformTask(path, name, inputs, outputs, inputProperties, customActions, supportedCustomActions, buildFile, null);
    }

    private static Task basicFileTransformTask(
        String path,
        String name,
        FileCollection inputs,
        FileCollection outputs,
        Map<String, String> inputProperties,
        boolean customActions,
        boolean supportedCustomActions,
        File buildFile,
        StartParameter startParameter
    ) {
        return basicFileTransformTask(path, name, inputs, outputs, inputProperties, customActions, supportedCustomActions, buildFile, startParameter == null ? null : gradle(startParameter), null);
    }

    private static Task basicFileTransformTask(
        String path,
        String name,
        FileCollection inputs,
        FileCollection outputs,
        Map<String, String> inputProperties,
        boolean customActions,
        boolean supportedCustomActions,
        File buildFile,
        Gradle gradle,
        File rootDir
    ) {
        Project project = proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getPath")) {
                return ":";
            }
            if (method.getName().equals("getBuildFile")) {
                return buildFile;
            }
            if (method.getName().equals("getGradle") && gradle != null) {
                return gradle;
            }
            if (method.getName().equals("getRootProject")) {
                return proxy;
            }
            if (method.getName().equals("getProjectDir") && rootDir != null) {
                return rootDir;
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

    private static Gradle gradleWithSyntheticIncludedBuild() {
        return proxy(Gradle.class, (proxy, method, args) -> {
            if (method.getName().equals("getIncludedBuilds")) {
                return Collections.singleton(new Object());
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static Gradle gradle(StartParameter startParameter) {
        return proxy(Gradle.class, (proxy, method, args) -> {
            if (method.getName().equals("getStartParameter")) {
                return startParameter;
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static StartParameter startParameter(boolean offline, boolean refreshDependencies) {
        StartParameter startParameter = new StartParameter();
        startParameter.setOffline(offline);
        startParameter.setRefreshDependencies(refreshDependencies);
        return startParameter;
    }

    private static void restoreProperty(String name, String previousValue) {
        if (previousValue == null) {
            System.clearProperty(name);
        } else {
            System.setProperty(name, previousValue);
        }
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

    public static class ObjectProvider {
        private final Object value;

        ObjectProvider(Object value) {
            this.value = value;
        }

        public Object getOrNull() {
            return value;
        }
    }

    public static class TestOrganizationalEntity {
        public String getName() {
            return "Acme Security";
        }

        public Iterable<String> getUrls() {
            return Collections.singletonList("https://security.example.invalid");
        }

        public Iterable<TestOrganizationalContact> getContacts() {
            return Collections.singletonList(new TestOrganizationalContact());
        }
    }

    public static class TestOrganizationalContact {
        public String getName() {
            return "Security Team";
        }

        public String getEmail() {
            return "security@example.invalid";
        }

        public String getPhone() {
            return "";
        }
    }

    public static class TestExternalReference {
        @Override
        public String toString() {
            return getUrl();
        }

        public String getUrl() {
            return "https://example.invalid/sbom";
        }

        public String getType() {
            return "WEBSITE";
        }

        public String getComment() {
            return "SBOM docs";
        }

        public Iterable<TestHash> getHashes() {
            return Collections.singletonList(new TestHash());
        }
    }

    public static class RawExternalReference {
        @Override
        public String toString() {
            return "https://example.invalid/raw";
        }
    }

    public static class TestHash {
        public String getAlgorithm() {
            return "SHA-256";
        }

        public String getValue() {
            return "abc123";
        }
    }

    public static class TestLicenseChoice {
        @Override
        public String toString() {
            return "SPDX";
        }

        public Object getExpression() {
            return null;
        }

        public Iterable<TestLicense> getLicenses() {
            return Collections.singletonList(new TestLicense());
        }
    }

    public static class TestLicense {
        public String getId() {
            return "Apache-2.0";
        }

        public String getName() {
            return "";
        }

        public String getUrl() {
            return "https://www.apache.org/licenses/LICENSE-2.0";
        }

        public TestLicenseText getText() {
            return new TestLicenseText();
        }
    }

    public static class TestLicenseText {
        public String getContentType() {
            return "text/plain";
        }

        public String getEncoding() {
            return "";
        }

        public String getContent() {
            return "Apache License text";
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

    public interface CyclonedxDirectTask {
        Object getComponentGroup();
        Object getComponentName();
        Object getComponentVersion();
        Object getProjectType();
        Object getSchemaVersion();
        Object getIncludeBomSerialNumber();
        Object getIncludeBuildSystem();
        Object getIncludeBuildEnvironment();
        Object getIncludeLicenseText();
        Object getIncludeMetadataResolution();
        Object getOrganizationalEntity();
        Object getLicenseChoice();
        Object getBuildSystemEnvironmentVariable();
        Object getExternalReferences();
        Object getIncludeConfigs();
        Object getSkipConfigs();
        Object getJsonOutput();
        Object getXmlOutput();
        FileCollection getResolvedDependencies();
    }

    public interface CyclonedxAggregateTask {
        Object getComponentGroup();
        Object getComponentName();
        Object getComponentVersion();
        Object getProjectType();
        Object getSchemaVersion();
        Object getIncludeBomSerialNumber();
        Object getIncludeBuildSystem();
        Object getIncludeBuildEnvironment();
        Object getIncludeLicenseText();
        Object getIncludeMetadataResolution();
        Object getOrganizationalEntity();
        Object getLicenseChoice();
        Object getBuildSystemEnvironmentVariable();
        Object getExternalReferences();
        Object getIncludeConfigs();
        Object getSkipConfigs();
        Object getJsonOutput();
        Object getXmlOutput();
        FileCollection getInputSboms();
    }

    public static class Copy {
    }

    public static class ProcessResources {
    }

    public static class DefaultTask {
    }
}
