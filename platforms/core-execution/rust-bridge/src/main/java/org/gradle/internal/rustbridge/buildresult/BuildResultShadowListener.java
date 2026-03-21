package org.gradle.internal.rustbridge.buildresult;

import org.gradle.api.logging.Logging;
import org.gradle.initialization.RootBuildLifecycleListener;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

/**
 * A {@link RootBuildLifecycleListener} that reports build results to the Rust substrate
 * when a build completes.
 */
public class BuildResultShadowListener implements RootBuildLifecycleListener {

    private static final Logger LOGGER = Logging.getLogger(BuildResultShadowListener.class);

    private final RustBuildResultClient client;
    private final long startTimeMs;

    public BuildResultShadowListener(RustBuildResultClient client) {
        this.client = client;
        this.startTimeMs = System.currentTimeMillis();
    }

    @Override
    public void afterStart() {
        // Nothing to do at build start
    }

    @Override
    public void beforeComplete(@Nullable Throwable failure) {
        if (client == null) {
            return;
        }

        try {
            long durationMs = System.currentTimeMillis() - startTimeMs;

            if (failure != null) {
                String message = failure.getMessage() != null ? failure.getMessage() : failure.getClass().getName();
                client.reportBuildFailure("build", "build_failed", message, java.util.Collections.emptyList());
            }

            LOGGER.debug("[substrate:buildresult] build completed in {}ms (success={})",
                durationMs, failure == null);
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildresult] shadow build result failed", e);
        }
    }
}
