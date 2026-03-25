package org.gradle.internal.rustbridge.resources;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

/**
 * Shadow adapter that compares JVM resource management with Rust.
 *
 * <p>In shadow mode, queries resource usage and performs reservations
 * through both JVM and Rust paths, reporting mismatches.</p>
 */
public class ShadowingResourceCoordinator {

    private static final Logger LOGGER = Logging.getLogger(ShadowingResourceCoordinator.class);

    private final RustResourceManagementClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingResourceCoordinator(
        RustResourceManagementClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Shadow a get-resource-usage query, comparing usage strings.
     *
     * <p>Queries the Rust resource usage and compares the usage string
     * with the Java usage string.</p>
     *
     * @param buildId    the build identifier
     * @param javaUsage  the JVM resource usage string
     */
    public void shadowGetResourceUsage(String buildId, String javaUsage) {
        try {
            String rustUsage = rustClient.getResourceUsage(buildId);

            if (javaUsage.equals(rustUsage)) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:resources] shadow getResourceUsage MATCH: buildId={}", buildId);
            } else {
                mismatchReporter.reportMismatch(
                    "resources:getResourceUsage:" + buildId,
                    javaUsage,
                    rustUsage
                );
                LOGGER.debug("[substrate:resources] shadow getResourceUsage MISMATCH: buildId={}, java={}, rust={}",
                    buildId, javaUsage, rustUsage);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("resources:getResourceUsage:" + buildId, e);
            LOGGER.debug("[substrate:resources] shadow getResourceUsage error for buildId={}: {}",
                buildId, e.getMessage());
        }
    }

    /**
     * Fire-and-forget shadow of a resource reservation (side effect).
     *
     * <p>Performs the reservation in Rust, logs the result, and reports
     * match/mismatch. Since reservations are side-effect-only, the
     * comparison is best-effort.</p>
     *
     * @param buildId      the build identifier
     * @param javaSuccess  whether the JVM reservation succeeded
     */
    public void shadowReserveResources(String buildId, boolean javaSuccess) {
        try {
            boolean rustResult = rustClient.reserveResources(buildId, 0L, 0);

            if (javaSuccess == rustResult) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:resources] shadow reserveResources MATCH: buildId={}", buildId);
            } else {
                mismatchReporter.reportMismatch(
                    "resources:reserveResources:" + buildId,
                    String.valueOf(javaSuccess),
                    String.valueOf(rustResult)
                );
                LOGGER.debug("[substrate:resources] shadow reserveResources MISMATCH: buildId={}, java={}, rust={}",
                    buildId, javaSuccess, rustResult);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("resources:reserveResources:" + buildId, e);
            LOGGER.debug("[substrate:resources] shadow reserveResources error for buildId={}: {}",
                buildId, e.getMessage());
        }
    }
}
