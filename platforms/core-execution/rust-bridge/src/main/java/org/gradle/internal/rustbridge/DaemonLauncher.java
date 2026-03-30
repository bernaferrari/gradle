package org.gradle.internal.rustbridge;

import org.gradle.api.logging.Logging;
import org.slf4j.Logger;
import org.gradle.internal.rustbridge.jvmhost.JvmHostServer;
import org.gradle.internal.rustbridge.jvmhost.JvmHostServiceImpl;

import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import org.jspecify.annotations.Nullable;

/**
 * Manages the lifecycle of the Rust substrate daemon process.
 * Optionally starts a JVM Compatibility Host server for reverse-direction RPC.
 */
public class DaemonLauncher {

    private static final Logger LOGGER = Logging.getLogger(DaemonLauncher.class);
    private static final String SOCKET_NAME = "substrate.sock";
    private static final String JVM_HOST_SOCKET_NAME = "jvm-host.sock";
    private static final String SUBSTRATE_DIR_NAME = ".gradle-substrate";
    private static final String BINARY_NAME = "gradle-substrate-daemon";

    private final File daemonBinary;
    private final File socketDirectory;
    private final boolean noop;
    private final boolean enableJvmHost;
    private Process daemonProcess;
    private JvmHostServer jvmHostServer;

    private DaemonLauncher(File daemonBinary, File socketDirectory, boolean noop, boolean enableJvmHost) {
        this.daemonBinary = daemonBinary;
        this.socketDirectory = socketDirectory;
        this.noop = noop;
        this.enableJvmHost = enableJvmHost;
    }

    public static DaemonLauncher noop() {
        return new DaemonLauncher(null, null, true, false);
    }

    public static DaemonLauncher of(File daemonBinary, File socketDirectory) {
        return new DaemonLauncher(daemonBinary, socketDirectory, false, false);
    }

    public static DaemonLauncher withJvmHost(File daemonBinary, File socketDirectory) {
        return new DaemonLauncher(daemonBinary, socketDirectory, false, true);
    }

    /**
     * Resolve the substrate daemon binary for the current platform.
     * Tries platform-specific path first, then generic fallback.
     */
    public static File resolveBinary(File installDir) {
        String platform = detectPlatform();
        File platformSpecific = new File(installDir, "lib/substrate/" + BINARY_NAME + "-" + platform);
        if (platformSpecific.exists()) {
            return platformSpecific;
        }
        // Windows uses .exe extension
        if (platform.contains("windows")) {
            File withExe = new File(installDir, "lib/substrate/" + BINARY_NAME + "-" + platform + ".exe");
            if (withExe.exists()) {
                return withExe;
            }
        }
        // Fallback to generic path
        return new File(installDir, "lib/" + BINARY_NAME);
    }

    private static String detectPlatform() {
        String osName = System.getProperty("os.name", "").toLowerCase();
        String osArch = System.getProperty("os.arch", "").toLowerCase();

        String os;
        if (osName.contains("win")) {
            os = "windows";
        } else if (osName.contains("mac")) {
            os = "macos";
        } else {
            os = "linux";
        }

        String arch;
        if (osArch.contains("aarch64") || osArch.contains("arm64")) {
            arch = "aarch64";
        } else {
            arch = "x86_64";
        }

        return os + "-" + arch;
    }

    public String getSocketPath() {
        return new File(socketDirectory, SOCKET_NAME).getAbsolutePath();
    }

    /**
     * Get the JVM host service implementation, or null if JVM host is not enabled.
     * Used by Build-scoped services to set the ProjectModelProvider.
     */
    @Nullable
    public JvmHostServiceImpl getJvmHostServiceImpl() {
        return jvmHostServer != null ? jvmHostServer.getServiceImpl() : null;
    }

    /**
     * Get the JVM host socket path, or null if JVM host is not enabled.
     */
    @Nullable
    public String getJvmHostSocketPath() {
        if (jvmHostServer != null) {
            return jvmHostServer.getSocketPath();
        }
        return null;
    }

    /**
     * Launches the daemon if not already running, then connects to it.
     */
    public SubstrateClient launchOrConnect() throws IOException {
        if (noop) {
            return SubstrateClient.noop();
        }

        Path socketFile = new File(socketDirectory, SOCKET_NAME).toPath();

        String jvmHostSocketPath = null;

        // Check if daemon is already running by testing the socket.
        if (Files.exists(socketFile)) {
            LOGGER.info("[substrate] Connecting to existing daemon at {}", socketFile);
            jvmHostSocketPath = startJvmHostIfEnabled();
            try {
                return SubstrateClient.connect(socketFile.toString(), jvmHostSocketPath);
            } catch (IOException connectFailure) {
                // Stale socket or incompatible daemon; clean up and launch a fresh one.
                LOGGER.warn("[substrate] Failed to connect to existing daemon, relaunching: {}", connectFailure.getMessage());
                Files.deleteIfExists(socketFile);
                if (jvmHostServer != null) {
                    jvmHostServer.close();
                    jvmHostServer = null;
                    jvmHostSocketPath = null;
                }
            }
        }

        // Ensure socket directory exists
        Files.createDirectories(socketDirectory.toPath());

        if (!daemonBinary.exists()) {
            LOGGER.warn("[substrate] Daemon binary not found at {}, using no-op mode", daemonBinary);
            return SubstrateClient.noop();
        }

        // Phase 6: Start JVM Compatibility Host before launching the Rust daemon.
        jvmHostSocketPath = startJvmHostIfEnabled();

        LOGGER.info("[substrate] Launching daemon from {}", daemonBinary);

        File stateRoot = new File(socketDirectory, "state");
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
            "--socket-path", socketFile.toString(),
            "--log-level", "info",
            "--cache-dir", cacheDir.getAbsolutePath(),
            "--history-dir", historyDir.getAbsolutePath(),
            "--config-cache-dir", configCacheDir.getAbsolutePath(),
            "--toolchain-dir", toolchainDir.getAbsolutePath(),
            "--artifact-store-dir", artifactStoreDir.getAbsolutePath()
        );
        pb.environment().put("SUBSTRATE_LOG_LEVEL", "info");
        pb.redirectErrorStream(true);
        pb.directory(socketDirectory);

