package org.gradle.internal.rustbridge.shadow;

import org.gradle.api.logging.LogLevel;
import org.gradle.internal.logging.text.StyledTextOutput;
import org.gradle.internal.logging.text.StyledTextOutputFactory;
import org.junit.Before;
import org.junit.Test;

import java.util.Map;

import static org.junit.Assert.*;

/**
 * Unit tests for the shadow validation infrastructure, focusing on
 * {@link HashMismatchReporter} and its inner {@link HashMismatchReporter.SubsystemStats}.
 *
 * <p>These are pure-Java tests that exercise the reporter's counting, rate calculation,
 * tolerance checking, and per-subsystem tracking without needing a Rust daemon.</p>
 */
public class ShadowValidationTest {

    private HashMismatchReporter reporter;

    // --- No-op stubs so we don't need Mockito or a real service registry ---

    private static final StyledTextOutputFactory NOOP_FACTORY = new StyledTextOutputFactory() {
        @Override
        public StyledTextOutput create(String logCategory) {
            return new NoOpStyledTextOutput();
        }

        @Override
        public StyledTextOutput create(Class<?> logCategory) {
            return new NoOpStyledTextOutput();
        }

        @Override
        public StyledTextOutput create(Class<?> logCategory, LogLevel logLevel) {
            return new NoOpStyledTextOutput();
        }

        @Override
        public StyledTextOutput create(String logCategory, LogLevel logLevel) {
            return new NoOpStyledTextOutput();
        }
    };

    /** Minimal no-op implementation that silently discards all output. */
    private static class NoOpStyledTextOutput implements StyledTextOutput {
        @Override public StyledTextOutput append(char c) { return this; }
        @Override public StyledTextOutput append(CharSequence csq) { return this; }
        @Override public StyledTextOutput append(CharSequence csq, int start, int end) { return this; }
        @Override public StyledTextOutput style(Style style) { return this; }
        @Override public StyledTextOutput withStyle(Style style) { return this; }
        @Override public StyledTextOutput text(Object text) { return this; }
        @Override public StyledTextOutput println(Object text) { return this; }
        @Override public StyledTextOutput format(String pattern, Object... args) { return this; }
        @Override public StyledTextOutput formatln(String pattern, Object... args) { return this; }
        @Override public StyledTextOutput println() { return this; }
        @Override public StyledTextOutput exception(Throwable throwable) { return this; }
    }

    @Before
    public void setUp() {
        reporter = new HashMismatchReporter(NOOP_FACTORY);
    }

    // ==================== Match counting ====================

    @Test
    public void reportMatch_incrementsGlobalMatchCount() {
        reporter.reportMatch();
        reporter.reportMatch();
        reporter.reportMatch();
        assertEquals(3, reporter.getTotalMatches());
    }

    @Test
    public void reportMatch_withSubsystem_tracksPerSubsystem() {
        reporter.reportMatch("hash");
        reporter.reportMatch("hash");
        reporter.reportMatch("hash");
        HashMismatchReporter.SubsystemStats stats = reporter.getSubsystemSummaries().get("hash");
        assertNotNull(stats);
        assertEquals(3, stats.getMatchCount());
    }

    @Test
    public void reportMatch_parameterlessGoesToDefaultSubsystem() {
        reporter.reportMatch();
        reporter.reportMatch();
        HashMismatchReporter.SubsystemStats stats = reporter.getSubsystemSummaries().get("default");
        assertNotNull(stats);
        assertEquals(2, stats.getMatchCount());
    }

    // ==================== Mismatch counting ====================

    @Test
    public void reportMismatch_incrementsGlobalMismatchCount() {
        reporter.reportMismatch("abc123", "def456");
        reporter.reportMismatch("abc123", "def456");
        assertEquals(2, reporter.getTotalMismatches());
    }

