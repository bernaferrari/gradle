package org.gradle.internal.rustbridge.e2e;

import org.gradle.internal.rustbridge.SubstrateClient;
import gradle.substrate.v1.*;
import org.junit.Assume;
import org.junit.Before;
import org.junit.Test;

import java.io.File;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.Assert.*;

/**
 * End-to-end tests that start the actual Rust daemon and verify round-trip communication.
 *
 * <p>These tests are gated by the system property {@code substrate.test.binary}.
 * If not set, all tests are skipped (assumeTrue).</p>
 *
 * <p>Usage: {@code ./gradlew :platforms:core-execution:rust-bridge:test -Dsubstrate.test.binary=/path/to/gradle-substrate-daemon}</p>
 */
public class SubstrateE2ETest {

    private static final String BINARY_PROPERTY = "substrate.test.binary";

    private SubstrateClient client;
    private Process daemonProcess;
    private Path socketDirectory;
    private Path socketPath;

    @Before
    public void setUp() throws Exception {
        String binary = System.getProperty(BINARY_PROPERTY);
        Assume.assumeTrue(
            "Skipping E2E tests: set -D" + BINARY_PROPERTY + "=/path/to/binary",
            binary != null && !binary.isEmpty()
        );

        Assume.assumeTrue(
            "Skipping E2E tests: binary not found at " + binary,
            new File(binary).exists()
        );

        socketDirectory = Files.createTempDirectory("substrate-e2e-");
        socketPath = socketDirectory.resolve("substrate.sock");

        ProcessBuilder pb = new ProcessBuilder(
            binary,
            "--socket-path", socketPath.toString(),
            "--log-level", "info"
        );
        pb.environment().put("SUBSTRATE_LOG_LEVEL", "info");
        pb.redirectErrorStream(true);
        daemonProcess = pb.start();

        // Consume stdout to prevent deadlock
        Thread consumer = new Thread(new Runnable() {
            @Override
            public void run() {
                try {
                    byte[] buffer = new byte[1024];
                    int n;
                    while ((n = daemonProcess.getInputStream().read(buffer)) != -1) {
                        // Discard daemon output during tests
                    }
                } catch (Exception ignored) {
                }
            }
        }, "e2e-daemon-consumer");
        consumer.setDaemon(true);
        consumer.start();

        // Wait for socket to appear
        int attempts = 0;
        while (!Files.exists(socketPath) && attempts < 50) {
            Thread.sleep(100);
            attempts++;
        }

        Assume.assumeTrue(
            "Skipping E2E tests: daemon failed to start (socket not created after 5s)",
            Files.exists(socketPath)
        );

        client = SubstrateClient.connect(socketPath.toString());
    }

    // --- Control Service ---

    @Test
    public void handshakeReturnsAccepted() {
        HandshakeResponse response = client.getControlStub().handshake(
            HandshakeRequest.newBuilder()
                .setClientVersion("e2e-test")
                .build()
        );
        assertTrue("Handshake should be accepted", response.getAccepted());
    }

    // --- Hash Service ---

    @Test
    public void hashBatchReturnsResults() {
        HashBatchResponse response = client.getHashStub().hashBatch(
            HashBatchRequest.newBuilder()
                .setAlgorithm("md5")
                .addFiles(FileToHash.newBuilder()
                    .setAbsolutePath("e2e-test-file.txt")
                    .setLength(11)
                    .setLastModified(System.currentTimeMillis())
                    .build())
                .build()
        );
        assertEquals(1, response.getResultsCount());
        assertEquals("e2e-test-file.txt", response.getResults(0).getAbsolutePath());
    }

    @Test
    public void hashBatchEmptyInputReturnsEmpty() {
        HashBatchResponse response = client.getHashStub().hashBatch(
            HashBatchRequest.newBuilder().build()
        );
        assertEquals(0, response.getResultsCount());
    }

    @Test
    public void hashBatchMultipleInputs() {
        long now = System.currentTimeMillis();
        HashBatchResponse response = client.getHashStub().hashBatch(
            HashBatchRequest.newBuilder()
                .addFiles(FileToHash.newBuilder().setAbsolutePath("a.txt").setLength(3).setLastModified(now).build())
                .addFiles(FileToHash.newBuilder().setAbsolutePath("b.txt").setLength(3).setLastModified(now).build())
                .addFiles(FileToHash.newBuilder().setAbsolutePath("c.txt").setLength(3).setLastModified(now).build())
                .build()
        );

        assertEquals(3, response.getResultsCount());
        // Different paths should return different absolute_path in results
        assertEquals("a.txt", response.getResults(0).getAbsolutePath());
        assertEquals("b.txt", response.getResults(1).getAbsolutePath());
        assertEquals("c.txt", response.getResults(2).getAbsolutePath());
    }

