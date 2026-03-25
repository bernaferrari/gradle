package org.gradle.internal.rustbridge;

import io.grpc.ManagedChannel;
import io.grpc.netty.shaded.io.grpc.netty.NettyChannelBuilder;
import io.grpc.netty.shaded.io.netty.channel.unix.DomainSocketAddress;
import gradle.substrate.v1.*;

import java.io.Closeable;
import java.io.IOException;
import java.lang.management.ManagementFactory;
import java.util.concurrent.TimeUnit;

/**
 * gRPC client for communicating with the Rust substrate daemon.
 * Connects via Unix domain socket.
 *
 * <p>Provides blocking stubs for all 33 substrate services.
 * When the substrate is disabled (noop mode), all stub getters
 * throw {@link SubstrateException}.</p>
 */
public class SubstrateClient implements Closeable {

    private static final String CLIENT_PROTOCOL_VERSION = "1.0.0";

    private final ManagedChannel channel;
    private final boolean noop;

    // Phase 0: Control
    private final ControlServiceGrpc.ControlServiceBlockingStub controlStub;
    // Phase 1: Hashing
    private final HashServiceGrpc.HashServiceBlockingStub hashStub;
    // Phase 2: Build cache
    private final CacheServiceGrpc.CacheServiceBlockingStub cacheStub;
    private final CacheServiceGrpc.CacheServiceStub cacheAsyncStub;
    // Phase 3: Process execution
    private final ExecServiceGrpc.ExecServiceBlockingStub execStub;
    // Phase 4: Work scheduling
    private final WorkServiceGrpc.WorkServiceBlockingStub workStub;
    // Phase 5-6: Execution planning
    private final ExecutionPlanServiceGrpc.ExecutionPlanServiceBlockingStub executionPlanStub;
    // Phase 7: Execution history
    private final ExecutionHistoryServiceGrpc.ExecutionHistoryServiceBlockingStub executionHistoryStub;
    // Phase 8: Build cache orchestration
    private final BuildCacheOrchestrationServiceGrpc.BuildCacheOrchestrationServiceBlockingStub cacheOrchestrationStub;
    // Phase 9: File fingerprinting
    private final FileFingerprintServiceGrpc.FileFingerprintServiceBlockingStub fileFingerprintStub;
    // Phase 10: Value snapshotting
    private final ValueSnapshotServiceGrpc.ValueSnapshotServiceBlockingStub valueSnapshotStub;
    // Phase 11: Task graph
    private final TaskGraphServiceGrpc.TaskGraphServiceBlockingStub taskGraphStub;
    // Phase 12: Configuration model
    private final ConfigurationServiceGrpc.ConfigurationServiceBlockingStub configurationStub;
    // Phase 13: Plugin system
    private final PluginServiceGrpc.PluginServiceBlockingStub pluginStub;
    // Phase 14: Build operations
    private final BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub buildOperationsStub;
    // Phase 15: Bootstrap
    private final BootstrapServiceGrpc.BootstrapServiceBlockingStub bootstrapStub;
    // Phase 18: Dependency resolution
    private final DependencyResolutionServiceGrpc.DependencyResolutionServiceBlockingStub dependencyResolutionStub;
    // Phase 19: File watching
    private final FileWatchServiceGrpc.FileWatchServiceBlockingStub fileWatchStub;
    // Phase 20: Configuration cache
    private final ConfigurationCacheServiceGrpc.ConfigurationCacheServiceBlockingStub configCacheStub;
    // Phase 23: Toolchain management
    private final ToolchainServiceGrpc.ToolchainServiceBlockingStub toolchainStub;
    // Phase 24: Build event streaming
    private final BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub buildEventStreamStub;
    // Phase 25: Worker process management
    private final WorkerProcessServiceGrpc.WorkerProcessServiceBlockingStub workerProcessStub;
    // Phase 26: Build layout / project model
    private final BuildLayoutServiceGrpc.BuildLayoutServiceBlockingStub buildLayoutStub;
    // Phase 28: Build result reporting
    private final BuildResultServiceGrpc.BuildResultServiceBlockingStub buildResultStub;
    // Phase 29: Problem / diagnostic reporting
    private final ProblemReportingServiceGrpc.ProblemReportingServiceBlockingStub problemReportingStub;
    // Phase 30: Resource management
    private final ResourceManagementServiceGrpc.ResourceManagementServiceBlockingStub resourceManagementStub;
    // Phase 31: Build comparison
    private final BuildComparisonServiceGrpc.BuildComparisonServiceBlockingStub buildComparisonStub;
    // Phase 32: Console / rich output
    private final ConsoleServiceGrpc.ConsoleServiceBlockingStub consoleStub;
    // Phase 33: Test execution
    private final TestExecutionServiceGrpc.TestExecutionServiceBlockingStub testExecutionStub;
    // Phase 34: Artifact publishing
    private final ArtifactPublishingServiceGrpc.ArtifactPublishingServiceBlockingStub artifactPublishingStub;
    // Phase 35: Build initialization
    private final BuildInitServiceGrpc.BuildInitServiceBlockingStub buildInitStub;
    // Phase 36: Incremental compilation
    private final IncrementalCompilationServiceGrpc.IncrementalCompilationServiceBlockingStub incrementalCompilationStub;
    // Phase 37: Build metrics
    private final BuildMetricsServiceGrpc.BuildMetricsServiceBlockingStub buildMetricsStub;
    // Phase 38: Garbage collection
    private final GarbageCollectionServiceGrpc.GarbageCollectionServiceBlockingStub garbageCollectionStub;
    // Phase 39: Parser
    private final ParserServiceGrpc.ParserServiceBlockingStub parserStub;
    // Phase 6: JVM Compatibility Host
    private final String jvmHostSocketPath;

