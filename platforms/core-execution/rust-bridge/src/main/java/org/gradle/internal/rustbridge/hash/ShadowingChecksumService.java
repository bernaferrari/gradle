package org.gradle.internal.rustbridge.hash;

import org.gradle.internal.hash.ChecksumService;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateException;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;

import java.io.File;

/**
 * Runs Java and Rust checksum services side by side, with optional Rust authority.
 */
public class ShadowingChecksumService implements ChecksumService {

    private final ChecksumService javaDelegate;
    private final ChecksumService rustDelegate;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public ShadowingChecksumService(
        ChecksumService javaDelegate,
        ChecksumService rustDelegate,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.javaDelegate = javaDelegate;
        this.rustDelegate = rustDelegate;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    @Override
    public HashCode md5(File file) {
        return compare(file, "md5");
    }

    @Override
    public HashCode sha1(File file) {
        return compare(file, "sha1");
    }

    @Override
    public HashCode sha256(File file) {
        return compare(file, "sha256");
    }

    @Override
    public HashCode sha512(File file) {
        return compare(file, "sha512");
    }

    @Override
    public HashCode hash(File src, String algorithm) {
        return compare(src, algorithm);
    }

    private HashCode compare(File file, String algorithm) {
        return authoritative
            ? compareAuthoritative(file, algorithm)
            : compareShadow(file, algorithm);
    }

    private HashCode compareShadow(File file, String algorithm) {
        HashCode javaHash = javaDelegate.hash(file, algorithm);
        try {
            HashCode rustHash = rustDelegate.hash(file, algorithm);
            report(algorithm, javaHash, rustHash);
        } catch (Exception e) {
            mismatchReporter.reportRustError(subsystem(algorithm), e);
        }
        return javaHash;
    }

    private HashCode compareAuthoritative(File file, String algorithm) {
        try {
            HashCode rustHash = rustDelegate.hash(file, algorithm);
            HashCode javaHash = javaDelegate.hash(file, algorithm);
            if (!report(algorithm, javaHash, rustHash)) {
                throw new SubstrateException("Authoritative Rust checksum mismatch for " + file.getAbsolutePath() + " using " + algorithm);
            }
            return rustHash;
        } catch (Exception e) {
            if (e instanceof SubstrateException) {
                throw (SubstrateException) e;
            }
            mismatchReporter.reportRustError(subsystem(algorithm), e);
            throw new SubstrateException("Authoritative Rust checksum failed for " + file.getAbsolutePath() + " using " + algorithm, e);
        }
    }

    private boolean report(String algorithm, HashCode javaHash, HashCode rustHash) {
        if (javaHash.equals(rustHash)) {
            mismatchReporter.reportMatch(subsystem(algorithm));
            return true;
        }
        mismatchReporter.reportMismatch(subsystem(algorithm), javaHash, rustHash);
        return false;
    }

    private static String subsystem(String algorithm) {
        return "checksum:" + algorithm;
    }
}
