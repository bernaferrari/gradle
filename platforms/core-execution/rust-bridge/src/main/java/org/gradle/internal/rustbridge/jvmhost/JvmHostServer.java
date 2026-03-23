package org.gradle.internal.rustbridge.jvmhost;

import org.gradle.api.logging.Logging;
import org.slf4j.Logger;

import java.io.Closeable;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

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
 * <p>{@code GetBuildEnvironment}, {@code GetBuildModel}, and {@code ResolveConfiguration}
 * are fully implemented. {@code EvaluateScript} parses build script content to extract
 * applied plugins without full script evaluation.</p>
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
                    try {
                        String scriptPath = request.getScriptPath();
                        String scriptContent = request.getScriptContent();
                        String scriptType = request.getScriptType();

                        LOGGER.debug("[substrate-jvmhost] evaluateScript called for {} (type={})",
                            scriptPath.isEmpty() ? "<inline>" : scriptPath, scriptType);

                        // Read script from file if content is empty and path is provided
                        String content = scriptContent;
                        if (content.isEmpty() && !scriptPath.isEmpty()) {
                            Path p = Path.of(scriptPath);
                            if (Files.exists(p)) {
                                content = Files.readString(p);
                            } else {
                                responseObserver.onNext(EvaluateScriptResponse.newBuilder()
                                    .setSuccess(false)
                                    .setErrorMessage("Script file not found: " + scriptPath)
                                    .build());
                                responseObserver.onCompleted();
                                return;
                            }
                        }

                        // Extract applied plugins from the script content
                        List<AppliedPlugin> plugins = extractPlugins(content);

                        EvaluateScriptResponse response = EvaluateScriptResponse.newBuilder()
                            .setSuccess(true)
                            .addAllAppliedPlugins(plugins)
                            .build();
                        responseObserver.onNext(response);
                        responseObserver.onCompleted();
                    } catch (Exception e) {
                        LOGGER.error("[substrate-jvmhost] evaluateScript failed", e);
                        responseObserver.onError(
                            io.grpc.Status.INTERNAL
                                .withDescription("Script evaluation failed: " + e.getMessage())
                                .asRuntimeException());
                    }
                }

                @Override
                public void getBuildModel(
                    GetBuildModelRequest request,
                    StreamObserver<GetBuildModelResponse> responseObserver) {
                    try {
                        LOGGER.debug("[substrate-jvmhost] getBuildModel called for build {}", request.getBuildId());
                        GetBuildModelResponse.Builder responseBuilder = GetBuildModelResponse.newBuilder();
                        for (JvmHostServiceImpl.ProjectModelEntry entry : serviceImpl.getProjectModels()) {
                            ProjectModel.Builder pm = ProjectModel.newBuilder()
                                .setPath(entry.getPath())
                                .setName(entry.getName())
                                .setBuildFile(entry.getBuildFile());
                            pm.addAllSubprojects(entry.getSubprojects());
                            responseBuilder.addProjects(pm);
                        }
                        responseObserver.onNext(responseBuilder.build());
                        responseObserver.onCompleted();
                    } catch (Exception e) {
                        LOGGER.error("[substrate-jvmhost] getBuildModel failed", e);
                        responseObserver.onError(
                            io.grpc.Status.INTERNAL
                                .withDescription("Failed to retrieve build model: " + e.getMessage())
                                .asRuntimeException());
                    }
                }

                @Override
                public void resolveConfiguration(
                    ResolveConfigRequest request,
                    StreamObserver<ResolveConfigResponse> responseObserver) {
                    try {
                        LOGGER.debug("[substrate-jvmhost] resolveConfiguration called for project {} config {}",
                            request.getProjectPath(), request.getConfigurationName());
                        ResolveConfigResponse.Builder responseBuilder = ResolveConfigResponse.newBuilder()
                            .setSuccess(true);
                        for (JvmHostServiceImpl.ResolvedArtifactEntry entry :
                                serviceImpl.resolveArtifacts(
                                    request.getProjectPath(),
                                    request.getConfigurationName())) {
                            ResolvedArtifact artifact = ResolvedArtifact.newBuilder()
                                .setGroup(entry.getGroup())
                                .setName(entry.getName())
                                .setVersion(entry.getVersion())
                                .setConfiguration(entry.getConfiguration())
                                .build();
                            responseBuilder.addArtifacts(artifact);
                        }
                        responseObserver.onNext(responseBuilder.build());
                        responseObserver.onCompleted();
                    } catch (Exception e) {
                        LOGGER.error("[substrate-jvmhost] resolveConfiguration failed", e);
                        responseObserver.onNext(ResolveConfigResponse.newBuilder()
                            .setSuccess(false)
                            .setErrorMessage("Configuration resolution failed: " + e.getMessage())
                            .build());
                        responseObserver.onCompleted();
                    }
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

    /**
     * Get the service implementation backing this server.
     * Used to set the ProjectModelProvider after construction.
     */
    public JvmHostServiceImpl getServiceImpl() {
        return serviceImpl;
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

    // --- Plugin extraction ---

    /**
     * Extract applied plugins from build script content.
     * Handles both Kotlin DSL ({@code plugins { id("foo") }}) and
     * Groovy DSL ({@code apply plugin: "foo"} / {@code id "foo"}).
     *
     * @return list of applied plugins in declaration order
     */
    static List<AppliedPlugin> extractPlugins(String scriptContent) {
        List<AppliedPlugin> plugins = new ArrayList<>();

        // Kotlin DSL: plugins { id("plugin-id") ... } and plugins { id("plugin-id") version "1.0" ... }
        Pattern kotlinIdPattern = Pattern.compile(
            "id\\s*\\(\\s*\"([^\"]+)\"\\s*\\)");
        Matcher kotlinMatcher = kotlinIdPattern.matcher(scriptContent);

        // Groovy DSL: plugins { id "plugin-id" ... } and plugins { id "plugin-id" version "1.0" }
        Pattern groovyIdPattern = Pattern.compile(
            "id\\s+['\"]([^'\"]+)['\"]");
        Matcher groovyMatcher = groovyIdPattern.matcher(scriptContent);

        // Legacy Groovy: apply plugin: "plugin-id"
        Pattern legacyPattern = Pattern.compile(
            "apply\\s+plugin\\s*:\\s*['\"]([^'\"]+)['\"]");
        Matcher legacyMatcher = legacyPattern.matcher(scriptContent);

        // Also handle Kotlin DSL: kotlin("jvm"), kotlin("js"), etc.
        Pattern kotlinPluginPattern = Pattern.compile(
            "kotlin\\s*\\(\\s*\"([^\"]+)\"\\s*\\)");
        Matcher kotlinPluginMatcher = kotlinPluginPattern.matcher(scriptContent);

        // Collect with positions to maintain declaration order
        List<PluginMatch> matches = new ArrayList<>();

        while (kotlinMatcher.find()) {
            matches.add(new PluginMatch(kotlinMatcher.start(), kotlinMatcher.group(1)));
        }
        while (groovyMatcher.find()) {
            matches.add(new PluginMatch(groovyMatcher.start(), groovyMatcher.group(1)));
        }
        while (legacyMatcher.find()) {
            matches.add(new PluginMatch(legacyMatcher.start(), legacyMatcher.group(1)));
        }
        while (kotlinPluginMatcher.find()) {
            // Convert kotlin("jvm") to org.jetbrains.kotlin.jvm
            String kotlinPlugin = kotlinPluginMatcher.group(1);
            String resolvedId = "org.jetbrains.kotlin." + kotlinPlugin;
            matches.add(new PluginMatch(kotlinPluginMatcher.start(), resolvedId));
        }

        // Sort by position to maintain declaration order
        matches.sort((a, b) -> Integer.compare(a.position, b.position));

        // Deduplicate (same plugin_id may appear in both patterns)
        java.util.Set<String> seen = new java.util.HashSet<>();
        for (PluginMatch match : matches) {
            if (seen.add(match.pluginId)) {
                plugins.add(AppliedPlugin.newBuilder()
                    .setPluginId(match.pluginId)
                    .setApplyOrder(String.valueOf(seen.size()))
                    .build());
            }
        }

        return plugins;
    }

    private static class PluginMatch {
        final int position;
        final String pluginId;

        PluginMatch(int position, String pluginId) {
            this.position = position;
            this.pluginId = pluginId;
        }
    }
}
