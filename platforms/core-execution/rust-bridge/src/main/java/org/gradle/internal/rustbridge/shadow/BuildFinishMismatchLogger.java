package org.gradle.internal.rustbridge.shadow;

import org.gradle.api.logging.Logging;
import org.gradle.internal.buildoption.InternalOptions;
import org.gradle.internal.buildoption.RustSubstrateOptions;
import org.gradle.internal.build.internal.BuildLifecycleListener;
import org.slf4j.Logger;

import java.util.concurrent.atomic.AtomicReference;

/**
 * Build lifecycle listener that logs shadow mode mismatch summary at build end.
 */
public class BuildFinishMismatchLogger implements BuildLifecycleListener {

    private static final Logger LOGGER = Logging.getLogger(BuildFinishMismatchLogger.class);
    private static final AtomicReference<HashMismatchReporter> REPORTER = new AtomicReference<>();

    private final boolean reportMismatches;

    public BuildFinishMismatchLogger(InternalOptions options) {
        this.reportMismatches = options.getOption(RustSubstrateOptions.REPORT_MISMATCHES).get();
    }

    /**
     * Registers a reporter to be logged at build end.
     * Only one reporter can be active at a time.
     */
    public static void setReporter(HashMismatchReporter reporter) {
        REPORTER.set(reporter);
    }

    @Override
    public void afterStart() {
        // No-op at build start
    }

    @Override
    public void beforeComplete() {
        HashMismatchReporter reporter = REPORTER.getAndSet(null);
        if (reporter != null && reportMismatches) {
            reporter.logSummary();
        }
    }
}