    private SubstrateClient(ManagedChannel channel, boolean noop, String jvmHostSocketPath) {
        this.channel = channel;
        this.noop = noop;
        this.jvmHostSocketPath = jvmHostSocketPath;
        if (noop) {
            this.controlStub = null;
            this.hashStub = null;
            this.cacheStub = null;
            this.cacheAsyncStub = null;
            this.execStub = null;
            this.workStub = null;
            this.executionPlanStub = null;
            this.executionHistoryStub = null;
            this.cacheOrchestrationStub = null;
            this.fileFingerprintStub = null;
            this.valueSnapshotStub = null;
            this.taskGraphStub = null;
            this.configurationStub = null;
            this.pluginStub = null;
            this.buildOperationsStub = null;
            this.bootstrapStub = null;
            this.dependencyResolutionStub = null;
            this.fileWatchStub = null;
            this.configCacheStub = null;
            this.toolchainStub = null;
            this.buildEventStreamStub = null;
            this.workerProcessStub = null;
            this.buildLayoutStub = null;
            this.buildResultStub = null;
            this.problemReportingStub = null;
            this.resourceManagementStub = null;
            this.buildComparisonStub = null;
            this.consoleStub = null;
            this.testExecutionStub = null;
            this.artifactPublishingStub = null;
            this.buildInitStub = null;
            this.incrementalCompilationStub = null;
            this.buildMetricsStub = null;
            this.garbageCollectionStub = null;
            this.parserStub = null;
        } else {
            this.controlStub = ControlServiceGrpc.newBlockingStub(channel);
            this.hashStub = HashServiceGrpc.newBlockingStub(channel);
            this.cacheStub = CacheServiceGrpc.newBlockingStub(channel);
            this.cacheAsyncStub = CacheServiceGrpc.newStub(channel);
            this.execStub = ExecServiceGrpc.newBlockingStub(channel);
            this.workStub = WorkServiceGrpc.newBlockingStub(channel);
            this.executionPlanStub = ExecutionPlanServiceGrpc.newBlockingStub(channel);
            this.executionHistoryStub = ExecutionHistoryServiceGrpc.newBlockingStub(channel);
            this.cacheOrchestrationStub = BuildCacheOrchestrationServiceGrpc.newBlockingStub(channel);
            this.fileFingerprintStub = FileFingerprintServiceGrpc.newBlockingStub(channel);
            this.valueSnapshotStub = ValueSnapshotServiceGrpc.newBlockingStub(channel);
            this.taskGraphStub = TaskGraphServiceGrpc.newBlockingStub(channel);
            this.configurationStub = ConfigurationServiceGrpc.newBlockingStub(channel);
            this.pluginStub = PluginServiceGrpc.newBlockingStub(channel);
            this.buildOperationsStub = BuildOperationsServiceGrpc.newBlockingStub(channel);
            this.bootstrapStub = BootstrapServiceGrpc.newBlockingStub(channel);
            this.dependencyResolutionStub = DependencyResolutionServiceGrpc.newBlockingStub(channel);
            this.fileWatchStub = FileWatchServiceGrpc.newBlockingStub(channel);
            this.configCacheStub = ConfigurationCacheServiceGrpc.newBlockingStub(channel);
            this.toolchainStub = ToolchainServiceGrpc.newBlockingStub(channel);
            this.buildEventStreamStub = BuildEventStreamServiceGrpc.newBlockingStub(channel);
            this.workerProcessStub = WorkerProcessServiceGrpc.newBlockingStub(channel);
            this.buildLayoutStub = BuildLayoutServiceGrpc.newBlockingStub(channel);
            this.buildResultStub = BuildResultServiceGrpc.newBlockingStub(channel);
            this.problemReportingStub = ProblemReportingServiceGrpc.newBlockingStub(channel);
            this.resourceManagementStub = ResourceManagementServiceGrpc.newBlockingStub(channel);
            this.buildComparisonStub = BuildComparisonServiceGrpc.newBlockingStub(channel);
            this.consoleStub = ConsoleServiceGrpc.newBlockingStub(channel);
            this.testExecutionStub = TestExecutionServiceGrpc.newBlockingStub(channel);
            this.artifactPublishingStub = ArtifactPublishingServiceGrpc.newBlockingStub(channel);
            this.buildInitStub = BuildInitServiceGrpc.newBlockingStub(channel);
            this.incrementalCompilationStub = IncrementalCompilationServiceGrpc.newBlockingStub(channel);
            this.buildMetricsStub = BuildMetricsServiceGrpc.newBlockingStub(channel);
            this.garbageCollectionStub = GarbageCollectionServiceGrpc.newBlockingStub(channel);
            this.parserStub = ParserServiceGrpc.newBlockingStub(channel);
        }
    }