        daemonProcess = pb.start();

        // Consume stdout/stderr to prevent buffer deadlock
        consumeStream(daemonProcess);

        // Wait briefly for the socket to appear
        int attempts = 0;
        while (!Files.exists(socketFile) && attempts < 50) {
            try {
                Thread.sleep(100);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new SubstrateException("Interrupted while waiting for daemon to start", e);
            }
            attempts++;
        }

        if (!Files.exists(socketFile)) {
            throw new SubstrateException("Daemon failed to start: socket not created after 5 seconds");
        }

        LOGGER.info("[substrate] Daemon started successfully");
        return SubstrateClient.connect(socketFile.toString(), jvmHostSocketPath);
    }

    @Nullable
    private String startJvmHostIfEnabled() {
        if (!enableJvmHost) {
            return null;
        }
        if (jvmHostServer != null) {
            return jvmHostServer.getSocketPath();
        }
        String jvmHostSocketPath = new File(socketDirectory, JVM_HOST_SOCKET_NAME).getAbsolutePath();
        try {
            jvmHostServer = new JvmHostServer(jvmHostSocketPath, new JvmHostServiceImpl());
            jvmHostServer.start();
            return jvmHostSocketPath;
        } catch (IOException e) {
            LOGGER.warn("[substrate] Failed to start JVM host server, continuing without: {}", e.getMessage());
            jvmHostServer = null;
            return null;
        }
    }

    private void consumeStream(Process process) {
        Thread consumer = new Thread(() -> {
            try {
                byte[] buffer = new byte[1024];
                java.io.InputStream input = process.getInputStream();
                int n;
                while ((n = input.read(buffer)) != -1) {
                    // Log daemon output at debug level
                    String output = new String(buffer, 0, n);
                    LOGGER.debug("[substrate] {}", output.trim());
                }
            } catch (IOException e) {
                // Process ended
            }
        }, "substrate-daemon-stdout");
        consumer.setDaemon(true);
        consumer.start();
    }

    public void shutdownDaemon() {
        // Shut down JVM host server first
        if (jvmHostServer != null) {
            jvmHostServer.close();
            jvmHostServer = null;
        }
        if (daemonProcess != null && isProcessAlive(daemonProcess)) {
            daemonProcess.destroy();
            try {
                long deadline = System.currentTimeMillis() + 5000;
                while (System.currentTimeMillis() < deadline) {
                    try {
                        daemonProcess.exitValue();
                        break; // process has terminated
                    } catch (IllegalThreadStateException e) {
                        // still alive
                    }
                    Thread.sleep(100);
                }
                // If still alive after timeout, try forceful kill
                if (isProcessAlive(daemonProcess)) {
                    forceKill(daemonProcess);
                }
            } catch (InterruptedException e) {
                forceKill(daemonProcess);
                Thread.currentThread().interrupt();
            }
            daemonProcess = null;
        }
    }

    /**
     * Check if a process is still alive. Compatible with Java 8.
     */
    private static boolean isProcessAlive(Process process) {
        try {
            process.exitValue();
            return false;
        } catch (IllegalThreadStateException e) {
            return true;
        }
    }

    /**
     * Forcefully kill a process. Compatible with Java 8.
     * On Unix systems, sends SIGKILL. On other systems, falls back to destroy().
     */
    private static void forceKill(Process process) {
        try {
            // Unix-only: use kill -9 to force kill the process
            String osName = System.getProperty("os.name", "").toLowerCase();
            if (osName.contains("nix") || osName.contains("nux") || osName.contains("mac")) {
                Runtime.getRuntime().exec(new String[]{"kill", "-9", String.valueOf(getProcessId(process))});
            } else {
                process.destroy();
            }
        } catch (Exception e) {
            process.destroy();
        }
    }

    /**
     * Get the native process ID. Uses reflection to access the private 'pid' field,
     * which exists in both Oracle and OpenJDK implementations since Java 8.
     * Falls back to 0 if reflection fails.
     */
    private static long getProcessId(Process process) {
        try {
            java.lang.reflect.Field pidField = process.getClass().getDeclaredField("pid");
            pidField.setAccessible(true);
            return pidField.getLong(process);
        } catch (Exception e) {
            return 0;
        }
    }
}
