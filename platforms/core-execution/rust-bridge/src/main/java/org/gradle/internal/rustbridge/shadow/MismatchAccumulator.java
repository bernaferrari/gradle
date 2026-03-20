package org.gradle.internal.rustbridge.shadow;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * Thread-safe accumulator for shadow mode comparison results.
 */
public class MismatchAccumulator {

    private final AtomicInteger matchCount = new AtomicInteger();
    private final AtomicInteger mismatchCount = new AtomicInteger();
    private final AtomicInteger rustErrorCount = new AtomicInteger();
    private final AtomicInteger javaErrorCount = new AtomicInteger();
    private final CopyOnWriteArrayList<String> mismatchPaths = new CopyOnWriteArrayList<>();

    public void recordMatch() {
        matchCount.incrementAndGet();
    }

    public void recordMismatch(String path) {
        mismatchCount.incrementAndGet();
        mismatchPaths.add(path);
    }

    public void recordRustError() {
        rustErrorCount.incrementAndGet();
    }

    public void recordJavaError() {
        javaErrorCount.incrementAndGet();
    }

    public MismatchSummary snapshot() {
        return new MismatchSummary(
            matchCount.get(),
            mismatchCount.get(),
            rustErrorCount.get(),
            javaErrorCount.get(),
            new ArrayList<>(mismatchPaths)
        );
    }
}
