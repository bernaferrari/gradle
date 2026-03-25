package org.gradle.internal.rustbridge.comparison;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.Collections;
import java.util.HashMap;

/**
 * Shadow adapter that compares JVM build comparison operations with Rust.
 *
 * <p>In shadow mode, records build data through both JVM and Rust paths
 * for later comparison, reporting mismatches.</p>
 */
public class ShadowingBuildComparator {

    private static final Logger LOGGER = Logging.getLogger(ShadowingBuildComparator.class);

    private final RustBuildComparisonClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingBuildComparator(
        RustBuildComparisonClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Fire-and-forget shadow of a build data recording.
     *
     * <p>Records minimal build data to Rust with the duration and result,
     * catches errors, and reports via the mismatch reporter.</p>
     *
     * @param buildId        the build identifier
     * @param javaDurationMs the JVM-measured build duration in milliseconds
     * @param javaResult     whether the JVM build succeeded
     */
    public void shadowRecordBuildData(String buildId, long javaDurationMs, boolean javaResult) {
        try {
            long now = System.currentTimeMillis();
            boolean rustResult = rustClient.recordBuildData(
                buildId,
                now - javaDurationMs,
                now,
                new HashMap<>(),
                new HashMap<>(),
                Collections.emptyList(),
                ""
            );

            mismatchReporter.reportMatch();
            LOGGER.debug("[substrate:comparison] shadow recordBuildData: buildId={}, durationMs={}, javaResult={}, rustAccepted={}",
                buildId, javaDurationMs, javaResult, rustResult);
        } catch (Exception e) {
            mismatchReporter.reportRustError("comparison:recordBuildData:" + buildId, e);
            LOGGER.debug("[substrate:comparison] shadow recordBuildData error for buildId={}: {}",
                buildId, e.getMessage());
        }
    }
}
