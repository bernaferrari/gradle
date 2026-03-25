package org.gradle.internal.rustbridge.shadow;

import org.gradle.internal.logging.text.StyledTextOutput;
import org.gradle.internal.logging.text.StyledTextOutputFactory;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.Collections;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

import static org.gradle.internal.logging.text.StyledTextOutput.Style.*;

/**
 * Reports hash mismatches between Java and Rust sides of the bridge.
 * Tracks per-subsystem statistics and enforces mismatch rate thresholds.
 */
public class HashMismatchReporter {

    private static final Logger LOGGER = LoggerFactory.getLogger(HashMismatchReporter.class);

    /**
     * Maximum allowed mismatch rate before the system is considered out of tolerance.
     * 1% of total checks may be mismatches.
     */
    public static final double MAX_MISMATCH_RATE = 0.01;

    private final AtomicLong totalMatches = new AtomicLong(0);
    private final AtomicLong totalMismatches = new AtomicLong(0);
    private final AtomicLong totalErrors = new AtomicLong(0);

    private final ConcurrentHashMap<String, SubsystemStats> subsystemStats = new ConcurrentHashMap<>();

    private final StyledTextOutputFactory textOutputFactory;
    private final String reporterName;
    private final boolean reportingEnabled;

    // --- SubsystemStats inner class ---

    /**
     * Per-subsystem tracking with thread-safe counters.
     */
    public static class SubsystemStats {
        private final AtomicLong matchCount = new AtomicLong(0);
        private final AtomicLong mismatchCount = new AtomicLong(0);
        private final AtomicLong errorCount = new AtomicLong(0);

        public void recordMatch() {
            matchCount.incrementAndGet();
        }

        public void recordMismatch() {
            mismatchCount.incrementAndGet();
        }

        public void recordError() {
            errorCount.incrementAndGet();
        }

        public long getMatchCount() {
            return matchCount.get();
        }

        public long getMismatchCount() {
            return mismatchCount.get();
        }

        public long getErrorCount() {
            return errorCount.get();
        }

        public long getTotal() {
            return matchCount.get() + mismatchCount.get() + errorCount.get();
        }

        /**
         * Returns the mismatch rate as a value between 0.0 and 1.0.
         * If no operations have been recorded, returns 0.0.
         */
        public double getMismatchRate() {
            long total = getTotal();
            if (total == 0) {
                return 0.0;
            }
            return (double) mismatchCount.get() / total;
        }

        @Override
        public String toString() {
            return String.format(
                "SubsystemStats{matches=%d, mismatches=%d, errors=%d, total=%d, mismatchRate=%.4f}",
                getMatchCount(), getMismatchCount(), getErrorCount(), getTotal(), getMismatchRate()
            );
        }
    }

    // --- Constructors ---

    public HashMismatchReporter(StyledTextOutputFactory textOutputFactory) {
        this(textOutputFactory, "HashMismatchReporter");
    }

    public HashMismatchReporter(StyledTextOutputFactory textOutputFactory, String reporterName) {
        this.textOutputFactory = textOutputFactory;
        this.reporterName = reporterName;
        this.reportingEnabled = true;
    }

    /**
     * Backward-compatible constructor used by older wiring.
     * The boolean controls whether mismatch/error logging is emitted.
     */
    public HashMismatchReporter(boolean reportingEnabled) {
        this.textOutputFactory = null;
        this.reporterName = "HashMismatchReporter";
        this.reportingEnabled = reportingEnabled;
    }

    // --- Existing API (unchanged signatures) ---

    /**
     * Reports a successful hash match.
     */
    public void reportMatch() {
        reportMatch("default");
    }

    /**
     * Reports a hash mismatch between Java and Rust.
     */
    public void reportMismatch(String java, String rust) {
        reportMismatch("default", java, rust);
    }

    /**
     * Reports an error during hash comparison.
     */
    public void reportError() {
        reportError("default");
    }

