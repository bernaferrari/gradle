package org.gradle.internal.rustbridge.configcache;

import gradle.substrate.v1.CleanConfigCacheRequest;
import gradle.substrate.v1.CleanConfigCacheResponse;
import gradle.substrate.v1.ConfigurationCacheServiceGrpc;
import gradle.substrate.v1.LoadConfigCacheRequest;
import gradle.substrate.v1.LoadConfigCacheResponse;
import gradle.substrate.v1.StoreConfigCacheRequest;
import gradle.substrate.v1.StoreConfigCacheResponse;
import gradle.substrate.v1.ValidateConfigRequest;
import gradle.substrate.v1.ValidateConfigResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust configuration cache service.
 * Stores, loads, validates, and cleans configuration cache entries via gRPC.
 */
public class RustConfigCacheClient {

    private static final Logger LOGGER = Logging.getLogger(RustConfigCacheClient.class);

    private final SubstrateClient client;

    public RustConfigCacheClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of loading a configuration cache entry.
     */
    public static class CacheLoadResult {
        private final boolean found;
        private final byte[] serializedConfig;
        private final long entryCount;
        private final long timestampMs;

        private CacheLoadResult(boolean found, byte[] serializedConfig, long entryCount, long timestampMs) {
            this.found = found;
            this.serializedConfig = serializedConfig;
            this.entryCount = entryCount;
            this.timestampMs = timestampMs;
        }

        public boolean isFound() { return found; }
        public byte[] getSerializedConfig() { return serializedConfig; }
        public long getEntryCount() { return entryCount; }
        public long getTimestampMs() { return timestampMs; }
    }

    /**
     * Result of validating a configuration cache entry.
     */
    public static class ValidationResult {
        private final boolean valid;
        private final String reason;

        private ValidationResult(boolean valid, String reason) {
            this.valid = valid;
            this.reason = reason;
        }

        public boolean isValid() { return valid; }
        public String getReason() { return reason; }
    }

    /**
     * Store a configuration cache entry.
     */
    public boolean storeConfigCache(String cacheKey, byte[] serializedConfig,
                                     long entryCount, List<String> inputHashes) {
        try {
            return storeConfigCacheStrict(cacheKey, serializedConfig, entryCount, inputHashes);
        } catch (Exception e) {
            LOGGER.debug("[substrate:config-cache] store failed for {}", cacheKey, e);
            return false;
        }
    }

    /**
     * Store a configuration cache entry.
     *
     * @throws RuntimeException when the Rust substrate is unavailable or the RPC fails.
     */
    public boolean storeConfigCacheStrict(
        String cacheKey,
        byte[] serializedConfig,
        long entryCount,
        List<String> inputHashes
    ) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        StoreConfigCacheResponse response = client.getConfigCacheStub()
            .storeConfigCache(StoreConfigCacheRequest.newBuilder()
                .setCacheKey(cacheKey)
                .setSerializedConfig(com.google.protobuf.ByteString.copyFrom(serializedConfig))
                .setEntryCount(entryCount)
                .addAllInputHashes(inputHashes)
                .setTimestampMs(System.currentTimeMillis())
                .build());

        LOGGER.debug("[substrate:config-cache] stored entry {} in {}ms",
            cacheKey, response.getStorageTimeMs());
        return response.getStored();
    }

    /**
     * Load a configuration cache entry.
     */
    public CacheLoadResult loadConfigCache(String cacheKey) {
        try {
            return loadConfigCacheStrict(cacheKey);
        } catch (Exception e) {
            LOGGER.debug("[substrate:config-cache] load failed for {}", cacheKey, e);
            return new CacheLoadResult(false, new byte[0], 0, 0);
        }
    }

    /**
     * Load a configuration cache entry.
     *
     * @throws RuntimeException when the Rust substrate is unavailable or the RPC fails.
     */
    public CacheLoadResult loadConfigCacheStrict(String cacheKey) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        LoadConfigCacheResponse response = client.getConfigCacheStub()
            .loadConfigCache(LoadConfigCacheRequest.newBuilder()
                .setCacheKey(cacheKey)
                .build());

        LOGGER.debug("[substrate:config-cache] load {} = found:{}",
            cacheKey, response.getFound());
        return new CacheLoadResult(
            response.getFound(),
            response.getSerializedConfig().toByteArray(),
            response.getEntryCount(),
            response.getTimestampMs()
        );
    }

    /**
     * Validate a configuration cache entry against current input hashes.
     */
    public ValidationResult validateConfig(String cacheKey, List<String> inputHashes) {
        try {
            return validateConfigStrict(cacheKey, inputHashes);
        } catch (Exception e) {
            LOGGER.debug("[substrate:config-cache] validate failed for {}", cacheKey, e);
            return new ValidationResult(false, e.getMessage());
        }
    }

    /**
     * Validate a configuration cache entry against current input hashes.
     *
     * @throws RuntimeException when the Rust substrate is unavailable or the RPC fails.
     */
    public ValidationResult validateConfigStrict(String cacheKey, List<String> inputHashes) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        ValidateConfigResponse response = client.getConfigCacheStub()
            .validateConfig(ValidateConfigRequest.newBuilder()
                .setCacheKey(cacheKey)
                .addAllInputHashes(inputHashes)
                .build());

        return new ValidationResult(response.getValid(), response.getReason());
    }

    /**
     * Clean stale configuration cache entries.
     */
    public CleanConfigCacheResponse cleanConfigCache(long maxAgeMs, int maxEntries) {
        if (client.isNoop()) {
            return CleanConfigCacheResponse.getDefaultInstance();
        }

        try {
            CleanConfigCacheResponse response = client.getConfigCacheStub()
                .cleanConfigCache(CleanConfigCacheRequest.newBuilder()
                    .setMaxAgeMs(maxAgeMs)
                    .setMaxEntries(maxEntries)
                    .build());

            LOGGER.debug("[substrate:config-cache] cleaned {} entries ({} bytes)",
                response.getEntriesRemoved(), response.getSpaceRecoveredBytes());
            return response;
        } catch (Exception e) {
            LOGGER.debug("[substrate:config-cache] clean failed", e);
            return CleanConfigCacheResponse.getDefaultInstance();
        }
    }
}