    @Test
    public void reportMismatch_withSubsystem_tracksPerSubsystem() {
        reporter.reportMismatch("hash", "abc123", "def456");
        reporter.reportMismatch("hash", "abc123", "def456");
        HashMismatchReporter.SubsystemStats stats = reporter.getSubsystemSummaries().get("hash");
        assertNotNull(stats);
        assertEquals(2, stats.getMismatchCount());
    }

    @Test
    public void reportMismatch_parameterlessGoesToDefaultSubsystem() {
        reporter.reportMismatch("a", "b");
        HashMismatchReporter.SubsystemStats stats = reporter.getSubsystemSummaries().get("default");
        assertNotNull(stats);
        assertEquals(1, stats.getMismatchCount());
    }

    // ==================== Error counting ====================

    @Test
    public void reportError_incrementsGlobalErrorCount() {
        reporter.reportError();
        reporter.reportError();
        assertEquals(2, reporter.getTotalErrors());
    }

    @Test
    public void reportError_withSubsystem_tracksPerSubsystem() {
        reporter.reportError("hash");
        reporter.reportError("hash");
        HashMismatchReporter.SubsystemStats stats = reporter.getSubsystemSummaries().get("hash");
        assertNotNull(stats);
        assertEquals(2, stats.getErrorCount());
    }

    @Test
    public void reportError_parameterlessGoesToDefaultSubsystem() {
        reporter.reportError();
        HashMismatchReporter.SubsystemStats stats = reporter.getSubsystemSummaries().get("default");
        assertNotNull(stats);
        assertEquals(1, stats.getErrorCount());
    }

    // ==================== Total counts ====================

    @Test
    public void totalChecks_aggregatesAllCategories() {
        reporter.reportMatch("hash");
        reporter.reportMatch("hash");
        reporter.reportMismatch("hash", "a", "b");
        reporter.reportError("hash");
        assertEquals(4, reporter.getTotalChecks());
    }

    @Test
    public void subsystemStats_total_aggregatesAllCategories() {
        reporter.reportMatch("hash");
        reporter.reportMatch("hash");
        reporter.reportMismatch("hash", "a", "b");
        reporter.reportError("hash");
        HashMismatchReporter.SubsystemStats stats = reporter.getSubsystemSummaries().get("hash");
        assertEquals(4, stats.getTotal());
    }

    @Test
    public void totalChecks_zeroWhenEmpty() {
        assertEquals(0, reporter.getTotalChecks());
        assertEquals(0, reporter.getTotalMatches());
        assertEquals(0, reporter.getTotalMismatches());
        assertEquals(0, reporter.getTotalErrors());
    }

    // ==================== Mismatch rate calculation ====================

    @Test
    public void overallMismatchRate_oneMismatchInHundred() {
        for (int i = 0; i < 99; i++) reporter.reportMatch("hash");
        reporter.reportMismatch("hash", "a", "b");
        assertEquals(0.01, reporter.getOverallMismatchRate(), 0.0001);
    }

    @Test
    public void overallMismatchRate_zeroWhenEmpty() {
        assertEquals(0.0, reporter.getOverallMismatchRate(), 0.0);
    }

    @Test
    public void overallMismatchRate_allMatches() {
        for (int i = 0; i < 100; i++) reporter.reportMatch("hash");
        assertEquals(0.0, reporter.getOverallMismatchRate(), 0.0);
    }

    @Test
    public void subsystemMismatchRate_oneMismatchInHundred() {
        for (int i = 0; i < 99; i++) reporter.reportMatch("hash");
        reporter.reportMismatch("hash", "a", "b");
        HashMismatchReporter.SubsystemStats stats = reporter.getSubsystemSummaries().get("hash");
        assertEquals(0.01, stats.getMismatchRate(), 0.0001);
    }

    @Test
    public void subsystemMismatchRate_zeroWhenEmpty() {
        HashMismatchReporter.SubsystemStats empty = new HashMismatchReporter.SubsystemStats();
        assertEquals(0.0, empty.getMismatchRate(), 0.0);
    }

