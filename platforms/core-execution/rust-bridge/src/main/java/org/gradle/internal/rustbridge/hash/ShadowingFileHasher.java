package org.gradle.internal.rustbridge.hash;

import org.gradle.internal.hash.FileHasher;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;

import java.io.File;

/**
 * A {@link FileHasher} that runs both Java and Rust implementations in parallel,
 * compares results, and always returns the Java result for correctness.
 * Used in shadow mode to validate the Rust implementation against the known-good Java one.
 */
public class ShadowingFileHasher implements FileHasher {

    private final FileHasher javaDelegate;
    private final RustGrpcFileHasher rustDelegate;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingFileHasher(
        FileHasher javaDelegate,
        RustGrpcFileHasher rustDelegate,
        HashMismatchReporter mismatchReporter
    ) {
        this.javaDelegate = javaDelegate;
        this.rustDelegate = rustDelegate;
        this.mismatchReporter = mismatchReporter;
    }

    @Override
    public HashCode hash(File file) {
        // Always use Java result for correctness
        HashCode javaHash = javaDelegate.hash(file);

        // Shadow: also compute via Rust and compare
        try {
            HashCode rustHash = rustDelegate.hash(file);
            if (!javaHash.toString().equals(rustHash.toString())) {
                mismatchReporter.reportMismatch(file.getAbsolutePath(), javaHash, rustHash);
            } else {
                mismatchReporter.reportMatch();
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError(file.getAbsolutePath(), e);
        }

        return javaHash;  // Always return Java result in shadow mode
    }

    @Override
    public HashCode hash(File file, long length, long lastModified) {
        HashCode javaHash = javaDelegate.hash(file, length, lastModified);

        try {
            HashCode rustHash = rustDelegate.hash(file, length, lastModified);
            if (!javaHash.toString().equals(rustHash.toString())) {
                mismatchReporter.reportMismatch(file.getAbsolutePath(), javaHash, rustHash);
            } else {
                mismatchReporter.reportMatch();
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError(file.getAbsolutePath(), e);
        }

        return javaHash;
    }
}
