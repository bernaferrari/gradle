package org.gradle.internal.rustbridge.history;

import gradle.substrate.v1.GetHistoryStatsRequest;
import gradle.substrate.v1.GetHistoryStatsResponse;
import gradle.substrate.v1.LoadHistoryRequest;
import gradle.substrate.v1.LoadHistoryResponse;
import gradle.substrate.v1.RemoveHistoryRequest;
import gradle.substrate.v1.RemoveHistoryResponse;
import gradle.substrate.v1.StoreHistoryRequest;
import gradle.substrate.v1.StoreHistoryResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

/**
 * Client for the Rust execution history service.
 * Stores and retrieves execution history entries via gRPC.
 *
 * <p>The Rust implementation uses bincode serialization with disk persistence,
 * which is significantly faster than Java's IndexedCache for large histories.</p>
 */
public class RustExecutionHistoryClient {

    private static final Logger LOGGER = Logging.getLogger(RustExecutionHistoryClient.class);

    private final SubstrateClient client;

    public RustExecutionHistoryClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * A serialized execution history entry returned from Rust.
     */
    public static class HistoryEntry {
        private final String key;
        private final byte[] serializedState;
        private final long timestampMs;
        private final int entrySize;

        private HistoryEntry(String key, byte[] serializedState, long timestampMs, int entrySize) {
            this.key = key;
            this.serializedState = serializedState;
            this.timestampMs = timestampMs;
            this.entrySize = entrySize;
        }

        public String getKey() { return key; }
        public byte[] getSerializedState() { return serializedState; }
        public long getTimestampMs() { return timestampMs; }
        public int getEntrySize() { return entrySize; }
    }

    /**
     * Store an execution history entry.
     *
     * @param key the work identity key
     * @param serializedState the serialized PreviousExecutionState bytes
     * @return true if stored successfully
     */
    public boolean store(String key, byte[] serializedState) {
        if (client.isNoop()) {
            return false;
        }

        try {
            StoreHistoryRequest request = StoreHistoryRequest.newBuilder()
                .setWorkIdentity(key)
                .setState(com.google.protobuf.ByteString.copyFrom(serializedState))
                .build();

            StoreHistoryResponse response = client.getExecutionHistoryStub()
                .storeHistory(request);

            if (response.getSuccess()) {
                LOGGER.debug("[substrate:history] stored {} ({} bytes)", key, serializedState.length);
            } else {
                LOGGER.debug("[substrate:history] store failed for {}", key);
            }
            return response.getSuccess();
        } catch (Exception e) {
            LOGGER.debug("[substrate:history] store failed for {}: {}", key, e.getMessage());
            return false;
        }
    }

    /**
     * Load an execution history entry.
     *
     * @param key the work identity key
     * @return the history entry, or null if not found
     */
    public HistoryEntry load(String key) {
        if (client.isNoop()) {
            return null;
        }

        try {
            LoadHistoryRequest request = LoadHistoryRequest.newBuilder()
                .setWorkIdentity(key)
                .build();

            LoadHistoryResponse response = client.getExecutionHistoryStub()
                .loadHistory(request);

            if (response.getFound()) {
                LOGGER.debug("[substrate:history] loaded {} ({} bytes, ts={})",
                    key, response.getState().size(), response.getTimestampMs());
                return new HistoryEntry(
                    key,
                    response.getState().toByteArray(),
                    response.getTimestampMs(),
                    response.getState().size()
                );
            } else {
                LOGGER.debug("[substrate:history] not found: {}", key);
                return null;
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:history] load failed for {}: {}", key, e.getMessage());
            return null;
        }
    }

    /**
     * Remove an execution history entry.
     *
     * @param key the work identity key
     * @return true if removed successfully
     */
    public boolean remove(String key) {
        if (client.isNoop()) {
            return false;
        }

        try {
            RemoveHistoryRequest request = RemoveHistoryRequest.newBuilder()
                .setWorkIdentity(key)
                .build();

            RemoveHistoryResponse response = client.getExecutionHistoryStub()
                .removeHistory(request);

            if (response.getSuccess()) {
                LOGGER.debug("[substrate:history] removed {}", key);
            }
            return response.getSuccess();
        } catch (Exception e) {
            LOGGER.debug("[substrate:history] remove failed for {}: {}", key, e.getMessage());
            return false;
        }
    }

    /**
     * Get statistics about the history store.
     *
     * @return stats response, or default if the client is a noop
     */
    public GetHistoryStatsResponse getStats() {
        if (client.isNoop()) {
            return GetHistoryStatsResponse.getDefaultInstance();
        }

        try {
            return client.getExecutionHistoryStub()
                .getHistoryStats(GetHistoryStatsRequest.newBuilder().build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:history] getStats failed: {}", e.getMessage());
            return GetHistoryStatsResponse.getDefaultInstance();
        }
    }
}
