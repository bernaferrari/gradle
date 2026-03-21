package org.gradle.internal.rustbridge.snapshot;

import org.gradle.api.logging.Logging;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * A value snapshotter that runs both Java and Rust implementations in parallel,
 * compares composite hashes, and always returns the Java result for correctness.
 *
 * <p>In shadow mode, this validates the Rust ValueSnapshotService against the
 * Java DefaultValueSnapshotter. Once validated, this can be replaced with
 * a Rust-only implementation.</p>
 */
public class ShadowingValueSnapshotter {

    private static final Logger LOGGER = Logging.getLogger(ShadowingValueSnapshotter.class);

    private final ValueSnapshotterDelegate javaDelegate;
    private final RustValueSnapshotClient rustClient;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public ShadowingValueSnapshotter(
        ValueSnapshotterDelegate javaDelegate,
        RustValueSnapshotClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.javaDelegate = javaDelegate;
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    /**
     * Snapshot all input properties using the Java delegate, and compare with Rust in shadow mode.
     *
     * @param properties map of property name to Java value
     * @param implementationFingerprint fingerprint of the task implementation
     * @return the Java-computed snapshot hash (authoritative)
     */
    public byte[] snapshot(Map<String, Object> properties, String implementationFingerprint) {
        // Authoritative mode: use Rust directly, fall back to Java on failure
        if (authoritative && rustClient != null && !properties.isEmpty()) {
            try {
                RustValueSnapshotClient.SnapshotResult rustResult = rustClient.snapshotValues(properties, implementationFingerprint);
                if (rustResult.isSuccess()) {
                    LOGGER.debug("[substrate:snapshot] authoritative: using Rust hash");
                    return rustResult.getCompositeHash();
                }
            } catch (Exception e) {
                LOGGER.debug("[substrate:snapshot] authoritative Rust failed, falling back to Java", e);
            }
        }

        // Always use Java result for correctness
        byte[] javaHash = javaDelegate.snapshot(properties);

        // Shadow: also snapshot via Rust and compare
        try {
            shadowSnapshot(properties, implementationFingerprint, javaHash);
        } catch (Exception e) {
            LOGGER.debug("[substrate:snapshot] shadow comparison failed", e);
        }

        return javaHash;
    }

    private void shadowSnapshot(
        Map<String, Object> properties,
        String implementationFingerprint,
        byte[] javaHash
    ) {
        if (rustClient == null || properties.isEmpty()) {
            return;
        }

        RustValueSnapshotClient.SnapshotResult rustResult =
            rustClient.snapshotValues(properties, implementationFingerprint);

        if (rustResult.isSuccess()) {
            byte[] rustHash = rustResult.getCompositeHash();

            if (javaHash.length == rustHash.length) {
                boolean match = true;
                for (int i = 0; i < javaHash.length; i++) {
                    if (javaHash[i] != rustHash[i]) {
                        match = false;
                        break;
                    }
                }
                if (match) {
                    mismatchReporter.reportMatch();
                    LOGGER.debug("[substrate:snapshot] shadow OK: {} properties matched",
                        rustResult.getResults().size());
                    return;
                }
            }

            mismatchReporter.reportMismatch(
                "value-snapshot",
                HashCode.fromBytes(javaHash),
                HashCode.fromBytes(rustHash)
            );
        } else {
            mismatchReporter.reportRustError("value-snapshot", new RuntimeException(rustResult.getErrorMessage()));
            LOGGER.debug("[substrate:snapshot] Rust snapshot returned error: {}",
                rustResult.getErrorMessage());
        }
    }

    /**
     * Delegate interface for the Java value snapshot implementation.
     */
    public interface ValueSnapshotterDelegate {
        byte[] snapshot(Map<String, Object> properties);
    }
}
