package org.gradle.internal.rustbridge;

import org.gradle.api.logging.Logging;
import org.gradle.internal.util.Either;
import org.slf4j.Logger;
import org.gradle.internal.rustbridge.jvmhost.JvmHostServer;
import org.gradle.internal.rustbridge.jvmhost.JvmHostServiceImpl;

import java.io.File;
import java.io.IOException;
import java.io.OutputStream;
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

        // Check if daemon is already running by testing the socket
        if (Files.exists(socketFile)) {
            LOGGER.lifecycle("[substrate] Connecting to existing daemon at {}", socketFile);
            return SubstrateClient.connect(socketFile.toString());
        }

        // Ensure socket directory exists
        Files.createDirectories(socketDirectory.toPath());

        if (!daemonBinary.exists()) {
            LOGGER.warn("[substrate] Daemon binary not found at {}, using no-op mode", daemonBinary);
            return SubstrateClient.noop();
        }

        // Phase 6: Start JVM Compatibility Host before launching the Rust daemon
        String jvmHostSocketPath = null;
        if (enableJvmHost) {
            jvmHostSocketPath = new File(socketDirectory, JVM_HOST_SOCKET_NAME).getAbsolutePath();
            try {
                jvmHostServer = new JvmHostServer(jvmHostSocketPath, new JvmHostServiceImpl());
                jvmHostServer.start();
            } catch (IOException e) {
                LOGGER.warn("[substrate] Failed to start JVM host server, continuing without: {}", e.getMessage());
                jvmHostServer = null;
                jvmHostSocketPath = null;
            }
        }

        LOGGER.lifecycle("[substrate] Launching daemon from {}", daemonBinary);

        ProcessBuilder pb = new ProcessBuilder(
            daemonBinary.getAbsolutePath(),
            "--socket-path", socketFile.toString(),
            "--log-level", "info"
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

        LOGGER.lifecycle("[substrate] Daemon started successfully");
        return SubstrateClient.connect(socketFile.toString(), jvmHostSocketPath);
    }

    private void consumeStream(Process process) {
        Thread consumer = new Thread(() -> {
            try {
                byte[] buffer = new byte[1024];
                var input = process.getInputStream();
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
        if (daemonProcess != null && daemonProcess.isAlive()) {
            daemonProcess.destroy();
            try {
                if (!daemonProcess.waitFor(5, java.util.concurrent.TimeUnit.SECONDS)) {
                    daemonProcess.destroyForcibly();
                }
            } catch (InterruptedException e) {
                daemonProcess.destroyForcibly();
                Thread.currentThread().interrupt();
            }
            daemonProcess = null;
        }
    }
}
