package org.gradle.internal.rustbridge.eventstream;

/**
 * Thread-safe holder for the current build ID.
 * Written by BootstrapLifecycleListener.afterStart(), read by event forwarders.
 */
public class BuildIdHolder {
    private static volatile String currentBuildId = "";

    public static void setBuildId(String buildId) {
        currentBuildId = buildId != null ? buildId : "";
    }

    public static String getBuildId() {
        return currentBuildId;
    }

    public static void clear() {
        currentBuildId = "";
    }
}
