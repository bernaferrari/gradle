package org.gradle.internal.rustbridge.history;

import org.gradle.api.logging.Logging;
import org.gradle.internal.execution.history.AfterExecutionState;
import org.gradle.internal.execution.history.ExecutionHistoryStore;
import org.gradle.internal.execution.history.PreviousExecutionState;
import org.gradle.internal.serialize.Decoder;
import org.gradle.internal.serialize.Encoder;
import org.gradle.internal.serialize.Serializer;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.util.Optional;

/**
 * An {@link ExecutionHistoryStore} that delegates to both Java and Rust stores.
 *
 * <p>In shadow mode (authoritative=false): reads come from Java, writes go to both.
 * In authoritative mode (authoritative=true): reads come from Rust first (with Java fallback),
 * writes go to both.</p>
 *
 * <p>Once validated in shadow mode, authoritative mode can be enabled to use the Rust store
 * as the primary, with Java as the fallback for correctness.</p>
 */
public class ShadowingExecutionHistoryStore implements ExecutionHistoryStore {

    private static final Logger LOGGER = Logging.getLogger(ShadowingExecutionHistoryStore.class);

    private final ExecutionHistoryStore javaDelegate;
    private final RustExecutionHistoryClient rustClient;
    private final ExecutionHistorySerializer serializer;
    private final boolean authoritative;
    private long storeCount = 0;
    private long loadCount = 0;
    private long rustHitCount = 0;
    private long rustMissCount = 0;
    private long rustErrorCount = 0;

    public ShadowingExecutionHistoryStore(
        ExecutionHistoryStore javaDelegate,
        RustExecutionHistoryClient rustClient,
        ExecutionHistorySerializer serializer
    ) {
        this(javaDelegate, rustClient, serializer, false);
    }

    public ShadowingExecutionHistoryStore(
        ExecutionHistoryStore javaDelegate,
        RustExecutionHistoryClient rustClient,
        ExecutionHistorySerializer serializer,
        boolean authoritative
    ) {
        this.javaDelegate = javaDelegate;
        this.rustClient = rustClient;
        this.serializer = serializer;
        this.authoritative = authoritative;
    }

    @Override
    public Optional<PreviousExecutionState> load(String key) {
        loadCount++;

        if (authoritative) {
            // Try Rust first in authoritative mode
            try {
                RustExecutionHistoryClient.HistoryEntry rustEntry = rustClient.load(key);
                if (rustEntry != null) {
                    rustHitCount++;
                    LOGGER.debug("[substrate:history] authoritative load HIT from Rust: {}", key);
                    PreviousExecutionState state = serializer.deserialize(rustEntry.getSerializedState());
                    if (state != null) {
                        return Optional.of(state);
                    }
                } else {
                    rustMissCount++;
                }
            } catch (Exception e) {
                rustErrorCount++;
                LOGGER.debug("[substrate:history] authoritative load from Rust failed for {}: {}", key, e.getMessage());
            }
            // Fall through to Java
        }

        // Read from Java (always available as fallback)
        Optional<PreviousExecutionState> result = javaDelegate.load(key);

        // In shadow mode, verify against Rust
        if (!authoritative && result.isPresent()) {
            try {
                RustExecutionHistoryClient.HistoryEntry rustEntry = rustClient.load(key);
                if (rustEntry != null) {
                    rustHitCount++;
                } else {
                    rustMissCount++;
                    LOGGER.debug("[substrate:history] shadow: Rust missing entry that Java has: {}", key);
                }
            } catch (Exception e) {
                LOGGER.debug("[substrate:history] shadow load verify failed for {}: {}", key, e.getMessage());
            }
        }

        return result;
    }

    @Override
    public void store(String key, AfterExecutionState executionState) {
        // Store in Java (always, as fallback)
        javaDelegate.store(key, executionState);

        // Store in Rust (primary in authoritative, shadow otherwise)
        try {
            byte[] serialized = serializer.serialize(executionState);
            boolean success = rustClient.store(key, serialized);
            if (success) {
                storeCount++;
            } else {
                rustErrorCount++;
            }
        } catch (Exception e) {
            rustErrorCount++;
            LOGGER.debug("[substrate:history] store to Rust failed for {}: {}", key, e.getMessage());
        }
    }

    @Override
    public void remove(String key) {
        javaDelegate.remove(key);

        try {
            rustClient.remove(key);
        } catch (Exception e) {
            LOGGER.debug("[substrate:history] remove from Rust failed for {}: {}", key, e.getMessage());
        }
    }

    /**
     * Get shadow statistics for logging.
     */
    public ShadowStats getStats() {
        return new ShadowStats(storeCount, loadCount, rustHitCount, rustMissCount, rustErrorCount);
    }

    public static class ShadowStats {
        private final long stores;
        private final long loads;
        private final long rustHits;
        private final long rustMisses;
        private final long errors;

        private ShadowStats(long stores, long loads, long rustHits, long rustMisses, long errors) {
            this.stores = stores;
            this.loads = loads;
            this.rustHits = rustHits;
            this.rustMisses = rustMisses;
            this.errors = errors;
        }

        public long getStores() { return stores; }
        public long getLoads() { return loads; }
        public long getRustHits() { return rustHits; }
        public long getRustMisses() { return rustMisses; }
        public long getErrors() { return errors; }
        public double getErrorRate() { return stores == 0 ? 0 : (double) errors / stores; }
        public double getHitRate() { return loads == 0 ? 0 : (double) rustHits / loads; }

        @Override
        public String toString() {
            return String.format("stores=%d, loads=%d, rustHits=%d, rustMisses=%d, errors=%d, hitRate=%.1f%%",
                stores, loads, rustHits, rustMisses, errors, getHitRate() * 100);
        }
    }

    /**
     * Serializes {@link AfterExecutionState} to bytes for transport to Rust,
     * and deserializes bytes back to {@link PreviousExecutionState}.
     *
     * <p>Uses a simple binary format that captures the essential fields:
     * cache key, implementation class, input properties, input file fingerprints,
     * output files, and success status.</p>
     */
    public interface ExecutionHistorySerializer {
        byte[] serialize(AfterExecutionState state);

        /**
         * Deserialize a {@link PreviousExecutionState} from bytes returned by Rust.
         *
         * @return the deserialized state, or null if deserialization fails
         */
        @Nullable
        PreviousExecutionState deserialize(byte[] data);
    }
}
