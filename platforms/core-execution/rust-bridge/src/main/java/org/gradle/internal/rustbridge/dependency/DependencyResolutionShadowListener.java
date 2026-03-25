package org.gradle.internal.rustbridge.dependency;

import org.gradle.api.artifacts.DependencyResolutionListener;
import org.gradle.api.artifacts.ResolvableDependencies;
import org.gradle.api.artifacts.result.ResolutionResult;
import org.gradle.api.artifacts.result.UnresolvedDependencyResult;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.concurrent.ConcurrentHashMap;
import java.util.Map;

/**
 * A {@link DependencyResolutionListener} that shadows dependency resolution results
 * to the Rust substrate. Tracks resolution timing, resolved artifact counts, and
 * compares resolution success between Java and Rust.
 */
public class DependencyResolutionShadowListener implements DependencyResolutionListener {

    private static final Logger LOGGER = Logging.getLogger(DependencyResolutionShadowListener.class);

    private final RustDependencyResolutionClient client;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    // Track resolution start times for timing measurement
    private final Map<String, Long> resolutionStartTimes = new ConcurrentHashMap<>();
    private final java.util.concurrent.atomic.AtomicLong totalResolutionTimeMs =
        new java.util.concurrent.atomic.AtomicLong(0);
    private final java.util.concurrent.atomic.AtomicLong resolutionCount =
        new java.util.concurrent.atomic.AtomicLong(0);

    public DependencyResolutionShadowListener(
        RustDependencyResolutionClient client,
        HashMismatchReporter mismatchReporter
    ) {
        this(client, mismatchReporter, false);
    }

    public DependencyResolutionShadowListener(
        RustDependencyResolutionClient client,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.client = client;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
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
            Long startTime = resolutionStartTimes.remove(configName);
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

                if (result != null) {
                    artifactCount = result.getAllComponents().size();
                    failureCount = result.getAllDependencies().stream()
                        .mapToInt(dep -> dep instanceof UnresolvedDependencyResult ? 1 : 0)
                        .sum();
                }
            } catch (Exception e) {
                // Resolution may not be complete yet
                LOGGER.debug("[substrate:dep-resolve] could not extract resolution result", e);
            }

            String source = recordResolutionInMode(configName, durationMs, artifactCount, javaSuccess, failureCount);
            LOGGER.debug(
                "[substrate:dep-resolve] shadow OK: {} ({}ms, {} artifacts, {} failures, source={})",
                configName, durationMs, artifactCount, failureCount, source
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

    public boolean isAuthoritative() {
        return authoritative;
    }

    private String recordResolutionInMode(
        String configName,
        long durationMs,
        int artifactCount,
        boolean javaSuccess,
        long failureCount
    ) {
        if (authoritative) {
            try {
                client.recordResolutionStrict(configName, durationMs, artifactCount, javaSuccess, failureCount);
                mismatchReporter.reportMatch();
                return "rust";
            } catch (Exception e) {
                mismatchReporter.reportRustError("dep-resolve:" + configName, e);
                // Listener side-effects must never fail the Java resolution path.
                client.recordResolution(configName, durationMs, artifactCount, javaSuccess, failureCount);
                return "java-fallback";
            }
        }

        client.recordResolution(configName, durationMs, artifactCount, javaSuccess, failureCount);
        mismatchReporter.reportMatch();
        return "java-shadow";
    }
}
