package org.gradle.internal.rustbridge.buildinit;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

/**
 * Shadow adapter that compares JVM build init data with Rust.
 *
 * <p>Tracks build settings, init scripts, and init status
 * and compares against the Rust BuildInitService.</p>
 */
public class ShadowingBuildInitTracker {

    private static final Logger LOGGER = Logging.getLogger(ShadowingBuildInitTracker.class);

    private final RustBuildInitClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingBuildInitTracker(
        RustBuildInitClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Compare JVM build init status with Rust build init status.
     */
    public void compareInitStatus(String buildId, boolean javaInitialized,
                                   int javaSettingsCount) {
        try {
            gradle.substrate.v1.GetBuildInitStatusResponse rustResponse =
                rustClient.getBuildInitStatus(buildId);
            boolean rustInitialized = rustResponse.getInitialized();
            int rustSettingsCount = rustResponse.getSettingsDetailsCount();

            if (javaInitialized != rustInitialized) {
                mismatchReporter.reportMismatch(
                    "build-init:status:" + buildId,
                    String.valueOf(javaInitialized),
                    String.valueOf(rustInitialized)
                );
            } else if (javaSettingsCount != rustSettingsCount) {
                mismatchReporter.reportMismatch(
                    "build-init:settingsCount:" + buildId,
                    String.valueOf(javaSettingsCount),
                    String.valueOf(rustSettingsCount)
                );
            } else {
                mismatchReporter.reportMatch();
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("build-init:status:" + buildId, e);
        }
    }
}
