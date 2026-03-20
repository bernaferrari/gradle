package org.gradle.internal.rustbridge.shadow;

import java.util.Collections;
import java.util.List;

/**
 * Immutable summary of shadow mode comparison results.
 */
public class MismatchSummary {
    private final int matchCount;
    private final int mismatchCount;
    private final int rustErrorCount;
    private final int javaErrorCount;
    private final List<String> mismatchPaths;

    public MismatchSummary(int matchCount, int mismatchCount, int rustErrorCount,
                           int javaErrorCount, List<String> mismatchPaths) {
        this.matchCount = matchCount;
        this.mismatchCount = mismatchCount;
        this.rustErrorCount = rustErrorCount;
        this.javaErrorCount = javaErrorCount;
        this.mismatchPaths = Collections.unmodifiableList(mismatchPaths);
    }

    public int getMatchCount() {
        return matchCount;
    }

    public int getMismatchCount() {
        return mismatchCount;
    }

    public int getRustErrorCount() {
        return rustErrorCount;
    }

    public int getJavaErrorCount() {
        return javaErrorCount;
    }

    public List<String> getMismatchPaths() {
        return mismatchPaths;
    }

    public boolean hasMismatches() {
        return mismatchCount > 0;
    }

    public int getTotalComparisons() {
        return matchCount + mismatchCount + rustErrorCount + javaErrorCount;
    }

    @Override
    public String toString() {
        return String.format(
            "ShadowMode: %d matches, %d mismatches, %d rust errors, %d java errors (total: %d)",
            matchCount, mismatchCount, rustErrorCount, javaErrorCount, getTotalComparisons()
        );
    }
}
