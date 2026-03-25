package org.gradle.internal.rustbridge.console;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

/**
 * Shadow adapter that monitors JVM console output against Rust.
 *
 * <p>In shadow mode, shadows log messages and build descriptions in
 * fire-and-forget mode. Console output has real side effects (UI rendering),
 * so Rust calls are best-effort and results are not used to influence Java
 * behavior.</p>
 */
public class ShadowingConsoleOutput {

    private static final Logger LOGGER = Logging.getLogger(ShadowingConsoleOutput.class);

    private final RustConsoleClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingConsoleOutput(
        RustConsoleClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Shadow a log message in fire-and-forget mode.
     *
     * <p>Log messages have real side effects (UI rendering, user-facing output),
     * so the Rust call is best-effort. The Java result is authoritative.</p>
     *
     * @param buildId    the build identifier
     * @param level      the log level (e.g. INFO, WARN, ERROR)
     * @param message    the log message
     * @param javaResult whether the JVM log call succeeded
     */
    public void shadowLogMessage(String buildId, String level, String message, boolean javaResult) {
        try {
            rustClient.logMessage(buildId, level, "", message, "");
            mismatchReporter.reportMatch();
            LOGGER.debug("[substrate:console] shadow logMessage (fire-and-forget): buildId={}, level={}, success={}",
                buildId, level, javaResult);
        } catch (Exception e) {
            mismatchReporter.reportRustError("console:logMessage:" + buildId, e);
            LOGGER.debug("[substrate:console] shadow logMessage error for buildId={}: {}",
                buildId, e.getMessage());
        }
    }

    /**
     * Shadow a build description update in fire-and-forget mode.
     *
     * <p>Setting the build description has real side effects (console header),
     * so the Rust call is best-effort and not compared against the Java result.</p>
     *
     * @param buildId      the build identifier
     * @param description  the build description
     * @param javaResult   whether the JVM setBuildDescription succeeded
     */
    public void shadowSetBuildDescription(String buildId, String description, boolean javaResult) {
        try {
            rustClient.setBuildDescription(buildId, description);
            mismatchReporter.reportMatch();
            LOGGER.debug("[substrate:console] shadow setBuildDescription (fire-and-forget): buildId={}, success={}",
                buildId, javaResult);
        } catch (Exception e) {
            mismatchReporter.reportRustError("console:setBuildDescription:" + buildId, e);
            LOGGER.debug("[substrate:console] shadow setBuildDescription error for buildId={}: {}",
                buildId, e.getMessage());
        }
    }
}