    /**
     * Creates a SubstrateClient connected to the given Unix socket path.
     */
    public static SubstrateClient connect(String socketPath) throws IOException {
        return connect(socketPath, null);
    }

    /**
     * Creates a SubstrateClient connected to the given Unix socket path,
     * with an optional JVM host socket path for reverse-direction RPC.
     */
    public static SubstrateClient connect(String socketPath, String jvmHostSocketPath) throws IOException {
        ManagedChannel channel = NettyChannelBuilder
            .forAddress(new DomainSocketAddress(socketPath))
            .usePlaintext()
            .build();
        SubstrateClient client = new SubstrateClient(channel, false, jvmHostSocketPath);
        try {
            client.performHandshake();
        } catch (IOException e) {
            client.close();
            throw e;
        }
        return client;
    }

    /**
     * Creates a no-op client that does nothing. Used when the substrate is disabled.
     */
    public static SubstrateClient noop() {
        return new SubstrateClient(null, true, null);
    }

    public boolean isNoop() {
        return noop;
    }

    /**
     * Get the JVM host socket path that will be passed in the handshake.
     */
    public String getJvmHostSocketPath() {
        return jvmHostSocketPath;
    }

    // -- Stub getters --

    public ControlServiceGrpc.ControlServiceBlockingStub getControlStub() {
        throwIfNoop();
        return controlStub;
    }

