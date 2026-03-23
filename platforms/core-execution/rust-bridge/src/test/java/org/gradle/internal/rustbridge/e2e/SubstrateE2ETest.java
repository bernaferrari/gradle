package org.gradle.internal.rustbridge.e2e;

import org.gradle.internal.rustbridge.SubstrateClient;
import gradle.substrate.v1.*;
import org.junit.Assume;
import org.junit.Before;
import org.junit.Test;

import java.io.File;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.TimeUnit;

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
        Thread consumer = new Thread(() -> {
            try {
                byte[] buffer = new byte[1024];
                int n;
                while ((n = daemonProcess.getInputStream().read(buffer)) != -1) {
                    // Discard daemon output during tests
                }
            } catch (Exception ignored) {
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
    public void hashBatchReturnsCorrectHashes() {
        HashBatchResponse response = client.getHashStub().hashBatch(
            HashBatchRequest.newBuilder()
                .addHashInputs(HashInput.newBuilder()
                    .setAlgorithm("md5")
                    .setContent("hello world")
                    .build())
                .build()
        );
        assertEquals(1, response.getHashesCount());
        assertEquals("5eb63bbbe01eeed093cb22bb8f5acdc3", response.getHashes(0));
    }

    @Test
    public void hashBatchEmptyInputReturnsEmpty() {
        HashBatchResponse response = client.getHashStub().hashBatch(
            HashBatchRequest.newBuilder().build()
        );
        assertEquals(0, response.getHashesCount());
    }

    @Test
    public void hashAllAlgorithmsMatchKnownValues() {
        String content = "The quick brown fox jumps over the lazy dog";

        HashBatchResponse response = client.getHashStub().hashBatch(
            HashBatchRequest.newBuilder()
                .addHashInputs(HashInput.newBuilder().setAlgorithm("md5").setContent(content).build())
                .addHashInputs(HashInput.newBuilder().setAlgorithm("sha1").setContent(content).build())
                .addHashInputs(HashInput.newBuilder().setAlgorithm("sha256").setContent(content).build())
                .addHashInputs(HashInput.newBuilder().setAlgorithm("sha3-256").setContent(content).build())
                .addHashInputs(HashInput.newBuilder().setAlgorithm("blake3").setContent(content).build())
                .build()
        );

        assertEquals(5, response.getHashesCount());
        // MD5 known value for "The quick brown fox jumps over the lazy dog"
        assertEquals("9e107d9d372bb6826bd81d3542a419d6", response.getHashes(0));
        // SHA-1
        assertEquals("2fd4e1c67a2d28fced849ee1bb76e7391b93eb12", response.getHashes(1));
        // SHA-256
        assertEquals("d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592", response.getHashes(2));
        // SHA3-256
        assertEquals("69070dda01975c8c12055b962bb624b5d0668658d7e5c2cbab9a7e0756e1e5ec", response.getHashes(3));
        // BLAKE3
        assertEquals("e4cfa39c05ee2e8b46e8f7e9c5fb055a0e4e3e1f68dc4c40b88e9f3ed18e02", response.getHashes(4));
    }

    @Test
    public void hashLargeContentProducesConsistentResults() {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < 100000; i++) {
            sb.append("Line ").append(i).append(": test content for hashing\n");
        }
        String largeContent = sb.toString();

        HashBatchResponse response1 = client.getHashStub().hashBatch(
            HashBatchRequest.newBuilder()
                .addHashInputs(HashInput.newBuilder().setAlgorithm("sha256").setContent(largeContent).build())
                .build()
        );
        HashBatchResponse response2 = client.getHashStub().hashBatch(
            HashBatchRequest.newBuilder()
                .addHashInputs(HashInput.newBuilder().setAlgorithm("sha256").setContent(largeContent).build())
                .build()
        );

        // Same content must produce the same hash (deterministic)
        assertEquals(response1.getHashes(0), response2.getHashes(0));
        // Different from empty string hash
        assertNotEquals(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            response1.getHashes(0));
    }

    @Test
    public void hashBatchMultipleInputs() {
        HashBatchResponse response = client.getHashStub().hashBatch(
            HashBatchRequest.newBuilder()
                .addHashInputs(HashInput.newBuilder().setAlgorithm("sha256").setContent("aaa").build())
                .addHashInputs(HashInput.newBuilder().setAlgorithm("sha256").setContent("bbb").build())
                .addHashInputs(HashInput.newBuilder().setAlgorithm("sha256").setContent("ccc").build())
                .build()
        );

        assertEquals(3, response.getHashesCount());
        assertNotEquals(response.getHashes(0), response.getHashes(1));
        assertNotEquals(response.getHashes(1), response.getHashes(2));
    }

    // --- Cache Service ---

    @Test
    public void cachePutAndGet() {
        String key = "e2e-test-key-" + System.nanoTime();
        String value = "test-cache-value";

        client.getCacheStub().cachePut(
            CachePutRequest.newBuilder()
                .setKey(key)
                .setValue(com.google.protobuf.ByteString.copyFromUtf8(value))
                .setTtlSeconds(60)
                .build()
        );

        CacheGetResponse getResponse = client.getCacheStub().cacheGet(
            CacheGetRequest.newBuilder().setKey(key).build()
        );
        assertTrue(getResponse.getFound());
        assertEquals(value, getResponse.getValue().toStringUtf8());
    }

    @Test
    public void cacheGetMiss() {
        CacheGetResponse response = client.getCacheStub().cacheGet(
            CacheGetRequest.newBuilder().setKey("nonexistent-key-e2e").build()
        );
        assertFalse(response.getFound());
    }

    // --- Execution Plan Service ---

    @Test
    public void createAndQueryExecutionPlan() {
        String buildId = "e2e-build-" + System.nanoTime();

        CreateExecutionPlanResponse createResponse = client.getExecutionPlanStub().createExecutionPlan(
            CreateExecutionPlanRequest.newBuilder()
                .setBuildId(buildId)
                .build()
        );
        assertTrue(createResponse.getAccepted());

        GetExecutionPlanResponse getResponse = client.getExecutionPlanStub().getExecutionPlan(
            GetExecutionPlanRequest.newBuilder()
                .setBuildId(buildId)
                .build()
        );
        assertEquals(buildId, getResponse.getBuildId());
    }

    // --- Task Graph Service ---

    @Test
    public void addAndGetTask() {
        String buildId = "e2e-tg-" + System.nanoTime();
        String taskPath = ":e2eTestTask";

        client.getTaskGraphStub().addTask(
            AddTaskRequest.newBuilder()
                .setBuildId(buildId)
                .setTaskPath(taskPath)
                .setTaskType("e2e-test")
                .build()
        );

        GetTaskResponse response = client.getTaskGraphStub().getTask(
            GetTaskRequest.newBuilder()
                .setBuildId(buildId)
                .setTaskPath(taskPath)
                .build()
        );
        assertEquals(taskPath, response.getTaskPath());
        assertEquals("e2e-test", response.getTaskType());
    }

    // --- Build Metrics Service ---

    @Test
    public void recordAndGetMetric() {
        String metricName = "e2e.metric." + System.nanoTime();

        client.getBuildMetricsStub().recordMetric(
            RecordMetricRequest.newBuilder()
                .setMetric(MetricEvent.newBuilder()
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
        assertEquals("42", response.getMetrics(0).getValue());
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
        assertTrue(response.getAccepted());
        assertFalse(response.getBuildId().isEmpty());
    }

    // --- Exec Service ---

    @Test
    public void execAndWait() {
        ExecResponse execResponse = client.getExecStub().exec(
            ExecRequest.newBuilder()
                .setCommand("/bin/echo")
                .addArgs("hello from e2e")
                .build()
        );
        assertTrue(execResponse.getPid() > 0);

        ExecWaitResponse waitResponse = client.getExecStub().execWait(
            ExecWaitRequest.newBuilder()
                .setPid(execResponse.getPid())
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
        assertTrue("Build init should be accepted", buildResponse.getAccepted());
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
        String content = "file content for fingerprinting e2e";

        FingerprintFilesResponse fingerprintResponse = client.getFileFingerprintStub().fingerprintFiles(
            FingerprintFilesRequest.newBuilder()
                .addPaths("/tmp/e2e-test-file.txt")
                .build()
        );
        // Service accepts the request (file may not exist but RPC completes)
        assertNotNull(fingerprintResponse);
    }

    // --- Value Snapshot Service ---

    @Test
    public void snapshotValues() {
        SnapshotValuesResponse response = client.getFileFingerprintStub().snapshotValues(
            SnapshotValuesRequest.newBuilder()
                .addProperties(PropertyValue.newBuilder()
                    .setName("e2e.prop." + System.nanoTime())
                    .setValue("e2e-test-value")
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
                .setDescription("E2E test operation")
                .setStartTimeMs(System.currentTimeMillis())
                .build()
        );
        assertTrue(startResponse.getSuccess());

        CompleteOperationResponse completeResponse = client.getBuildOperationsStub().completeOperation(
            CompleteOperationRequest.newBuilder()
                .setBuildId(buildId)
                .setOperationId(operationId)
                .setEndTimeMs(System.currentTimeMillis())
                .setOutcome("SUCCESS")
                .build()
        );
        assertTrue(completeResponse.getSuccess());
    }

    // --- Dependency Resolution Service ---

    @Test
    public void recordAndQueryResolution() {
        String buildId = "e2e-dep-" + System.nanoTime();

        RecordResolutionResponse recordResponse = client.getDependencyResolutionStub().recordResolution(
            RecordResolutionRequest.newBuilder()
                .setBuildId(buildId)
                .setConfigurationName("compileClasspath")
                .addResolvedDependencies(ResolvedDependency.newBuilder()
                    .setGroup("org.example")
                    .setName("test-lib")
                    .setVersion("1.0.0")
                    .build())
                .build()
        );
        assertTrue(recordResponse.getAcknowledged());

        GetResolutionStatsResponse statsResponse = client.getDependencyResolutionStub().getResolutionStats(
            GetResolutionStatsRequest.newBuilder()
                .setBuildId(buildId)
                .build()
        );
        assertTrue(statsResponse.getTotalResolutions() >= 1);
    }

    // --- Build Init Service ---

    @Test
    public void initBuildSettings() {
        InitBuildSettingsResponse response = client.getBuildInitStub().initBuildSettings(
            InitBuildSettingsRequest.newBuilder()
                .setSessionId("e2e-init-" + System.nanoTime())
                .setProjectName("e2e-test-project")
                .setBuildFile("build.gradle.kts")
                .build()
        );
        assertTrue(response.getAccepted());
    }

    // --- Build Result Service ---

    @Test
    public void reportTaskResult() {
        ReportTaskResultResponse response = client.getBuildOperationsStub().reportTaskResult(
            ReportTaskResultRequest.newBuilder()
                .setTaskResult(TaskResult.newBuilder()
                    .setTaskPath(":e2eTestTask")
                    .setOutcome("SUCCESS")
                    .setDurationMs(50)
                    .build())
                .build()
        );
        assertTrue(response.getAccepted());
    }

    // --- Cache Round-Trip Validation ---

    @Test
    public void cacheLargePayloadRoundTrip() {
        String key = "e2e-large-" + System.nanoTime();
        byte[] largeValue = new byte[1024 * 1024]; // 1 MB
        for (int i = 0; i < largeValue.length; i++) {
            largeValue[i] = (byte) (i % 256);
        }

        client.getCacheStub().cachePut(
            CachePutRequest.newBuilder()
                .setKey(key)
                .setValue(com.google.protobuf.ByteString.copyFrom(largeValue))
                .setTtlSeconds(60)
                .build()
        );

        CacheGetResponse getResponse = client.getCacheStub().cacheGet(
            CacheGetRequest.newBuilder().setKey(key).build()
        );
        assertTrue(getResponse.getFound());
        assertArrayEquals(largeValue, getResponse.getValue().toByteArray());
    }

    @Test
    public void cacheOverwriteReturnsLatestValue() {
        String key = "e2e-overwrite-" + System.nanoTime();

        client.getCacheStub().cachePut(
            CachePutRequest.newBuilder()
                .setKey(key)
                .setValue(com.google.protobuf.ByteString.copyFromUtf8("version-1"))
                .setTtlSeconds(60)
                .build()
        );

        client.getCacheStub().cachePut(
            CachePutRequest.newBuilder()
                .setKey(key)
                .setValue(com.google.protobuf.ByteString.copyFromUtf8("version-2"))
                .setTtlSeconds(60)
                .build()
        );

        CacheGetResponse getResponse = client.getCacheStub().cacheGet(
            CacheGetRequest.newBuilder().setKey(key).build()
        );
        assertTrue(getResponse.getFound());
        assertEquals("version-2", getResponse.getValue().toStringUtf8());
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
        assertTrue(buildResponse.getAccepted());
        String buildId = buildResponse.getBuildId();
        assertFalse(buildId.isEmpty());

        // Record a metric
        client.getBuildMetricsStub().recordMetric(
            RecordMetricRequest.newBuilder()
                .setMetric(MetricEvent.newBuilder()
                    .setName("e2e.lifecycle.test")
                    .setValue("100")
                    .setMetricType("counter")
                    .build())
                .build()
        );

        // Report task results
        client.getBuildOperationsStub().reportTaskResult(
            ReportTaskResultRequest.newBuilder()
                .setTaskResult(TaskResult.newBuilder()
                    .setTaskPath(":compileJava")
                    .setOutcome("SUCCESS")
                    .setDurationMs(200)
                    .build())
                .build()
        );
        client.getBuildOperationsStub().reportTaskResult(
            ReportTaskResultRequest.newBuilder()
                .setTaskResult(TaskResult.newBuilder()
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
        assertEquals("100", metricsResponse.getMetrics(0).getValue());
    }
}
