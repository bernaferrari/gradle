package org.gradle.internal.rustbridge;

import io.grpc.ManagedChannel;
import io.grpc.netty.shaded.io.grpc.netty.NettyChannelBuilder;
import io.grpc.netty.shaded.io.netty.channel.unix.DomainSocketAddress;
import gradle.substrate.v1.*;

import java.io.Closeable;
import java.io.IOException;
import java.util.concurrent.TimeUnit;

/**
 * gRPC client for communicating with the Rust substrate daemon.
 * Connects via Unix domain socket.
 *
 * <p>Provides blocking stubs for all 20 substrate services.
 * When the substrate is disabled (noop mode), all stub getters
 * throw {@link SubstrateException}.</p>
 */
public class SubstrateClient implements Closeable {

    private final ManagedChannel channel;
    private final boolean noop;

    // Phase 0: Control
    private final ControlServiceGrpc.ControlServiceBlockingStub controlStub;
    // Phase 1: Hashing
    private final HashServiceGrpc.HashServiceBlockingStub hashStub;
    // Phase 2: Build cache
    private final CacheServiceGrpc.CacheServiceBlockingStub cacheStub;
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

    private SubstrateClient(ManagedChannel channel, boolean noop) {
        this.channel = channel;
        this.noop = noop;
        if (noop) {
            this.controlStub = null;
            this.hashStub = null;
            this.cacheStub = null;
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
        } else {
            this.controlStub = ControlServiceGrpc.newBlockingStub(channel);
            this.hashStub = HashServiceGrpc.newBlockingStub(channel);
            this.cacheStub = CacheServiceGrpc.newBlockingStub(channel);
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
        }
    }

    /**
     * Creates a SubstrateClient connected to the given Unix socket path.
     */
    public static SubstrateClient connect(String socketPath) throws IOException {
        ManagedChannel channel = NettyChannelBuilder
            .forAddress(new DomainSocketAddress(socketPath))
            .usePlaintext()
            .build();
        return new SubstrateClient(channel, false);
    }

    /**
     * Creates a no-op client that does nothing. Used when the substrate is disabled.
     */
    public static SubstrateClient noop() {
        return new SubstrateClient(null, true);
    }

    public boolean isNoop() {
        return noop;
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

    private void throwIfNoop() {
        if (noop) {
            throw new SubstrateException("Substrate client is in no-op mode");
        }
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