    public long getTotalMatches() {
        return totalMatches.get();
    }

    public long getTotalMismatches() {
        return totalMismatches.get();
    }

    public long getTotalErrors() {
        return totalErrors.get();
    }

    public long getTotalChecks() {
        return totalMatches.get() + totalMismatches.get() + totalErrors.get();
    }

    public double getOverallMismatchRate() {
        long total = getTotalChecks();
        if (total == 0) {
            return 0.0;
        }
        return (double) totalMismatches.get() / total;
    }

    /**
     * Logs a summary of all hash checks.
     */
    public void logSummary() {
        if (textOutputFactory == null) {
            LOGGER.info(
                "[{}] checks={}, matches={}, mismatches={}, errors={}, mismatchRate={}%",
                reporterName,
                getTotalChecks(),
                getTotalMatches(),
                getTotalMismatches(),
                getTotalErrors(),
                String.format("%.2f", getOverallMismatchRate() * 100)
            );
            return;
        }

        StyledTextOutput output = textOutputFactory.create(reporterName);

        long checks = getTotalChecks();
        long matches = getTotalMatches();
        long mismatches = getTotalMismatches();
        long errors = getTotalErrors();
        double rate = getOverallMismatchRate();

        output.println();
        output.style(Header).formatln("=== %s Summary ===", reporterName);
        output.formatln("  Total checks:  %d", checks);
        output.style(Success).formatln("  Matches:       %d", matches);
        if (mismatches > 0) {
            output.style(Failure).formatln("  Mismatches:    %d (%.2f%%)", mismatches, rate * 100);
        } else {
            output.formatln("  Mismatches:    %d", mismatches);
        }
        if (errors > 0) {
            output.style(Info).formatln("  Errors:        %d", errors);
        } else {
            output.formatln("  Errors:        %d", errors);
        }

        // Per-subsystem breakdown
        Map<String, SubsystemStats> summaries = getSubsystemSummaries();
        if (!summaries.isEmpty()) {
            output.println();
            output.style(Header).println("  Per-Subsystem Breakdown:");
            for (Map.Entry<String, SubsystemStats> entry : summaries.entrySet()) {
                String subsystem = entry.getKey();
                SubsystemStats stats = entry.getValue();
                output.format("    %-20s", subsystem);
                output.format("total=%-6d", stats.getTotal());
                output.format("matches=%-6d", stats.getMatchCount());
                if (stats.getMismatchCount() > 0) {
                    output.style(Failure).format("mismatches=%-6d", stats.getMismatchCount());
                } else {
                    output.format("mismatches=%-6d", stats.getMismatchCount());
                }
                if (stats.getErrorCount() > 0) {
                    output.style(Info).format("errors=%-6d", stats.getErrorCount());
                } else {
                    output.format("errors=%-6d", stats.getErrorCount());
                }
                output.formatln("rate=%.2f%%", stats.getMismatchRate() * 100);
            }
        }

        // Tolerance status
        if (checks > 0) {
            if (isWithinTolerance()) {
                output.style(Success).println("  Status: WITHIN TOLERANCE");
            } else {
                output.style(Failure).formatln("  Status: EXCEEDS TOLERANCE (max %.1f%%)", MAX_MISMATCH_RATE * 100);
            }
        }

        output.println();
    }

    // --- New overloaded methods ---

    /**
     * Reports a successful hash match for a specific subsystem.
     *
     * @param subsystem the subsystem identifier (e.g., "hash", "cache", "fingerprint")
     */
    public void reportMatch(String subsystem) {
        totalMatches.incrementAndGet();
        getOrCreateSubsystem(subsystem).recordMatch();
    }

    /**
     * Reports a hash mismatch for a specific subsystem.
     *
     * @param subsystem the subsystem identifier
     * @param java      the Java-side hash value
     * @param rust      the Rust-side hash value
     */
    public void reportMismatch(String subsystem, String java, String rust) {
        totalMismatches.incrementAndGet();
        getOrCreateSubsystem(subsystem).recordMismatch();
        if (reportingEnabled) {
            LOGGER.warn("[{}] Hash mismatch in subsystem '{}': java={}, rust={}", reporterName, subsystem, java, rust);
        }
    }

