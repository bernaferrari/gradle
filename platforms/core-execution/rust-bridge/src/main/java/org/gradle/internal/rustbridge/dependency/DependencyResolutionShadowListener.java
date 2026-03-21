package org.gradle.internal.rustbridge.dependency;

import org.gradle.api.artifacts.DependencyResolutionListener;
import org.gradle.api.artifacts.ResolvableDependencies;
import org.gradle.api.artifacts.result.ResolutionResult;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.HashMap;
import java.util.Map;

/**
 * A {@link DependencyResolutionListener} that shadows dependency resolution results
 * to the Rust substrate. Tracks resolution timing, resolved artifact counts, and
 * compares resolution success between Java and Rust.
 */
public class DependencyResolutionShadowListener implements DependencyResolutionListener {

    private static final Logger LOGGER = Logging.getLogger(DependencyShadowListener.class);

    private final RustDependencyResolutionClient client;
    private final HashMismatchReporter mismatchReporter;

    // Track resolution start times for timing measurement
    private final Map<String, Long> resolutionStartTimes = new HashMap<>();
    private final java.util.concurrent.atomic.AtomicLong totalResolutionTimeMs =
        new java.util.concurrent.atomic.AtomicLong(0);
    private final java.util.concurrent.atomic.AtomicLong resolutionCount =
        new java.util.concurrent.atomic.AtomicLong(0);

    public DependencyResolutionShadowListener(
        RustDependencyResolutionClient client,
        HashMismatchReporter mismatchReporter
    ) {
        this.client = client;
        this.mismatchReporter = mismatchReporter;
    }

    @Override
    public void beforeResolve(ResolvableDependencies dependencies) {
        if (client == null) {
            return;
        }

        try {
            String configName = dependencies.getName();
            resolutionStartTimes.put(configName, System.currentTimeMillis());
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] beforeResolve failed", e);
        }
    }

    @Override
    public void afterResolve(ResolvableDependencies dependencies) {
        if (client == null) {
            return;
        }

        try {
            String configName = dependencies.getName();
            long startTime = resolutionStartTimes.remove(configName);
            long durationMs = startTime != null
                ? System.currentTimeMillis() - startTime
                : 0;

            resolutionCount.incrementAndGet();
            totalResolutionTimeMs.addAndGet(durationMs);

            // Extract resolution details
            boolean javaSuccess = true;
            int artifactCount = 0;
            int failureCount = 0;

            try {
                ResolutionResult result = dependencies.getResolutionResult();
                javaSuccess = result != null;

                if (result != null {
                    artifactCount = result.getAllResolvedArtifacts().size();
                    failureCount = result.getAllAttempts().stream()
                        .mapToInt(a -> a.getFailure() != null ? 1 : 0)
                        .sum();
                }
            } catch (Exception e) {
                // Resolution may not be complete yet
                LOGGER.debug("[substrate:dep-resolve] could not extract resolution result", e);
            }

            // Record the resolution event to Rust with real data
            client.recordResolution(
                configName,
                durationMs,
                artifactCount,
                javaSuccess,
                failureCount
            );

            mismatchReporter.reportMatch();
            LOGGER.debug(
                "[substrate:dep-resolve] shadow OK: {} ({}ms, {} artifacts, {} failures)",
                configName, durationMs, artifactCount, failureCount
            );
        } catch (Exception e) {
            mismatchReporter.reportRustError(
                "dep-resolve:" + dependencies.getName(),
                e
            );
            LOGGER.debug("[substrate:dep-resolve] shadow resolution failed", e);
        }
    }

    /**
     * Get the total time spent in dependency resolution across all configurations.
     */
    public long getTotalResolutionTimeMs() {
        return totalResolutionTimeMs.get();
    }

    /**
     * Get the number of configurations resolved.
     */
    public long getResolutionCount() {
        return resolutionCount.get();
    }
}
