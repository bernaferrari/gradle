package org.gradle.internal.rustbridge.buildresult;

import org.gradle.api.logging.Logging;
import org.gradle.initialization.RootBuildLifecycleListener;
import org.gradle.internal.rustbridge.history.RustExecutionHistoryClient;
import org.gradle.internal.rustbridge.metrics.BuildMetricsRecorder;
import org.gradle.internal.rustbridge.metrics.RustBuildMetricsClient;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

/**
 * A {@link RootBuildLifecycleListener} that reports build results to the Rust substrate
 * when a build completes. Also feeds the build metrics recorder for performance tracking.
 */
public class BuildResultShadowListener implements RootBuildLifecycleListener {

    private static final Logger LOGGER = Logging.getLogger(BuildResultShadowListener.class);

    private final RustBuildResultClient client;
    private final BuildMetricsRecorder metricsRecorder;
    @Nullable
    private final RustExecutionHistoryClient historyClient;
    private final long startTimeMs;

    public BuildResultShadowListener(RustBuildResultClient client) {
        this(client, null, null);
    }

    public BuildResultShadowListener(RustBuildResultClient client, @Nullable RustBuildMetricsClient metricsClient) {
        this(client, metricsClient, null);
    }

    public BuildResultShadowListener(RustBuildResultClient client, @Nullable RustBuildMetricsClient metricsClient, @Nullable RustExecutionHistoryClient historyClient) {
        this.client = client;
        this.metricsRecorder = metricsClient != null ? new BuildMetricsRecorder(metricsClient, "build") : null;
        this.historyClient = historyClient;
        this.startTimeMs = System.currentTimeMillis();
    }

    @Override
    public void afterStart() {
        if (metricsRecorder != null) {
            metricsRecorder.recordBuildStart();
        }
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

            // Feed metrics
            if (metricsRecorder != null) {
                metricsRecorder.recordBuildEnd(failure == null);
            }

            LOGGER.debug("[substrate:buildresult] build completed in {}ms (success={})",
                durationMs, failure == null);

            // Log execution history stats for monitoring
            if (historyClient != null) {
                try {
                    gradle.substrate.v1.GetHistoryStatsResponse stats = historyClient.getStats();
                    if (stats.getEntryCount() > 0) {
                        LOGGER.info("[substrate:buildresult] history stats: {} entries, {}KB, hit rate {:.1f}%, {} stores, {} removes",
                            stats.getEntryCount(),
                            stats.getTotalBytesStored() / 1024,
                            stats.getHitRate() * 100,
                            stats.getStores(),
                            stats.getRemoves());
                    }
                } catch (Exception e) {
                    LOGGER.debug("[substrate:buildresult] failed to log history stats", e);
                }
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildresult] shadow build result failed", e);
        }
    }
}