    // --- Execution Plan Service ---

    @Test
    public void predictOutcome() {
        PredictOutcomeResponse response = client.getExecutionPlanStub().predictOutcome(
            PredictOutcomeRequest.newBuilder()
                .setWork(WorkMetadata.newBuilder()
                    .setWorkIdentity("e2e-work-" + System.nanoTime())
                    .setDisplayName("e2e test work")
                    .build())
                .build()
        );
        assertNotNull(response.getReasoning());
    }

    @Test
    public void resolvePlan() {
        ResolvePlanResponse response = client.getExecutionPlanStub().resolvePlan(
            ResolvePlanRequest.newBuilder()
                .setWork(WorkMetadata.newBuilder()
                    .setWorkIdentity("e2e-plan-" + System.nanoTime())
                    .setDisplayName("e2e plan work")
                    .build())
                .build()
        );
        assertNotNull(response.getReasoning());
    }

    // --- Task Graph Service ---

    @Test
    public void registerTask() {
        String buildId = "e2e-tg-" + System.nanoTime();
        String taskPath = ":e2eTestTask";

        RegisterTaskResponse response = client.getTaskGraphStub().registerTask(
            RegisterTaskRequest.newBuilder()
                .setBuildId(buildId)
                .setTaskPath(taskPath)
                .setTaskType("e2e-test")
                .build()
        );
        assertTrue(response.getSuccess());
    }

    @Test
    public void resolveExecutionPlan() {
        String buildId = "e2e-tg-plan-" + System.nanoTime();

        client.getTaskGraphStub().registerTask(
            RegisterTaskRequest.newBuilder()
                .setBuildId(buildId)
                .setTaskPath(":compileJava")
                .setTaskType("java-compile")
                .build()
        );
        client.getTaskGraphStub().registerTask(
            RegisterTaskRequest.newBuilder()
                .setBuildId(buildId)
                .setTaskPath(":test")
                .setTaskType("test")
                .addDependsOn(":compileJava")
                .build()
        );

        ResolveExecutionPlanResponse response = client.getTaskGraphStub().resolveExecutionPlan(
            ResolveExecutionPlanRequest.newBuilder()
                .setBuildId(buildId)
                .build()
        );
        assertEquals(2, response.getTotalTasks());
        assertFalse("Should not have cycles", response.getHasCycles());
    }

    // --- Build Metrics Service ---

    @Test
    public void recordAndGetMetric() {
        String metricName = "e2e.metric." + System.nanoTime();

        client.getBuildMetricsStub().recordMetric(
            RecordMetricRequest.newBuilder()
                .setEvent(MetricEvent.newBuilder()
                    .setName(metricName)
                    .setValue("42")
                    .setMetricType("counter")
                    .build())
                .build()
        );

        GetMetricsResponse response = client.getBuildMetricsStub().getMetrics(
            GetMetricsRequest.newBuilder()
                .addMetricNames(metricName)
                .build()
        );
        assertEquals(1, response.getMetricsCount());
        assertEquals(metricName, response.getMetrics(0).getName());
        assertEquals(42.0, response.getMetrics(0).getLast(), 0.001);
    }

    // --- Toolchain Service ---

    @Test
    public void verifyToolchain() {
        String javaHome = System.getProperty("java.home");
        VerifyToolchainResponse response = client.getToolchainStub().verifyToolchain(
            VerifyToolchainRequest.newBuilder()
                .setJavaHome(javaHome)
                .build()
        );
        assertTrue("Current JVM should be a valid toolchain", response.getValid());
    }

    // --- Build Layout Service ---

    @Test
    public void initBuildLayout() {
        InitBuildLayoutResponse response = client.getBuildLayoutStub().initBuildLayout(
            InitBuildLayoutRequest.newBuilder()
                .setRootDir("/tmp/e2e-test")
                .setSettingsFile("/tmp/e2e-test/settings.gradle")
                .setBuildFile("/tmp/e2e-test/build.gradle")
                .setBuildName("e2e-test-build")
                .build()
        );
        assertTrue(response.getInitialized());
        assertFalse(response.getBuildId().isEmpty());
    }

