package org.gradle.internal.rustbridge.hash;

import org.gradle.internal.hash.FileHasher;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;

import java.io.File;

/**
 * A {@link FileHasher} that runs both Java and Rust implementations in parallel,
 * compares results, and can use Rust results authoritatively.
 *
 * <p>In shadow mode, validates the Rust implementation against the known-good Java one
 * and always returns the Java result.</p>
 *
 * <p>In authoritative mode, uses the Rust result as primary with Java fallback.</p>
 */
public class ShadowingFileHasher implements FileHasher {

    private final FileHasher javaDelegate;
    private final RustGrpcFileHasher rustDelegate;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public ShadowingFileHasher(
        FileHasher javaDelegate,
        RustGrpcFileHasher rustDelegate,
        HashMismatchReporter mismatchReporter
    ) {
        this(javaDelegate, rustDelegate, mismatchReporter, false);
    }

    public ShadowingFileHasher(
        FileHasher javaDelegate,
        RustGrpcFileHasher rustDelegate,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.javaDelegate = javaDelegate;
        this.rustDelegate = rustDelegate;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    public boolean isAuthoritative() {
        return authoritative;
    }

    @Override
    public HashCode hash(File file) {
        if (authoritative) {
            return hashAuthoritative(file);
        }
        return hashShadow(file);
    }

    @Override
    public HashCode hash(File file, long length, long lastModified) {
        if (authoritative) {
            return hashAuthoritative(file, length, lastModified);
        }
        return hashShadow(file, length, lastModified);
    }

    private HashCode hashAuthoritative(File file) {
        try {
            HashCode rustHash = rustDelegate.hash(file);
            // Validate against Java for monitoring
            HashCode javaHash = javaDelegate.hash(file);
            if (!javaHash.toString().equals(rustHash.toString())) {
                mismatchReporter.reportMismatch(file.getAbsolutePath(), javaHash, rustHash);
            } else {
                mismatchReporter.reportMatch();
            }
            return rustHash;
        } catch (Exception e) {
            mismatchReporter.reportRustError(file.getAbsolutePath(), e);
            return javaDelegate.hash(file);
        }
    }

    private HashCode hashAuthoritative(File file, long length, long lastModified) {
        try {
            HashCode rustHash = rustDelegate.hash(file, length, lastModified);
            HashCode javaHash = javaDelegate.hash(file, length, lastModified);
            if (!javaHash.toString().equals(rustHash.toString())) {
                mismatchReporter.reportMismatch(file.getAbsolutePath(), javaHash, rustHash);
            } else {
                mismatchReporter.reportMatch();
            }
            return rustHash;
        } catch (Exception e) {
            mismatchReporter.reportRustError(file.getAbsolutePath(), e);
            return javaDelegate.hash(file, length, lastModified);
        }
    }

    private HashCode hashShadow(File file) {
        HashCode javaHash = javaDelegate.hash(file);
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
        return javaHash;
    }

    private HashCode hashShadow(File file, long length, long lastModified) {
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
