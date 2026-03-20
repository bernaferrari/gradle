package org.gradle.internal.rustbridge.shadow;

import org.gradle.api.logging.Logging;
import org.gradle.internal.hash.HashCode;
import org.slf4j.Logger;

/**
 * Reports hash mismatches between Java and Rust implementations.
 * Thread-safe for concurrent use during builds.
 */
public class HashMismatchReporter {

    private static final Logger LOGGER = Logging.getLogger(HashMismatchReporter.class);
    private static final int MAX_MISMATCH_PATHS_LOGGED = 20;

    private final MismatchAccumulator accumulator;
    private final boolean reportMismatches;

    public HashMismatchReporter(boolean reportMismatches) {
        this.accumulator = new MismatchAccumulator();
        this.reportMismatches = reportMismatches;
    }

    public void reportMatch() {
        accumulator.recordMatch();
    }

    public void reportMismatch(String filePath, HashCode javaHash, HashCode rustHash) {
        accumulator.recordMismatch(filePath);
        if (reportMismatches) {
            LOGGER.warn(
                "[substrate] HASH MISMATCH for {}: java={} rust={}",
                filePath, javaHash, rustHash
            );
        }
    }

    public void reportMismatch(String filePath, HashCode javaHash, byte[] rustHashBytes) {
        reportMismatch(filePath, javaHash, HashCode.fromBytes(rustHashBytes));
    }

    public void reportRustError(String filePath, Throwable error) {
        accumulator.recordRustError();
        if (reportMismatches && LOGGER.isDebugEnabled()) {
            LOGGER.debug("[substrate] Rust hash error for {}: {}", filePath, error.getMessage());
        }
    }

    public void reportJavaError(String filePath, Throwable error) {
        accumulator.recordJavaError();
        if (reportMismatches) {
            LOGGER.warn("[substrate] Java hash error for {}: {}", filePath, error.getMessage());
        }
    }

    public MismatchSummary getSummary() {
        return accumulator.snapshot();
    }

    /**
     * Logs a summary of all mismatches at build end.
     */
    public void logSummary() {
        MismatchSummary summary = getSummary();
        if (summary.getTotalComparisons() == 0) {
            return;
        }

        if (summary.hasMismatches()) {
            LOGGER.lifecycle("[substrate] {}", summary);
            List<String> paths = summary.getMismatchPaths();
            int limit = Math.min(paths.size(), MAX_MISMATCH_PATHS_LOGGED);
            for (int i = 0; i < limit; i++) {
                LOGGER.lifecycle("[substrate]   mismatch: {}", paths.get(i));
            }
            if (paths.size() > limit) {
                LOGGER.lifecycle("[substrate]   ... and {} more", paths.size() - limit);
            }
        } else {
            LOGGER.lifecycle("[substrate] {}", summary);
        }
    }
}