    @Test
    public void getMismatchRate_bySubsystem_delegates() {
        for (int i = 0; i < 90; i++) reporter.reportMatch("parser");
        reporter.reportMismatch("parser", "x", "y");
        assertEquals(10.0 / 91.0, reporter.getMismatchRate("parser"), 0.0001);
    }

    @Test
    public void getMismatchRate_unknownSubsystem_returnsZero() {
        assertEquals(0.0, reporter.getMismatchRate("nonexistent"), 0.0);
    }

    // ==================== Tolerance threshold (1%) ====================

    @Test
    public void isWithinTolerance_empty_isVacuouslyTrue() {
        assertTrue(reporter.isWithinTolerance());
    }

    @Test
    public void isWithinTolerance_allMatches() {
        for (int i = 0; i < 1000; i++) reporter.reportMatch("hash");
        assertTrue(reporter.isWithinTolerance());
    }

    @Test
    public void isWithinTolerance_exactly1Percent() {
        // 99 matches + 1 mismatch = 100 total = 1% mismatch rate
        for (int i = 0; i < 99; i++) reporter.reportMatch("hash");
        reporter.reportMismatch("hash", "a", "b");
        assertTrue(reporter.isWithinTolerance());
    }

    @Test
    public void isWithinTolerance_over1Percent() {
        // 99 matches + 2 mismatches = 101 total = ~1.98% mismatch rate
        for (int i = 0; i < 99; i++) reporter.reportMatch("hash");
        reporter.reportMismatch("hash", "a", "b");
        reporter.reportMismatch("hash", "a", "b");
        assertFalse(reporter.isWithinTolerance());
    }

    @Test
    public void isWithinTolerance_perSubsystem_withinThreshold() {
        for (int i = 0; i < 99; i++) reporter.reportMatch("cache");
        reporter.reportMismatch("cache", "x", "y");
        assertTrue(reporter.isWithinTolerance("cache"));
    }

    @Test
    public void isWithinTolerance_perSubsystem_overThreshold() {
        for (int i = 0; i < 9; i++) reporter.reportMatch("cache");
        reporter.reportMismatch("cache", "x", "y");
        // 1/10 = 10% > 1%
        assertFalse(reporter.isWithinTolerance("cache"));
    }

    @Test
    public void isWithinTolerance_perSubsystem_unknownSubsystem() {
        assertTrue(reporter.isWithinTolerance("nonexistent"));
    }

    @Test
    public void isWithinTolerance_errorsCountTowardTotal() {
        // 99 matches + 1 error = 100 total, 0 mismatches = 0% mismatch rate
        for (int i = 0; i < 99; i++) reporter.reportMatch("hash");
        reporter.reportError("hash");
        assertTrue(reporter.isWithinTolerance());
    }

    // ==================== Per-subsystem tracking ====================

    @Test
    public void subsystemSummaries_multipleSubsystemsTrackedSeparately() {
        reporter.reportMatch("hash");
        reporter.reportMatch("hash");
        reporter.reportMismatch("parser", "3", "2");
        reporter.reportMatch("parser");

        Map<String, HashMismatchReporter.SubsystemStats> summaries = reporter.getSubsystemSummaries();
        assertEquals(2, summaries.size());

        HashMismatchReporter.SubsystemStats hashStats = summaries.get("hash");
        assertEquals(2, hashStats.getMatchCount());
        assertEquals(0, hashStats.getMismatchCount());

        HashMismatchReporter.SubsystemStats parserStats = summaries.get("parser");
        assertEquals(1, parserStats.getMatchCount());
        assertEquals(1, parserStats.getMismatchCount());
    }

    @Test
    public void hasSubsystemMismatches_trueWhenMismatchesExist() {
        reporter.reportMismatch("parser", "3", "2");
        assertTrue(reporter.hasSubsystemMismatches("parser"));
    }

    @Test
    public void hasSubsystemMismatches_falseWhenOnlyMatches() {
        reporter.reportMatch("hash");
        assertFalse(reporter.hasSubsystemMismatches("hash"));
    }

