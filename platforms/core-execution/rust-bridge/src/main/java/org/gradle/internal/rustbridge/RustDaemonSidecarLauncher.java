package org.gradle.internal.rustbridge;

import org.gradle.api.logging.Logging;
import org.gradle.internal.buildoption.InternalOptions;
import org.gradle.internal.buildoption.RustSubstrateOptions;
import org.slf4j.Logger;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Minimal launcher/connector for the Rust daemon sidecar.
 *
 * <p>KISS behavior:
 * 1) connect to existing socket if available;
 * 2) otherwise launch daemon binary (if available);
 * 3) fail open to {@link SubstrateClient#noop()}.</p>
 */
public final class RustDaemonSidecarLauncher {

    private static final Logger LOGGER = Logging.getLogger(RustDaemonSidecarLauncher.class);
    private static final Object LAUNCH_LOCK = new Object();

    private static final String DEFAULT_SOCKET_RELATIVE_PATH = ".gradle-substrate/substrate.sock";
    private static final String SOCKET_PATH_PROPERTY = "org.gradle.rust.substrate.socket.path";

    private RustDaemonSidecarLauncher() {
    }

    public static SubstrateClient connectOrLaunch(InternalOptions options) {
        if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
            return SubstrateClient.noop();
        }

        String socketPath = resolveSocketPath();
        Path socket = new File(socketPath).toPath();
        if (Files.exists(socket)) {
            try {
                return SubstrateClient.connect(socketPath);
            } catch (Exception e) {
                LOGGER.debug("[substrate] existing socket connect failed: {}", e.getMessage());
                try {
                    Files.deleteIfExists(socket);
                } catch (IOException ignored) {
                    // ignore stale socket cleanup failure
                }
            }
        }

        File daemonBinary = resolveDaemonBinary(options);
        if (daemonBinary == null || !daemonBinary.exists()) {
            LOGGER.debug("[substrate] daemon binary not found, using no-op client");
            return SubstrateClient.noop();
        }

        synchronized (LAUNCH_LOCK) {
            if (!Files.exists(socket)) {
                try {
                    launchDaemon(daemonBinary, socketPath);
                } catch (Exception e) {
                    LOGGER.debug("[substrate] daemon launch failed: {}", e.getMessage(), e);
                    return SubstrateClient.noop();
                }
            }
        }

        if (!Files.exists(socket)) {
            return SubstrateClient.noop();
        }
        try {
            return SubstrateClient.connect(socketPath);
        } catch (Exception e) {
            LOGGER.debug("[substrate] post-launch connect failed: {}", e.getMessage(), e);
            return SubstrateClient.noop();
        }
    }

    private static String resolveSocketPath() {
        String override = System.getProperty(SOCKET_PATH_PROPERTY, "").trim();
        if (!override.isEmpty()) {
            return override;
        }
        return new File(System.getProperty("user.home"), DEFAULT_SOCKET_RELATIVE_PATH).getAbsolutePath();
    }

    private static File resolveDaemonBinary(InternalOptions options) {
        String configured = options.getOption(RustSubstrateOptions.DAEMON_BINARY_PATH).get().trim();
        if (!configured.isEmpty()) {
            return new File(configured);
        }
        String javaHome = System.getProperty("java.home");
        File installDir = new File(javaHome).getParentFile();
        if (installDir == null) {
            return null;
        }
        return new File(installDir, "lib/gradle-substrate-daemon");
    }

    private static void launchDaemon(File daemonBinary, String socketPath) throws IOException, InterruptedException {
        File socketFile = new File(socketPath);
        File socketDir = socketFile.getParentFile();
        if (socketDir == null) {
            throw new IOException("Invalid substrate socket path: " + socketPath);
        }
        Files.createDirectories(socketDir.toPath());

        File stateRoot = new File(socketDir, "state");
        File cacheDir = new File(stateRoot, "cache");
        File historyDir = new File(stateRoot, "history");
        File configCacheDir = new File(stateRoot, "config-cache");
        File toolchainDir = new File(stateRoot, "toolchains");
        File artifactStoreDir = new File(stateRoot, "artifacts");
        Files.createDirectories(cacheDir.toPath());
        Files.createDirectories(historyDir.toPath());
        Files.createDirectories(configCacheDir.toPath());
        Files.createDirectories(toolchainDir.toPath());
        Files.createDirectories(artifactStoreDir.toPath());

        ProcessBuilder pb = new ProcessBuilder(
            daemonBinary.getAbsolutePath(),
            "--socket-path", socketPath,
            "--log-level", "info",
            "--cache-dir", cacheDir.getAbsolutePath(),
            "--history-dir", historyDir.getAbsolutePath(),
            "--config-cache-dir", configCacheDir.getAbsolutePath(),
            "--toolchain-dir", toolchainDir.getAbsolutePath(),
            "--artifact-store-dir", artifactStoreDir.getAbsolutePath()
        );
        pb.redirectErrorStream(true);
        pb.directory(socketDir);

        Process daemonProcess = pb.start();
        Thread streamConsumer = new Thread(() -> consumeStdout(daemonProcess), "substrate-daemon-stdout");
        streamConsumer.setDaemon(true);
        streamConsumer.start();

        Path socket = socketFile.toPath();
        int attempts = 0;
        while (!Files.exists(socket) && attempts < 50) {
            Thread.sleep(100);
            attempts++;
        }
    }

    private static void consumeStdout(Process process) {
        try {
            byte[] buffer = new byte[1024];
            int n;
            while ((n = process.getInputStream().read(buffer)) != -1) {
                LOGGER.debug("[substrate] {}", new String(buffer, 0, n, StandardCharsets.UTF_8).trim());
            }
        } catch (IOException ignored) {
            // process stream closed
        }
    }
}
