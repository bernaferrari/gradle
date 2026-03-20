package org.gradle.internal.rustbridge.history;

import org.gradle.api.logging.Logging;
import org.gradle.internal.execution.history.AfterExecutionState;
import org.gradle.internal.execution.history.ExecutionHistoryStore;
import org.gradle.internal.execution.history.PreviousExecutionState;
import org.gradle.internal.serialize.Decoder;
import org.gradle.internal.serialize.Encoder;
import org.gradle.internal.serialize.Serializer;
import org.slf4j.Logger;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.util.Optional;

/**
 * An {@link ExecutionHistoryStore} that writes to both Java and Rust stores.
 *
 * <p>Reads always come from the Java store (authoritative). Writes go to both stores.
 * This allows the Rust store to accumulate data for validation without affecting
 * build correctness.</p>
 *
 * <p>Once the Rust store is validated, this can be replaced with a Rust-only store
 * that reads and writes exclusively from Rust.</p>
 */
public class ShadowingExecutionHistoryStore implements ExecutionHistoryStore {

    private static final Logger LOGGER = Logging.getLogger(ShadowingExecutionHistoryStore.class);

    private final ExecutionHistoryStore javaDelegate;
    private final RustExecutionHistoryClient rustClient;
    private final ExecutionHistorySerializer serializer;
    private long storeCount = 0;
    private long rustErrorCount = 0;

    public ShadowingExecutionHistoryStore(
        ExecutionHistoryStore javaDelegate,
        RustExecutionHistoryClient rustClient,
        ExecutionHistorySerializer serializer
    ) {
        this.javaDelegate = javaDelegate;
        this.rustClient = rustClient;
        this.serializer = serializer;
    }

    @Override
    public Optional<PreviousExecutionState> load(String key) {
        // Always read from Java store in shadow mode
        return javaDelegate.load(key);
    }

    @Override
    public void store(String key, AfterExecutionState executionState) {
        // Store in Java (authoritative)
        javaDelegate.store(key, executionState);

        // Shadow: also store in Rust
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
            LOGGER.debug("[substrate:history] shadow store failed for {}: {}", key, e.getMessage());
        }
    }

    @Override
    public void remove(String key) {
        javaDelegate.remove(key);

        try {
            rustClient.remove(key);
        } catch (Exception e) {
            LOGGER.debug("[substrate:history] shadow remove failed for {}: {}", key, e.getMessage());
        }
    }

    /**
     * Get shadow statistics for logging.
     */
    public ShadowStats getStats() {
        return new ShadowStats(storeCount, rustErrorCount);
    }

    public static class ShadowStats {
        private final long stores;
        private final long errors;

        private ShadowStats(long stores, long errors) {
            this.stores = stores;
            this.errors = errors;
        }

        public long getStores() { return stores; }
        public long getErrors() { return errors; }
        public double getErrorRate() { return stores == 0 ? 0 : (double) errors / stores; }

        @Override
        public String toString() {
            return String.format("stores=%d, rustErrors=%d, errorRate=%.2f%%", stores, errors, getErrorRate() * 100);
        }
    }

    /**
     * Serializes {@link AfterExecutionState} to bytes for transport to Rust.
     *
     * <p>Uses a simple binary format that captures the essential fields:
     * cache key, implementation class, input properties, input file fingerprints,
     * output files, and success status.</p>
     */
    public interface ExecutionHistorySerializer {
        byte[] serialize(AfterExecutionState state);
    }
}
