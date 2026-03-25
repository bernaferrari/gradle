package org.gradle.internal.rustbridge.cache;

import gradle.substrate.v1.ComputeCacheKeyRequest;
import gradle.substrate.v1.ComputeCacheKeyResponse;
import gradle.substrate.v1.ProbeCacheRequest;
import gradle.substrate.v1.ProbeCacheResponse;
import gradle.substrate.v1.StoreOutputsRequest;
import gradle.substrate.v1.StoreOutputsResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;
import java.util.Map;

/**
 * Client for the Rust build cache orchestration service.
 * Computes cache keys, probes cache availability, and marks outputs as stored.
 */
public class BuildCacheOrchestrationClient {

    private static final Logger LOGGER = Logging.getLogger(BuildCacheOrchestrationClient.class);

    private final SubstrateClient client;

    public BuildCacheOrchestrationClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Compute a deterministic cache key for the given work identity and input fingerprints.
     */
    public CacheKeyResult computeCacheKey(
        String workIdentity,
        String implHash,
        Map<String, String> inputPropertyHashes,
        Map<String, String> inputFileHashes,
        List<String> outputNames
    ) {
        try {
            return computeCacheKeyStrict(workIdentity, implHash, inputPropertyHashes, inputFileHashes, outputNames);
        } catch (Exception e) {
            LOGGER.debug("[substrate:cache-orch] computeCacheKey failed", e);
            return CacheKeyResult.error("gRPC error: " + e.getMessage());
        }
    }

    /**
     * Compute a deterministic cache key for the given work identity and input fingerprints.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public CacheKeyResult computeCacheKeyStrict(
        String workIdentity,
        String implHash,
        Map<String, String> inputPropertyHashes,
        Map<String, String> inputFileHashes,
        List<String> outputNames
    ) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        ComputeCacheKeyResponse response = client.getCacheOrchestrationStub()
            .computeCacheKey(ComputeCacheKeyRequest.newBuilder()
                .setWorkIdentity(workIdentity)
                .setImplementationHash(implHash)
                .putAllInputPropertyHashes(inputPropertyHashes)
                .putAllInputFileHashes(inputFileHashes)
                .addAllOutputPropertyNames(outputNames)
                .build());

        return new CacheKeyResult(
            response.getCacheKey().toByteArray(),
            response.getCacheKeyString(),
            true,
            ""
        );
    }

    /**
     * Check if a cache entry exists for the given key without loading it.
     */
    public ProbeResult probeCache(byte[] cacheKey) {
        try {
            return probeCacheStrict(cacheKey);
        } catch (Exception e) {
            LOGGER.debug("[substrate:cache-orch] probeCache failed", e);
            return ProbeResult.unavailable("gRPC error: " + e.getMessage());
        }
    }

    /**
     * Check if a cache entry exists for the given key without loading it.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public ProbeResult probeCacheStrict(byte[] cacheKey) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        ProbeCacheResponse response = client.getCacheOrchestrationStub()
            .probeCache(ProbeCacheRequest.newBuilder()
                .setCacheKey(com.google.protobuf.ByteString.copyFrom(cacheKey))
                .build());

        return new ProbeResult(response.getAvailable(), response.getLocation(), true, "");
    }

    /**
     * Mark outputs as stored for the given cache key.
     */
    public boolean storeOutputs(byte[] cacheKey, long executionTimeMs) {
        try {
            return storeOutputsStrict(cacheKey, executionTimeMs);
        } catch (Exception e) {
            LOGGER.debug("[substrate:cache-orch] storeOutputs failed", e);
            return false;
        }
    }

    /**
     * Mark outputs as stored for the given cache key.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public boolean storeOutputsStrict(byte[] cacheKey, long executionTimeMs) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        StoreOutputsResponse response = client.getCacheOrchestrationStub()
            .storeOutputs(StoreOutputsRequest.newBuilder()
                .setCacheKey(com.google.protobuf.ByteString.copyFrom(cacheKey))
                .setExecutionTimeMs(executionTimeMs)
                .build());

        return response.getSuccess();
    }

    /**
     * Result of computing a cache key.
     */
    public static class CacheKeyResult {
        private final byte[] keyBytes;
        private final String keyString;
        private final boolean success;
        private final String errorMessage;

        private CacheKeyResult(byte[] keyBytes, String keyString, boolean success, String errorMessage) {
            this.keyBytes = keyBytes;
            this.keyString = keyString;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public static CacheKeyResult error(String message) {
            return new CacheKeyResult(new byte[0], "", false, message);
        }

        public byte[] getKeyBytes() { return keyBytes; }
        public String getKeyString() { return keyString; }
        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }
    }

    /**
     * Result of probing the cache.
     */
    public static class ProbeResult {
        private final boolean available;
        private final String location;
        private final boolean success;
        private final String errorMessage;

        private ProbeResult(boolean available, String location, boolean success, String errorMessage) {
            this.available = available;
            this.location = location;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public static ProbeResult unavailable(String reason) {
            return new ProbeResult(false, "", false, reason);
        }

        public boolean isAvailable() { return available; }
        public String getLocation() { return location; }
        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }
    }
}
