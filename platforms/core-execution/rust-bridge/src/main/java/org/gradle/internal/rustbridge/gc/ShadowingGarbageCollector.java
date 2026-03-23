package org.gradle.internal.rustbridge.gc;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;
import org.jspecify.annotations.Nullable;

/**
 * Shadow adapter that compares JVM garbage collection results with Rust.
 *
 * <p>Runs GC on both JVM stores and Rust stores, then compares
 * entries-removed and bytes-recovered counts.</p>
 */
public class ShadowingGarbageCollector {

    private static final Logger LOGGER = Logging.getLogger(ShadowingGarbageCollector.class);

    private final RustGarbageCollectionClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingGarbageCollector(
        RustGarbageCollectionClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Compare JVM build cache GC with Rust build cache GC.
     */
    public void compareBuildCacheGc(long maxAgeMs, long maxTotalBytes, boolean dryRun,
                                      @Nullable Integer javaEntriesRemoved,
                                      @Nullable Long javaBytesRecovered) {
        try {
            RustGarbageCollectionClient.GcResult rustResult =
                rustClient.gcBuildCache(maxAgeMs, maxTotalBytes, dryRun);

            if (rustResult == null) {
                mismatchReporter.reportRustError("gc:build-cache", new RuntimeException("Rust returned null"));
                return;
            }

            if (javaEntriesRemoved != null && javaEntriesRemoved != rustResult.getEntriesRemoved()) {
                mismatchReporter.reportMismatch(
                    "gc:build-cache:entriesRemoved",
                    String.valueOf(javaEntriesRemoved),
                    String.valueOf(rustResult.getEntriesRemoved())
                );
            } else if (javaBytesRecovered != null && javaBytesRecovered != rustResult.getBytesRecovered()) {
                mismatchReporter.reportMismatch(
                    "gc:build-cache:bytesRecovered",
                    String.valueOf(javaBytesRecovered),
                    String.valueOf(rustResult.getBytesRecovered())
                );
            } else {
                mismatchReporter.reportMatch();
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("gc:build-cache", e);
        }
    }

    /**
     * Compare JVM execution history GC with Rust execution history GC.
     */
    public void compareExecutionHistoryGc(long maxAgeMs, int maxEntries, boolean dryRun,
                                            @Nullable Integer javaEntriesRemoved) {
        try {
            RustGarbageCollectionClient.GcResult rustResult =
                rustClient.gcExecutionHistory(maxAgeMs, maxEntries, dryRun);

            if (rustResult == null) {
                mismatchReporter.reportRustError("gc:execution-history", new RuntimeException("Rust returned null"));
                return;
            }

            if (javaEntriesRemoved != null && javaEntriesRemoved != rustResult.getEntriesRemoved()) {
                mismatchReporter.reportMismatch(
                    "gc:execution-history:entriesRemoved",
                    String.valueOf(javaEntriesRemoved),
                    String.valueOf(rustResult.getEntriesRemoved())
                );
            } else {
                mismatchReporter.reportMatch();
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("gc:execution-history", e);
        }
    }

    /**
     * Compare JVM config cache GC with Rust config cache GC.
     */
    public void compareConfigCacheGc(long maxAgeMs, int maxEntries, boolean dryRun,
                                      @Nullable Integer javaEntriesRemoved) {
        try {
            RustGarbageCollectionClient.GcResult rustResult =
                rustClient.gcConfigCache(maxAgeMs, maxEntries, dryRun);

            if (rustResult == null) {
                mismatchReporter.reportRustError("gc:config-cache", new RuntimeException("Rust returned null"));
                return;
            }

            if (javaEntriesRemoved != null && javaEntriesRemoved != rustResult.getEntriesRemoved()) {
                mismatchReporter.reportMismatch(
                    "gc:config-cache:entriesRemoved",
                    String.valueOf(javaEntriesRemoved),
                    String.valueOf(rustResult.getEntriesRemoved())
                );
            } else {
                mismatchReporter.reportMatch();
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("gc:config-cache", e);
        }
    }
}
