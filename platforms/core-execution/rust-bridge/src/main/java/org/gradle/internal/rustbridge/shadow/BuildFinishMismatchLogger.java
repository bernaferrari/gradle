package org.gradle.internal.rustbridge.shadow;

import org.gradle.initialization.RootBuildLifecycleListener;
import org.jspecify.annotations.Nullable;

/**
 * Build lifecycle listener that logs shadow mode mismatch summary at build end.
 */
public class BuildFinishMismatchLogger implements RootBuildLifecycleListener {

    private final HashMismatchReporter reporter;

    public BuildFinishMismatchLogger(HashMismatchReporter reporter) {
        this.reporter = reporter;
    }

    @Override
    public void afterStart() {
        // No-op at build start
    }

    @Override
    public void beforeComplete(@Nullable Throwable failure) {
        reporter.logSummary();
    }
}