    @Test
    public void hasSubsystemMismatches_falseForUnknownSubsystem() {
        assertFalse(reporter.hasSubsystemMismatches("nonexistent"));
    }

    @Test
    public void subsystemSummaries_returnsUnmodifiableMap() {
        reporter.reportMatch("hash");
        Map<String, HashMismatchReporter.SubsystemStats> summaries = reporter.getSubsystemSummaries();
        try {
            summaries.put("injected", new HashMismatchReporter.SubsystemStats());
            fail("Expected UnsupportedOperationException");
        } catch (UnsupportedOperationException expected) {
            // expected
        }
    }

    @Test
    public void subsystemSummaries_snapshotIsolation() {
        reporter.reportMatch("hash");
        Map<String, HashMismatchReporter.SubsystemStats> snapshot = reporter.getSubsystemSummaries();
        assertEquals(1, snapshot.size());

        // Adding more data after the snapshot does not affect the snapshot keys
        reporter.reportMatch("parser");
        assertEquals(1, snapshot.size());
        assertEquals(2, reporter.getSubsystemSummaries().size());
    }

    // ==================== Reporter name constructor ====================

    @Test
    public void customReporterName_accepted() {
        HashMismatchReporter custom = new HashMismatchReporter(NOOP_FACTORY, "CustomReporter");
        custom.reportMatch("hash");
        assertEquals(1, custom.getTotalMatches());
        // logSummary should not throw with custom name
        custom.logSummary();
    }

    // ==================== logSummary ====================

    @Test
    public void logSummary_doesNotThrowWhenEmpty() {
        reporter.logSummary();
    }

    @Test
    public void logSummary_doesNotThrowWithMixedData() {
        for (int i = 0; i < 100; i++) reporter.reportMatch("hash");
        reporter.reportMismatch("hash", "a", "b");
        reporter.reportMatch("parser");
        reporter.reportError("exec");
        // Should not throw
        reporter.logSummary();
    }

    @Test
    public void logSummary_doesNotThrowWhenExceedsTolerance() {
        for (int i = 0; i < 50; i++) reporter.reportMatch("hash");
        for (int i = 0; i < 2; i++) reporter.reportMismatch("hash", "x", "y");
        reporter.logSummary();
    }

    // ==================== SubsystemStats toString ====================

    @Test
    public void subsystemStats_toString_containsUsefulInfo() {
        HashMismatchReporter.SubsystemStats stats = new HashMismatchReporter.SubsystemStats();
        stats.recordMatch();
        stats.recordMatch();
        stats.recordMismatch();
        String str = stats.toString();
        assertTrue(str.contains("matches=2"));
        assertTrue(str.contains("mismatches=1"));
    }

    // ==================== Thread safety (smoke test) ====================

    @Test
    public void concurrentReports_doNotLoseUpdates() throws InterruptedException {
        int threadCount = 8;
        int opsPerThread = 1000;
        Thread[] threads = new Thread[threadCount];

        for (int t = 0; t < threadCount; t++) {
            final String subsystem = "sub" + (t % 4);
            threads[t] = new Thread(() -> {
                for (int i = 0; i < opsPerThread; i++) {
                    reporter.reportMatch(subsystem);
                }
            });
        }

        for (Thread t : threads) t.start();
        for (Thread t : threads) t.join();

        // Each thread did 1000 match reports, 8 threads total
        assertEquals(threadCount * opsPerThread, reporter.getTotalMatches());

        // 4 subsystems, each receiving 2000 matches (2 threads * 1000 ops)
        Map<String, HashMismatchReporter.SubsystemStats> summaries = reporter.getSubsystemSummaries();
        assertEquals(4, summaries.size());
        for (HashMismatchReporter.SubsystemStats stats : summaries.values()) {
            assertEquals(2000, stats.getMatchCount());
        }
    }

    // ==================== MAX_MISMATCH_RATE constant ====================

    @Test
    public void maxMismatchRate_isOnePercent() {
        assertEquals(0.01, HashMismatchReporter.MAX_MISMATCH_RATE, 0.0);
    }
}
