package org.gradle.internal.rustbridge;

import org.gradle.internal.buildoption.DefaultInternalOptions;
import org.gradle.internal.buildoption.RustSubstrateOptions;
import org.junit.Test;

import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

public class SubstrateLifecycleTest {
    @Test
    public void noopClientCarriesReasonIntoStubFailure() {
        SubstrateClient client = SubstrateClient.noop("daemon-binary-missing:/tmp/nope");

        assertTrue(client.isNoop());
        assertEquals("daemon-binary-missing:/tmp/nope", client.getNoopReason());
        try {
            client.getControlStub();
            fail("Expected no-op client to reject stub access");
        } catch (SubstrateException e) {
            assertTrue(e.getMessage().contains("daemon-binary-missing:/tmp/nope"));
        }
    }

    @Test
    public void disabledDaemonLauncherProducesExplicitNoopReason() throws Exception {
        SubstrateClient client = DaemonLauncher.noop("explicit-disablement").launchOrConnect();

        assertTrue(client.isNoop());
        assertEquals("explicit-disablement", client.getNoopReason());
    }

    @Test
    public void missingDaemonBinaryIsVisibleInShadowMode() throws Exception {
        Path tempDir = Files.createTempDirectory("substrate-lifecycle-");
        String previousSocketPath = System.getProperty("org.gradle.rust.substrate.socket.path");
        System.setProperty("org.gradle.rust.substrate.socket.path", tempDir.resolve("substrate.sock").toString());
        try {
            File missingBinary = tempDir.resolve("missing-daemon").toFile();
            SubstrateClient client = RustDaemonSidecarLauncher.connectOrLaunch(options(true, false, missingBinary));

            assertTrue(client.isNoop());
            assertEquals("daemon-binary-missing:" + missingBinary.getAbsolutePath(), client.getNoopReason());
        } finally {
            restoreProperty("org.gradle.rust.substrate.socket.path", previousSocketPath);
        }
    }

    @Test
    public void missingDaemonBinaryFailsClosedInAuthoritativeMode() throws Exception {
        Path tempDir = Files.createTempDirectory("substrate-lifecycle-");
        String previousSocketPath = System.getProperty("org.gradle.rust.substrate.socket.path");
        System.setProperty("org.gradle.rust.substrate.socket.path", tempDir.resolve("substrate.sock").toString());
        try {
            File missingBinary = tempDir.resolve("missing-daemon").toFile();
            RustDaemonSidecarLauncher.connectOrLaunch(options(true, true, missingBinary));
            fail("Expected authoritative substrate mode to fail closed");
        } catch (SubstrateException e) {
            assertTrue(e.getMessage().contains("authoritative but unavailable"));
            assertTrue(e.getMessage().contains("daemon-binary-missing"));
        } finally {
            restoreProperty("org.gradle.rust.substrate.socket.path", previousSocketPath);
        }
    }

    @Test
    public void requestedJvmHostStartupFailureIsNotIgnored() throws Exception {
        Path tempDir = Files.createTempDirectory("substrate-lifecycle-");
        String previousUnixSocket = System.getProperty("org.gradle.rust.substrate.unixSocket");
        System.setProperty("org.gradle.rust.substrate.unixSocket", "true");
        Files.createFile(tempDir.resolve("substrate.sock"));
        Path blockedJvmHostSocket = Files.createDirectory(tempDir.resolve("jvm-host.sock"));
        Files.createFile(blockedJvmHostSocket.resolve("child"));

        try {
            DaemonLauncher.withJvmHost(tempDir.resolve("daemon").toFile(), tempDir.toFile()).launchOrConnect();
            fail("Expected JVM host startup failure to abort launch");
        } catch (IOException e) {
            assertTrue(e.getMessage().contains("JVM host was requested but failed to start"));
        } finally {
            restoreProperty("org.gradle.rust.substrate.unixSocket", previousUnixSocket);
        }
    }

    @Test
    public void daemonLauncherExposesPersistedTcpEndpointPath() throws Exception {
        Path tempDir = Files.createTempDirectory("substrate-lifecycle-");
        DaemonLauncher launcher = DaemonLauncher.of(tempDir.resolve("daemon").toFile(), tempDir.toFile());

        assertEquals(tempDir.resolve("substrate.tcp-endpoint").toString(), launcher.getTcpEndpointPath());
    }

    @Test
    public void shadowModeDoesNotStartJvmHostUnlessExplicitlyRequested() {
        Map<String, String> values = new HashMap<>();
        values.put(RustSubstrateOptions.SUBSTRATE_MODE.getPropertyName(), "shadow");

        assertTrue(RustSubstrateOptions.isSubstrateEnabled(new DefaultInternalOptions(values)));
        assertFalse(RustBridgeCoreServices.shouldEnableJvmHost(new DefaultInternalOptions(values)));
    }

    @Test
    public void explicitJvmHostFlagStillEnablesCompatibilityBackchannel() {
        Map<String, String> values = new HashMap<>();
        values.put(RustSubstrateOptions.ENABLE_SUBSTRATE.getPropertyName(), "true");
        values.put(RustSubstrateOptions.ENABLE_JVM_HOST.getPropertyName(), "true");

        assertTrue(RustBridgeCoreServices.shouldEnableJvmHost(new DefaultInternalOptions(values)));
    }

    private static DefaultInternalOptions options(boolean enabled, boolean authoritative, File daemonBinary) {
        Map<String, String> values = new HashMap<>();
        values.put(RustSubstrateOptions.ENABLE_SUBSTRATE.getPropertyName(), Boolean.toString(enabled));
        values.put(RustSubstrateOptions.ENABLE_AUTHORITATIVE_EXECUTION.getPropertyName(), Boolean.toString(authoritative));
        values.put(RustSubstrateOptions.DAEMON_BINARY_PATH.getPropertyName(), daemonBinary.getAbsolutePath());
        return new DefaultInternalOptions(values);
    }

    private static void restoreProperty(String key, String previousValue) {
        if (previousValue == null) {
            System.clearProperty(key);
        } else {
            System.setProperty(key, previousValue);
        }
    }
}
