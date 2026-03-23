package org.gradle.internal.rustbridge.jvmhost;

import org.gradle.api.logging.Logging;
import org.slf4j.Logger;

import java.io.Closeable;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import gradle.substrate.v1.*;
import io.grpc.Server;
import io.grpc.stub.StreamObserver;

/**
 * gRPC server running inside the Gradle JVM daemon that accepts callbacks
 * from the Rust substrate daemon.
 *
 * <p>Listens on a Unix domain socket (e.g., {@code jvm-host.sock}) and serves
 * the {@code JvmHostService} defined in {@code substrate.proto}.</p>
 *
 * <p>Only {@code GetBuildEnvironment} is fully implemented; other RPCs return
 * UNIMPLEMENTED status until Phase 6 is complete.</p>
 */
public class JvmHostServer implements Closeable {

    private static final Logger LOGGER = Logging.getLogger(JvmHostServer.class);

    private final Server server;
    private final String socketPath;
    private final JvmHostServiceImpl serviceImpl;

    public JvmHostServer(String socketPath, JvmHostServiceImpl serviceImpl) throws IOException {
        this.socketPath = socketPath;
        this.serviceImpl = serviceImpl;

        // Clean up stale socket if present
        Path socketFile = Path.of(socketPath);
        if (Files.exists(socketFile)) {
            Files.delete(socketFile);
        }

        // Create parent directory
        Path parent = socketFile.getParent();
        if (parent != null) {
            Files.createDirectories(parent);
        }

        this.server = io.grpc.netty.shaded.io.grpc.netty.NettyServerBuilder
            .forAddress(new io.grpc.netty.shaded.io.netty.channel.unix.DomainSocketAddress(socketPath))
            .channelType(io.grpc.netty.shaded.io.netty.channel.unix.DomainServerSocketChannel.class)
            .addService(new JvmHostServiceGrpc.JvmHostServiceImplBase() {
                @Override
                public void evaluateScript(
                    EvaluateScriptRequest request,
                    StreamObserver<EvaluateScriptResponse> responseObserver) {
                    LOGGER.debug("[substrate-jvmhost] evaluateScript called (UNIMPLEMENTED)");
                    responseObserver.onError(
                        io.grpc.Status.UNIMPLEMENTED
                            .withDescription("Script evaluation not yet implemented")
                            .asRuntimeException());
                }

                @Override
                public void getBuildModel(
                    GetBuildModelRequest request,
                    StreamObserver<GetBuildModelResponse> responseObserver) {
                    LOGGER.debug("[substrate-jvmhost] getBuildModel called (UNIMPLEMENTED)");
                    responseObserver.onError(
                        io.grpc.Status.UNIMPLEMENTED
                            .withDescription("Build model access not yet implemented")
                            .asRuntimeException());
                }

                @Override
                public void resolveConfiguration(
                    ResolveConfigRequest request,
                    StreamObserver<ResolveConfigResponse> responseObserver) {
                    LOGGER.debug("[substrate-jvmhost] resolveConfiguration called (UNIMPLEMENTED)");
                    responseObserver.onError(
                        io.grpc.Status.UNIMPLEMENTED
                            .withDescription("Configuration resolution not yet implemented")
                            .asRuntimeException());
                }

                @Override
                public void getBuildEnvironment(
                    GetBuildEnvironmentRequest request,
                    StreamObserver<GetBuildEnvironmentResponse> responseObserver) {
                    GetBuildEnvironmentResponse response =
                        GetBuildEnvironmentResponse.newBuilder()
                            .setJavaVersion(serviceImpl.getJavaVersion())
                            .setJavaHome(serviceImpl.getJavaHome())
                            .setGradleVersion(serviceImpl.getGradleVersion())
                            .setOsName(serviceImpl.getOsName())
                            .setOsArch(serviceImpl.getOsArch())
                            .setAvailableProcessors(serviceImpl.getAvailableProcessors())
                            .setMaxMemoryBytes(serviceImpl.getMaxMemoryBytes())
                            .putSystemProperties("java.vm.name", System.getProperty("java.vm.name", ""))
                            .putSystemProperties("java.vendor", System.getProperty("java.vendor", ""))
                            .putSystemProperties("file.encoding", System.getProperty("file.encoding", ""))
                            .putSystemProperties("user.language", System.getProperty("user.language", ""))
                            .putSystemProperties("user.country", System.getProperty("user.country", ""))
                            .putSystemProperties("user.timezone", System.getProperty("user.timezone", ""))
                            .build();
                    responseObserver.onNext(response);
                    responseObserver.onCompleted();
                }
            })
            .build();
    }

    /**
     * Start the JVM host server.
     */
    public void start() throws IOException {
        server.start();
        LOGGER.lifecycle("[substrate] JVM host server started on {}", socketPath);
    }

    /**
     * Get the socket path this server is listening on.
     */
    public String getSocketPath() {
        return socketPath;
    }

    @Override
    public void close() {
        if (server != null) {
            server.shutdown();
            try {
                if (!server.awaitTermination(5, java.util.concurrent.TimeUnit.SECONDS)) {
                    server.shutdownNow();
                }
            } catch (InterruptedException e) {
                server.shutdownNow();
                Thread.currentThread().interrupt();
            }
            // Clean up socket file
            try {
                Files.deleteIfExists(Path.of(socketPath));
            } catch (IOException ignored) {
            }
            LOGGER.lifecycle("[substrate] JVM host server stopped");
        }
    }
}
