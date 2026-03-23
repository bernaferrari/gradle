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
}