    /**
     * Backward-compatible mismatch API for callers that pass non-string values
     * (e.g. HashCode, byte[]).
     */
    public void reportMismatch(String subsystem, Object java, Object rust) {
        reportMismatch(subsystem, stringify(java), stringify(rust));
    }

    /**
     * Reports an error for a specific subsystem.
     *
     * @param subsystem the subsystem identifier
     */
    public void reportError(String subsystem) {
        totalErrors.incrementAndGet();
        getOrCreateSubsystem(subsystem).recordError();
    }

    public void reportRustError(String subsystem, Exception error) {
        reportError(subsystem);
        if (reportingEnabled) {
            LOGGER.debug("[{}] Rust error in subsystem '{}': {}", reporterName, subsystem, error.getMessage(), error);
        }
    }

    public void reportJavaError(String subsystem, Exception error) {
        reportError(subsystem);
        if (reportingEnabled) {
            LOGGER.debug("[{}] Java error in subsystem '{}': {}", reporterName, subsystem, error.getMessage(), error);
        }
    }

    // --- New query methods ---

    /**
     * Returns an unmodifiable snapshot of per-subsystem statistics.
     */
    public Map<String, SubsystemStats> getSubsystemSummaries() {
        // Return a defensive copy of the keys to avoid ConcurrentModification
        return Collections.unmodifiableMap(new ConcurrentHashMap<>(subsystemStats));
    }

    /**
     * Returns true if the specified subsystem has recorded any mismatches.
     *
     * @param subsystem the subsystem to check
     * @return true if at least one mismatch has been recorded for this subsystem
     */
    public boolean hasSubsystemMismatches(String subsystem) {
        SubsystemStats stats = subsystemStats.get(subsystem);
        return stats != null && stats.getMismatchCount() > 0;
    }

    /**
     * Returns the mismatch rate for a specific subsystem, as a value between 0.0 and 1.0.
     * Returns 0.0 if the subsystem has no recorded operations.
     *
     * @param subsystem the subsystem to query
     * @return the mismatch rate
     */
    public double getMismatchRate(String subsystem) {
        SubsystemStats stats = subsystemStats.get(subsystem);
        if (stats == null) {
            return 0.0;
        }
        return stats.getMismatchRate();
    }

    /**
     * Checks whether the overall mismatch rate is within the configured tolerance
     * ({@link #MAX_MISMATCH_RATE}).
     * Returns true if no checks have been performed (vacuously within tolerance).
     *
     * @return true if mismatch rate is at or below the threshold
     */
    public boolean isWithinTolerance() {
        return getOverallMismatchRate() <= MAX_MISMATCH_RATE;
    }

    /**
     * Checks whether a specific subsystem's mismatch rate is within the configured tolerance.
     *
     * @param subsystem the subsystem to check
     * @return true if the subsystem's mismatch rate is at or below the threshold,
     *         or if no operations have been recorded for the subsystem
     */
    public boolean isWithinTolerance(String subsystem) {
        return getMismatchRate(subsystem) <= MAX_MISMATCH_RATE;
    }

    // --- Internal helpers ---

    private SubsystemStats getOrCreateSubsystem(String subsystem) {
        return subsystemStats.computeIfAbsent(subsystem, k -> new SubsystemStats());
    }

    private static String stringify(Object value) {
        if (value instanceof byte[]) {
            byte[] bytes = (byte[]) value;
            StringBuilder sb = new StringBuilder(bytes.length * 2);
            for (byte b : bytes) {
                sb.append(Character.forDigit((b >> 4) & 0xF, 16));
                sb.append(Character.forDigit(b & 0xF, 16));
            }
            return sb.toString();
        }
        return String.valueOf(value);
    }
}