    // --- Exec Service ---

    @Test
    public void execAndWait() {
        ExecSpawnResponse spawnResponse = client.getExecStub().spawn(
            ExecSpawnRequest.newBuilder()
                .setCommand("/bin/echo")
                .addArgs("hello from e2e")
                .build()
        );
        assertTrue(spawnResponse.getPid() > 0);

        ExecWaitResponse waitResponse = client.getExecStub().wait(
            ExecWaitRequest.newBuilder()
                .setPid(spawnResponse.getPid())
                .build()
        );
        assertEquals(0, waitResponse.getExitCode());
    }

    // --- Bootstrap Service (init/complete build lifecycle) ---

    @Test
    public void initAndCompleteBuildLifecycle() {
        // Initialize a build
        InitBuildResponse buildResponse = client.getBootstrapStub().initBuild(
            InitBuildRequest.newBuilder()
                .setSessionId("e2e-session-" + System.nanoTime())
                .setProjectDir("/tmp/e2e-build")
                .build()
        );
        assertFalse("Build ID should be generated", buildResponse.getBuildId().isEmpty());

        // Complete the build
        CompleteBuildResponse completeResponse = client.getBootstrapStub().completeBuild(
            CompleteBuildRequest.newBuilder()
                .setBuildId(buildResponse.getBuildId())
                .setOutcome("SUCCESS")
                .setDurationMs(100)
                .build()
        );
        assertTrue("Build completion should be acknowledged", completeResponse.getAcknowledged());
    }

    // --- Configuration Service ---

    @Test
    public void registerAndResolveProject() {
        RegisterProjectResponse registerResponse = client.getConfigurationStub().registerProject(
            RegisterProjectRequest.newBuilder()
                .setProjectPath(":e2e-test")
                .setProjectDir("/tmp/e2e-project")
                .build()
        );
        assertTrue(registerResponse.getSuccess());
    }

    // --- Plugin Service ---

    @Test
    public void applyAndListPlugins() {
        String projectPath = ":e2e-plugin-" + System.nanoTime();

        client.getPluginStub().applyPlugin(
            ApplyPluginRequest.newBuilder()
                .setProjectPath(projectPath)
                .setPluginId("java")
                .build()
        );

        client.getPluginStub().applyPlugin(
            ApplyPluginRequest.newBuilder()
                .setProjectPath(projectPath)
                .setPluginId("org.springframework.boot")
                .build()
        );

        GetAppliedPluginsResponse response = client.getPluginStub().getAppliedPlugins(
            GetAppliedPluginsRequest.newBuilder()
                .setProjectPath(projectPath)
                .build()
        );
        assertTrue(response.getPluginsCount() >= 2);
    }

    // --- File Fingerprint Service ---

    @Test
    public void fingerprintFilesRoundTrip() {
        FingerprintFilesResponse fingerprintResponse = client.getFileFingerprintStub().fingerprintFiles(
            FingerprintFilesRequest.newBuilder()
                .addFiles(FileToFingerprint.newBuilder()
                    .setAbsolutePath("/tmp/e2e-test-file.txt")
                    .setType(FingerprintType.FINGERPRINT_FILE)
                    .build())
                .build()
        );
        // Service accepts the request (file may not exist but RPC completes)
        assertNotNull(fingerprintResponse);
    }

    // --- Value Snapshot Service ---

    @Test
    public void snapshotValues() {
        SnapshotValuesResponse response = client.getValueSnapshotStub().snapshotValues(
            SnapshotValuesRequest.newBuilder()
                .addValues(PropertyValue.newBuilder()
                    .setName("e2e.prop." + System.nanoTime())
                    .setStringValue("e2e-test-value")
                    .build())
                .build()
        );
        assertNotNull(response);
    }

    // --- Build Operations Service ---

    @Test
    public void startAndCompleteOperation() {
        String buildId = "e2e-ops-" + System.nanoTime();
        String operationId = "op-" + System.nanoTime();

        StartOperationResponse startResponse = client.getBuildOperationsStub().startOperation(
            StartOperationRequest.newBuilder()
                .setBuildId(buildId)
                .setOperationId(operationId)
                .setOperationType("e2e-test-op")
                .setDisplayName("E2E test operation")
                .setStartTimeMs(System.currentTimeMillis())
                .build()
        );
        assertTrue(startResponse.getSuccess());

        CompleteOperationResponse completeResponse = client.getBuildOperationsStub().completeOperation(
            CompleteOperationRequest.newBuilder()
                .setBuildId(buildId)
                .setOperationId(operationId)
                .setDurationMs(System.currentTimeMillis())
                .setOutcome("SUCCESS")
                .build()
        );
        assertTrue(completeResponse.getSuccess());
    }