    public HashServiceGrpc.HashServiceBlockingStub getHashStub() {
        throwIfNoop();
        return hashStub;
    }

    public CacheServiceGrpc.CacheServiceBlockingStub getCacheStub() {
        throwIfNoop();
        return cacheStub;
    }

    public CacheServiceGrpc.CacheServiceStub getCacheAsyncStub() {
        throwIfNoop();
        return cacheAsyncStub;
    }

    public ExecServiceGrpc.ExecServiceBlockingStub getExecStub() {
        throwIfNoop();
        return execStub;
    }

    public WorkServiceGrpc.WorkServiceBlockingStub getWorkStub() {
        throwIfNoop();
        return workStub;
    }

    public ExecutionPlanServiceGrpc.ExecutionPlanServiceBlockingStub getExecutionPlanStub() {
        throwIfNoop();
        return executionPlanStub;
    }

    public ExecutionHistoryServiceGrpc.ExecutionHistoryServiceBlockingStub getExecutionHistoryStub() {
        throwIfNoop();
        return executionHistoryStub;
    }

    public BuildCacheOrchestrationServiceGrpc.BuildCacheOrchestrationServiceBlockingStub getCacheOrchestrationStub() {
        throwIfNoop();
        return cacheOrchestrationStub;
    }

    public FileFingerprintServiceGrpc.FileFingerprintServiceBlockingStub getFileFingerprintStub() {
        throwIfNoop();
        return fileFingerprintStub;
    }

    public ValueSnapshotServiceGrpc.ValueSnapshotServiceBlockingStub getValueSnapshotStub() {
        throwIfNoop();
        return valueSnapshotStub;
    }

    public TaskGraphServiceGrpc.TaskGraphServiceBlockingStub getTaskGraphStub() {
        throwIfNoop();
        return taskGraphStub;
    }

    public ConfigurationServiceGrpc.ConfigurationServiceBlockingStub getConfigurationStub() {
        throwIfNoop();
        return configurationStub;
    }

    public PluginServiceGrpc.PluginServiceBlockingStub getPluginStub() {
        throwIfNoop();
        return pluginStub;
    }

    public BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub getBuildOperationsStub() {
        throwIfNoop();
        return buildOperationsStub;
    }

    public BootstrapServiceGrpc.BootstrapServiceBlockingStub getBootstrapStub() {
        throwIfNoop();
        return bootstrapStub;
    }

    public DependencyResolutionServiceGrpc.DependencyResolutionServiceBlockingStub getDependencyResolutionStub() {
        throwIfNoop();
        return dependencyResolutionStub;
    }

    public FileWatchServiceGrpc.FileWatchServiceBlockingStub getFileWatchStub() {
        throwIfNoop();
        return fileWatchStub;
    }

    public ConfigurationCacheServiceGrpc.ConfigurationCacheServiceBlockingStub getConfigCacheStub() {
        throwIfNoop();
        return configCacheStub;
    }

    public ToolchainServiceGrpc.ToolchainServiceBlockingStub getToolchainStub() {
        throwIfNoop();
        return toolchainStub;
    }

    public BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub getBuildEventStreamStub() {
        throwIfNoop();
        return buildEventStreamStub;
    }

    public WorkerProcessServiceGrpc.WorkerProcessServiceBlockingStub getWorkerProcessStub() {
        throwIfNoop();
        return workerProcessStub;
    }

    public BuildLayoutServiceGrpc.BuildLayoutServiceBlockingStub getBuildLayoutStub() {
        throwIfNoop();
        return buildLayoutStub;
    }

    public BuildResultServiceGrpc.BuildResultServiceBlockingStub getBuildResultStub() {
        throwIfNoop();
        return buildResultStub;
    }

