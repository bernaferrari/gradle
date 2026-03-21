package org.gradle.internal.rustbridge.metrics;

import org.gradle.api.logging.Logging;
import org.gradle.api.tasks.testing.TestDescriptor;
import org.gradle.api.tasks.testing.TestListener;
import org.gradle.api.tasks.testing.TestResult;
import org.gradle.internal.buildoption.InternalOptions;
import org.gradle.internal.buildoption.RustSubstrateOptions;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

/**
 * A {@link TestListener} that records test execution metrics to the Rust
 * build metrics service. Fire-and-forget: never affects build correctness.
 *
 * <p>Tracks test counts, pass/fail/skip rates, and timing information,
 * feeding into the Rust performance summary dashboard.</p>
 */
public class BuildMetricsRecorder {

    private static final Logger LOGGER = Logging.getLogger(BuildMetricsRecorder.class);

    private final RustBuildMetricsClient metricsClient;
    private final String buildId;

    private final AtomicLong buildStartTime = new AtomicLong(0);
    private final AtomicInteger totalTasks = new AtomicInteger(0);
    private final AtomicInteger cachedTasks = new AtomicInteger(0);
    private final AtomicInteger upToDateTasks = new AtomicInteger(0);
    private final AtomicInteger executedTasks = new AtomicInteger(0);
    private final AtomicInteger failedTasks = new AtomicInteger(0);

    public BuildMetricsRecorder(RustBuildMetricsClient metricsClient, String buildId) {
        this.metricsClient = metricsClient;
        this.buildId = buildId;
        this.buildStartTime.set(System.currentTimeMillis());
    }

    /**
     * Record the build start time.
     */
    public void recordBuildStart() {
        buildStartTime.set(System.currentTimeMillis());
        metricsClient.recordTimer(buildId, "build.start", 0);
    }

    /**
     * Record the build end and compute the performance summary.
     */
    public void recordBuildEnd(boolean success) {
        long duration = System.currentTimeMillis() - buildStartTime.get();
        metricsClient.recordTimer(buildId, "build.end", duration);
        metricsClient.recordCounter(buildId, "build.end", 1);

        // Log summary
        try {
            RustBuildMetricsClient.PerformanceSummary summary = metricsClient.getPerformanceSummary(buildId);
            if (summary != null) {
                LOGGER.info("[substrate:metrics] Build complete: {}", summary);
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:metrics] failed to get summary", e);
        }
    }

    /**
     * Record that a task was executed.
     */
    public void recordTaskExecution(String taskPath, boolean cached, boolean upToDate, boolean failed) {
        totalTasks.incrementAndGet();
        if (cached) {
            cachedTasks.incrementAndGet();
        } else if (upToDate) {
            upToDateTasks.incrementAndGet();
        } else {
            executedTasks.incrementAndGet();
        }
        if (failed) {
            failedTasks.incrementAndGet();
        }
    }

    /**
     * Record a cache hit.
     */
    public void recordCacheHit() {
        metricsClient.recordCounter(buildId, "cache.hits", 1);
    }

    /**
     * Record a cache miss.
     */
    public void recordCacheMiss() {
        metricsClient.recordCounter(buildId, "cache.misses", 1);
    }

    /**
     * Record bytes stored to cache.
     */
    public void recordCacheStore(long bytes) {
        metricsClient.recordCounter(buildId, "cache.bytes_stored", bytes);
    }

    /**
     * Record bytes loaded from cache.
     */
    public void recordCacheLoad(long bytes) {
        metricsClient.recordCounter(buildId, "cache.bytes_loaded", bytes);
    }

    /**
     * Get the build ID for this recorder.
     */
    public String getBuildId() {
        return buildId;
    }

    /**
     * Get current task counts for logging.
     */
    public String getTaskStats() {
        return String.format("total=%d, cached=%d, upToDate=%d, executed=%d, failed=%d",
            totalTasks.get(), cachedTasks.get(), upToDateTasks.get(),
            executedTasks.get(), failedTasks.get());
    }
}
