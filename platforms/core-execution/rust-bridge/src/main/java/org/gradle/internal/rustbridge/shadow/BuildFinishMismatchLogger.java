package org.gradle.internal.rustbridge.shadow;

import org.gradle.internal.rustbridge.RootBuildLifecycleBridge;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;
import org.jspecify.annotations.Nullable;

/**
 * Build lifecycle listener that logs shadow mode mismatch summary at build end.
 */
@ServiceScope(Scope.Build.class)
public class BuildFinishMismatchLogger implements RootBuildLifecycleBridge.Callbacks {

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