    public ProblemReportingServiceGrpc.ProblemReportingServiceBlockingStub getProblemReportingStub() {
        throwIfNoop();
        return problemReportingStub;
    }

    public ResourceManagementServiceGrpc.ResourceManagementServiceBlockingStub getResourceManagementStub() {
        throwIfNoop();
        return resourceManagementStub;
    }

    public BuildComparisonServiceGrpc.BuildComparisonServiceBlockingStub getBuildComparisonStub() {
        throwIfNoop();
        return buildComparisonStub;
    }

    public ConsoleServiceGrpc.ConsoleServiceBlockingStub getConsoleStub() {
        throwIfNoop();
        return consoleStub;
    }

    public TestExecutionServiceGrpc.TestExecutionServiceBlockingStub getTestExecutionStub() {
        throwIfNoop();
        return testExecutionStub;
    }

    public ArtifactPublishingServiceGrpc.ArtifactPublishingServiceBlockingStub getArtifactPublishingStub() {
        throwIfNoop();
        return artifactPublishingStub;
    }

    public BuildInitServiceGrpc.BuildInitServiceBlockingStub getBuildInitStub() {
        throwIfNoop();
        return buildInitStub;
    }

    public IncrementalCompilationServiceGrpc.IncrementalCompilationServiceBlockingStub getIncrementalCompilationStub() {
        throwIfNoop();
        return incrementalCompilationStub;
    }

    public BuildMetricsServiceGrpc.BuildMetricsServiceBlockingStub getBuildMetricsStub() {
        throwIfNoop();
        return buildMetricsStub;
    }

    public GarbageCollectionServiceGrpc.GarbageCollectionServiceBlockingStub getGarbageCollectionStub() {
        throwIfNoop();
        return garbageCollectionStub;
    }

    public ParserServiceGrpc.ParserServiceBlockingStub getParserStub() {
        throwIfNoop();
        return parserStub;
    }

    private void throwIfNoop() {
        if (noop) {
            throw new SubstrateException("Substrate client is in no-op mode");
        }
    }

    private void performHandshake() throws IOException {
        throwIfNoop();
        try {
            HandshakeRequest.Builder request = HandshakeRequest.newBuilder()
                .setClientVersion(resolveClientVersion())
                .setProtocolVersion(CLIENT_PROTOCOL_VERSION)
                .setClientPid(currentPid());
            if (jvmHostSocketPath != null) {
                request.setJvmHostSocketPath(jvmHostSocketPath);
            }
            HandshakeResponse response = controlStub.handshake(request.build());
            if (!response.getAccepted()) {
                throw new IOException(
                    "Substrate handshake rejected: " + response.getErrorMessage()
                        + " (clientProtocol=" + CLIENT_PROTOCOL_VERSION
                        + ", serverProtocol=" + response.getProtocolVersion() + ")"
                );
            }
        } catch (RuntimeException e) {
            throw new IOException("Failed to handshake with substrate daemon", e);
        }
    }

    private static String resolveClientVersion() {
        Package pkg = SubstrateClient.class.getPackage();
        if (pkg != null && pkg.getImplementationVersion() != null) {
            return pkg.getImplementationVersion();
        }
        return "gradle-rust-bridge";
    }

    private static int currentPid() {
        String runtimeName = ManagementFactory.getRuntimeMXBean().getName();
        if (runtimeName != null) {
            int at = runtimeName.indexOf('@');
            if (at > 0) {
                try {
                    return Integer.parseInt(runtimeName.substring(0, at));
                } catch (NumberFormatException ignored) {
                    // Fall through.
                }
            }
        }
        return -1;
    }

    @Override
    public void close() {
        if (channel != null) {
            channel.shutdown();
            try {
                if (!channel.awaitTermination(5, TimeUnit.SECONDS)) {
                    channel.shutdownNow();
                }
            } catch (InterruptedException e) {
                channel.shutdownNow();
                Thread.currentThread().interrupt();
            }
        }
    }
}
