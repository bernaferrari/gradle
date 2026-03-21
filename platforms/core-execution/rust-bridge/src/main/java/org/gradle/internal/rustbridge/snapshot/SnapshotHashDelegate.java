package org.gradle.internal.rustbridge.snapshot;

import org.gradle.internal.hash.Hasher;
import org.gradle.internal.hash.Hashing;
import org.gradle.internal.snapshot.ValueSnapshot;
import org.gradle.internal.snapshot.ValueSnapshotter;

import java.nio.charset.StandardCharsets;
import java.util.Map;
import java.util.TreeMap;

/**
 * {@link ShadowingValueSnapshotter.ValueSnapshotterDelegate} implementation that uses
 * Java's {@link ValueSnapshotter} to snapshot each property, then computes a composite
 * MD5 hash from all individual snapshot hashes (sorted by property name).
 *
 * <p>Note: Java and Rust use different serialization formats, so composite hashes will
 * differ in shadow mode. This validates plumbing works and tracks error rates.</p>
 */
public class SnapshotHashDelegate implements ShadowingValueSnapshotter.ValueSnapshotterDelegate {

    private static final byte[] SEPARATOR = new byte[]{0};

    private final ValueSnapshotter valueSnapshotter;

    public SnapshotHashDelegate(ValueSnapshotter valueSnapshotter) {
        this.valueSnapshotter = valueSnapshotter;
    }

    @Override
    public byte[] snapshot(Map<String, Object> properties) {
        Hasher hasher = Hashing.md5().newHasher();
        TreeMap<String, Object> sorted = new TreeMap<>(properties);
        for (Map.Entry<String, Object> entry : sorted.entrySet()) {
            hasher.putBytes(entry.getKey().getBytes(StandardCharsets.UTF_8));
            hasher.putBytes(SEPARATOR);
            ValueSnapshot vs = valueSnapshotter.snapshot(entry.getValue());
            vs.appendToHasher(hasher);
        }
        return hasher.hash().toByteArray();
    }
}
