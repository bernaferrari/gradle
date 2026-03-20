package org.gradle.internal.rustbridge;

import io.grpc.ManagedChannel;
import io.grpc.netty.shaded.io.grpc.netty.NettyChannelBuilder;
import io.grpc.netty.shaded.io.netty.channel.unix.DomainSocketAddress;
import gradle.substrate.v1.CacheServiceGrpc;
import gradle.substrate.v1.ControlServiceGrpc;
import gradle.substrate.v1.ExecServiceGrpc;
import gradle.substrate.v1.HashServiceGrpc;
import gradle.substrate.v1.WorkServiceGrpc;

import java.io.Closeable;
import java.io.IOException;
import java.util.concurrent.TimeUnit;

/**
 * gRPC client for communicating with the Rust substrate daemon.
 * Connects via Unix domain socket.
 */
public class SubstrateClient implements Closeable {

    private final ManagedChannel channel;
    private final ControlServiceGrpc.ControlServiceBlockingStub controlStub;
    private final HashServiceGrpc.HashServiceBlockingStub hashStub;
    private final CacheServiceGrpc.CacheServiceBlockingStub cacheStub;
    private final ExecServiceGrpc.ExecServiceBlockingStub execStub;
    private final WorkServiceGrpc.WorkServiceBlockingStub workStub;
    private final boolean noop;

    private SubstrateClient(ManagedChannel channel, boolean noop) {
        this.channel = channel;
        this.noop = noop;
        if (noop) {
            this.controlStub = null;
            this.hashStub = null;
            this.cacheStub = null;
            this.execStub = null;
            this.workStub = null;
        } else {
            this.controlStub = ControlServiceGrpc.newBlockingStub(channel);
            this.hashStub = HashServiceGrpc.newBlockingStub(channel);
            this.cacheStub = CacheServiceGrpc.newBlockingStub(channel);
            this.execStub = ExecServiceGrpc.newBlockingStub(channel);
            this.workStub = WorkServiceGrpc.newBlockingStub(channel);
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