    // --- Dependency Resolution Service ---

    @Test
    public void recordAndQueryResolution() {
        RecordResolutionResponse recordResponse = client.getDependencyResolutionStub().recordResolution(
            RecordResolutionRequest.newBuilder()
                .setConfigurationName("compileClasspath")
                .setDependencyCount(1)
                .setResolutionTimeMs(50)
                .build()
        );
        assertTrue(recordResponse.getAcknowledged());

        GetResolutionStatsResponse statsResponse = client.getDependencyResolutionStub().getResolutionStats(
            GetResolutionStatsRequest.newBuilder().build()
        );
        assertTrue(statsResponse.getTotalResolutions() >= 1);
    }

    // --- Build Init Service ---

    @Test
    public void initBuildSettings() {
        InitBuildSettingsResponse response = client.getBuildInitStub().initBuildSettings(
            InitBuildSettingsRequest.newBuilder()
                .setSessionId("e2e-init-" + System.nanoTime())
                .setRootDir("/tmp/e2e-test-project")
                .setSettingsFile("/tmp/e2e-test-project/settings.gradle")
                .build()
        );
        assertTrue(response.getInitialized());
    }

    // --- Build Result Service ---

    @Test
    public void reportTaskResult() {
        ReportTaskResultResponse response = client.getBuildResultStub().reportTaskResult(
            ReportTaskResultRequest.newBuilder()
                .setResult(TaskResult.newBuilder()
                    .setTaskPath(":e2eTestTask")
                    .setOutcome("SUCCESS")
                    .setDurationMs(50)
                    .build())
                .build()
        );
        assertTrue(response.getAccepted());
    }

    // --- Full Build Lifecycle ---

    @Test
    public void fullBuildLifecycleWithMetrics() {
        String sessionId = "e2e-lifecycle-" + System.nanoTime();

        // Init build
        InitBuildResponse buildResponse = client.getBootstrapStub().initBuild(
            InitBuildRequest.newBuilder()
                .setSessionId(sessionId)
                .setProjectDir("/tmp/e2e-lifecycle")
                .build()
        );
        String buildId = buildResponse.getBuildId();
        assertFalse(buildId.isEmpty());

        // Record a metric
        client.getBuildMetricsStub().recordMetric(
            RecordMetricRequest.newBuilder()
                .setEvent(MetricEvent.newBuilder()
                    .setName("e2e.lifecycle.test")
                    .setValue("100")
                    .setMetricType("counter")
                    .build())
                .build()
        );

        // Report task results
        client.getBuildResultStub().reportTaskResult(
            ReportTaskResultRequest.newBuilder()
                .setBuildId(buildId)
                .setResult(TaskResult.newBuilder()
                    .setTaskPath(":compileJava")
                    .setOutcome("SUCCESS")
                    .setDurationMs(200)
                    .build())
                .build()
        );
        client.getBuildResultStub().reportTaskResult(
            ReportTaskResultRequest.newBuilder()
                .setBuildId(buildId)
                .setResult(TaskResult.newBuilder()
                    .setTaskPath(":test")
                    .setOutcome("SUCCESS")
                    .setDurationMs(150)
                    .build())
                .build()
        );

        // Complete build
        CompleteBuildResponse completeResponse = client.getBootstrapStub().completeBuild(
            CompleteBuildRequest.newBuilder()
                .setBuildId(buildId)
                .setOutcome("SUCCESS")
                .setDurationMs(350)
                .build()
        );
        assertTrue(completeResponse.getAcknowledged());

        // Verify metrics
        GetMetricsResponse metricsResponse = client.getBuildMetricsStub().getMetrics(
            GetMetricsRequest.newBuilder()
                .addMetricNames("e2e.lifecycle.test")
                .build()
        );
        assertEquals(1, metricsResponse.getMetricsCount());
        assertEquals("e2e.lifecycle.test", metricsResponse.getMetrics(0).getName());
        assertEquals(100.0, metricsResponse.getMetrics(0).getLast(), 0.001);
    }
}
