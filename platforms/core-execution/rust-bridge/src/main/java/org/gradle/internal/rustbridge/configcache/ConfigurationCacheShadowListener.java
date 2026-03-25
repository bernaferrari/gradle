package org.gradle.internal.rustbridge.configcache;

import org.gradle.api.logging.Logging;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.Arrays;
import java.util.List;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Shadow listener that compares Gradle's Java configuration cache with the
 * Rust {@link RustConfigCacheService}. Follows the fire-and-forget shadow pattern:
 * Java is always authoritative; Rust results are compared and mismatches reported.
 *
 * <p>Since Gradle's ConfigurationCache does not expose a simple listener interface,
 * this is a utility class that gets called at key store/load/validate points.</p>
 */
public class ConfigurationCacheShadowListener {

    private static final Logger LOGGER = Logging.getLogger(ConfigurationCacheShadowListener.class);

    private final RustConfigCacheClient client;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    private final AtomicLong storeCount = new AtomicLong(0);
    private final AtomicLong loadCount = new AtomicLong(0);
    private final AtomicLong hitCount = new AtomicLong(0);
    private final AtomicLong missCount = new AtomicLong(0);
    private final AtomicLong validateCount = new AtomicLong(0);

    public ConfigurationCacheShadowListener(
        RustConfigCacheClient client,
        HashMismatchReporter mismatchReporter
    ) {
        this(client, mismatchReporter, false);
    }

    public ConfigurationCacheShadowListener(
        RustConfigCacheClient client,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.client = client;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    /**
     * Effective load decision returned by authoritative-or-fallback mode.
     */
    public static class EffectiveLoadResult {
        private final boolean found;
        private final byte[] serializedConfig;
        private final long entryCount;
        private final long timestampMs;
        private final String source;

        private EffectiveLoadResult(
            boolean found,
            byte[] serializedConfig,
            long entryCount,
            long timestampMs,
            String source
        ) {
            this.found = found;
            this.serializedConfig = serializedConfig;
            this.entryCount = entryCount;
            this.timestampMs = timestampMs;
            this.source = source;
        }

        public boolean isFound() {
            return found;
        }

        public byte[] getSerializedConfig() {
            return serializedConfig;
        }

        public long getEntryCount() {
            return entryCount;
        }

        public long getTimestampMs() {
            return timestampMs;
        }

        public String getSource() {
            return source;
        }
    }

    /**
     * Effective validation decision returned by authoritative-or-fallback mode.
     */
    public static class EffectiveValidationResult {
        private final boolean valid;
        private final String reason;
        private final String source;

        private EffectiveValidationResult(boolean valid, String reason, String source) {
            this.valid = valid;
            this.reason = reason;
            this.source = source;
        }

        public boolean isValid() {
            return valid;
        }

        public String getReason() {
            return reason;
        }

        public String getSource() {
            return source;
        }
    }

    /**
     * Shadow a configuration cache store operation.
     * Stores the same entry in Rust and reports whether the store succeeded.
     *
     * @param cacheKey        the configuration cache key
     * @param serializedConfig the Java-serialized configuration bytes
     * @param entryCount      number of entries in the configuration cache
     * @param inputHashes     input hashes used to compute the cache key
     */
    public void shadowStore(
        String cacheKey,
        byte[] serializedConfig,
        long entryCount,
        List<String> inputHashes
    ) {
        storeCount.incrementAndGet();
        shadowStoreInternal(cacheKey, serializedConfig, entryCount, inputHashes);
    }

    /**
     * Attempt Rust store as authoritative path and fall back to Java result on failure.
     *
     * @param javaStored the Java-side fallback outcome for this store operation
     * @return the effective persisted outcome after Rust-first with Java fallback
     */
    public boolean storeAuthoritativeOrFallback(
        String cacheKey,
        byte[] serializedConfig,
        long entryCount,
        List<String> inputHashes,
        boolean javaStored
    ) {
        storeCount.incrementAndGet();
        if (!authoritative) {
            shadowStoreInternal(cacheKey, serializedConfig, entryCount, inputHashes);
            return javaStored;
        }
        try {
            boolean rustStored = client.storeConfigCacheStrict(cacheKey, serializedConfig, entryCount, inputHashes);
            reportStoreComparison(cacheKey, serializedConfig, entryCount, rustStored);
            if (rustStored) {
                return true;
            }
            LOGGER.debug("[substrate:config-cache] authoritative store fallback to Java for key={}", cacheKey);
            return javaStored;
        } catch (Exception e) {
            reportRustError("store", cacheKey, e);
            LOGGER.debug("[substrate:config-cache] authoritative store error for key={}, using Java fallback: {}",
                cacheKey, e.getMessage());
            return javaStored;
        }
    }

    /**
     * Shadow a configuration cache load operation.
     * Loads from Rust and compares the result with the Java result.
     *
     * @param cacheKey        the configuration cache key
     * @param javaConfigBytes the Java-loaded configuration bytes (authoritative)
     * @param javaFound       whether Java found a cache entry
     */
    public void shadowLoad(
        String cacheKey,
        byte[] javaConfigBytes,
        boolean javaFound
    ) {
        loadCount.incrementAndGet();
        shadowLoadInternal(cacheKey, javaConfigBytes, javaFound);
    }

    /**
     * Attempt Rust load as authoritative path and fall back to Java result on failure.
     */
    public EffectiveLoadResult loadAuthoritativeOrFallback(
        String cacheKey,
        byte[] javaConfigBytes,
        boolean javaFound
    ) {
        loadCount.incrementAndGet();
        if (!authoritative) {
            shadowLoadInternal(cacheKey, javaConfigBytes, javaFound);
            return new EffectiveLoadResult(javaFound, javaConfigBytes, 0, 0, "java-shadow");
        }
        try {
            RustConfigCacheClient.CacheLoadResult rustResult = client.loadConfigCacheStrict(cacheKey);
            reportLoadComparison(cacheKey, javaConfigBytes, javaFound, rustResult);

            if (rustResult.isFound()) {
                return new EffectiveLoadResult(
                    true,
                    rustResult.getSerializedConfig(),
                    rustResult.getEntryCount(),
                    rustResult.getTimestampMs(),
                    "rust"
                );
            }
            LOGGER.debug("[substrate:config-cache] authoritative load fallback to Java for key={}", cacheKey);
        } catch (Exception e) {
            reportRustError("load", cacheKey, e);
            LOGGER.debug("[substrate:config-cache] authoritative load error for key={}, using Java fallback: {}",
                cacheKey, e.getMessage());
        }
        return new EffectiveLoadResult(javaFound, javaConfigBytes, 0, 0, "java-fallback");
    }

    /**
     * Shadow a configuration cache validation operation.
     * Validates against Rust and compares the result with the Java validation result.
     *
     * @param cacheKey    the configuration cache key
     * @param inputHashes the current input hashes to validate against
     * @param javaValid   whether Java considers the cache entry valid
     */
    public void shadowValidate(
        String cacheKey,
        List<String> inputHashes,
        boolean javaValid
    ) {
        validateCount.incrementAndGet();
        shadowValidateInternal(cacheKey, inputHashes, javaValid);
    }

    /**
     * Attempt Rust validation as authoritative path and fall back to Java decision on failure.
     */
    public EffectiveValidationResult validateAuthoritativeOrFallback(
        String cacheKey,
        List<String> inputHashes,
        boolean javaValid,
        String javaReason
    ) {
        validateCount.incrementAndGet();
        if (!authoritative) {
            shadowValidateInternal(cacheKey, inputHashes, javaValid);
            return new EffectiveValidationResult(javaValid, javaReason, "java-shadow");
        }

        try {
            RustConfigCacheClient.ValidationResult rustResult = client.validateConfigStrict(cacheKey, inputHashes);
            reportValidateComparison(cacheKey, javaValid, rustResult);
            return new EffectiveValidationResult(rustResult.isValid(), rustResult.getReason(), "rust");
        } catch (Exception e) {
            reportRustError("validate", cacheKey, e);
            LOGGER.debug("[substrate:config-cache] authoritative validate error for key={}, using Java fallback: {}",
                cacheKey, e.getMessage());
            return new EffectiveValidationResult(javaValid, javaReason, "java-fallback");
        }
    }

    /**
     * Attempt Rust validation as authoritative path and fall back to Java decision on failure.
     */
    public EffectiveValidationResult validateAuthoritativeOrFallback(
        String cacheKey,
        List<String> inputHashes,
        boolean javaValid
    ) {
        return validateAuthoritativeOrFallback(cacheKey, inputHashes, javaValid, "");
    }

    private void shadowStoreInternal(
        String cacheKey,
        byte[] serializedConfig,
        long entryCount,
        List<String> inputHashes
    ) {
        try {
            boolean stored = client.storeConfigCache(cacheKey, serializedConfig, entryCount, inputHashes);
            reportStoreComparison(cacheKey, serializedConfig, entryCount, stored);
        } catch (Exception e) {
            reportRustError("store", cacheKey, e);
            LOGGER.debug("[substrate:config-cache] shadow store error for key={}: {}",
                cacheKey, e.getMessage());
        }
    }

    private void shadowLoadInternal(
        String cacheKey,
        byte[] javaConfigBytes,
        boolean javaFound
    ) {
        try {
            RustConfigCacheClient.CacheLoadResult rustResult = client.loadConfigCache(cacheKey);
            reportLoadComparison(cacheKey, javaConfigBytes, javaFound, rustResult);
        } catch (Exception e) {
            reportRustError("load", cacheKey, e);
            LOGGER.debug("[substrate:config-cache] shadow load error for key={}: {}",
                cacheKey, e.getMessage());
        }
    }

    private void shadowValidateInternal(
        String cacheKey,
        List<String> inputHashes,
        boolean javaValid
    ) {
        try {
            RustConfigCacheClient.ValidationResult rustResult = client.validateConfig(cacheKey, inputHashes);
            reportValidateComparison(cacheKey, javaValid, rustResult);
        } catch (Exception e) {
            reportRustError("validate", cacheKey, e);
            LOGGER.debug("[substrate:config-cache] shadow validate error for key={}: {}",
                cacheKey, e.getMessage());
        }
    }

    private void reportStoreComparison(
        String cacheKey,
        byte[] serializedConfig,
        long entryCount,
        boolean rustStored
    ) {
        if (rustStored) {
            mismatchReporter.reportMatch();
            LOGGER.debug("[substrate:config-cache] shadow store OK: key={}, {} entries, {} bytes",
                cacheKey, entryCount, serializedConfig.length);
        } else {
            mismatchReporter.reportMismatch(
                "config-cache:store:" + cacheKey,
                HashCode.fromBytes(serializedConfig),
                new byte[0]
            );
            LOGGER.debug("[substrate:config-cache] shadow store FAILED for key={}", cacheKey);
        }
    }

    private void reportLoadComparison(
        String cacheKey,
        byte[] javaConfigBytes,
        boolean javaFound,
        RustConfigCacheClient.CacheLoadResult rustResult
    ) {
        if (javaFound && rustResult.isFound()) {
            // Both found -- compare bytes
            if (Arrays.equals(javaConfigBytes, rustResult.getSerializedConfig())) {
                hitCount.incrementAndGet();
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:config-cache] shadow load MATCH: key={}, {} bytes",
                    cacheKey, javaConfigBytes.length);
            } else {
                hitCount.incrementAndGet();
                mismatchReporter.reportMismatch(
                    "config-cache:load:" + cacheKey,
                    HashCode.fromBytes(javaConfigBytes),
                    rustResult.getSerializedConfig()
                );
                LOGGER.debug("[substrate:config-cache] shadow load MISMATCH: key={}, java={} bytes, rust={} bytes",
                    cacheKey, javaConfigBytes.length, rustResult.getSerializedConfig().length);
            }
        } else if (javaFound && !rustResult.isFound()) {
            // Java found but Rust did not
            missCount.incrementAndGet();
            mismatchReporter.reportMismatch(
                "config-cache:load:" + cacheKey,
                HashCode.fromBytes(javaConfigBytes),
                new byte[0]
            );
            LOGGER.debug("[substrate:config-cache] shadow load MISS (Rust): key={}, Java has {} bytes",
                cacheKey, javaConfigBytes.length);
        } else if (!javaFound && rustResult.isFound()) {
            // Rust found but Java did not -- unexpected but not a functional issue
            missCount.incrementAndGet();
            LOGGER.debug("[substrate:config-cache] shadow load MISS (Java): key={}, Rust has {} bytes",
                cacheKey, rustResult.getSerializedConfig().length);
        } else {
            // Neither found -- consistent miss
            missCount.incrementAndGet();
            LOGGER.debug("[substrate:config-cache] shadow load MISS (both): key={}", cacheKey);
        }
    }

    private void reportValidateComparison(
        String cacheKey,
        boolean javaValid,
        RustConfigCacheClient.ValidationResult rustResult
    ) {
        if (javaValid == rustResult.isValid()) {
            mismatchReporter.reportMatch();
            LOGGER.debug("[substrate:config-cache] shadow validate OK: key={}, valid={}",
                cacheKey, javaValid);
        } else {
            mismatchReporter.reportMismatch(
                "config-cache:validate:" + cacheKey,
                HashCode.fromBytes((javaValid ? "VALID" : "INVALID").getBytes(java.nio.charset.StandardCharsets.UTF_8)),
                HashCode.fromBytes((rustResult.isValid() ? "VALID" : "INVALID").getBytes(java.nio.charset.StandardCharsets.UTF_8))
            );
            LOGGER.debug("[substrate:config-cache] shadow validate MISMATCH: key={}, java={}, rust={}, reason={}",
                cacheKey, javaValid, rustResult.isValid(), rustResult.getReason());
        }
    }

    private void reportRustError(String operation, String cacheKey, Exception e) {
        mismatchReporter.reportRustError(
            "config-cache:" + operation + ":" + cacheKey,
            new RuntimeException("Rust " + operation + " failed: " + e.getMessage(), e)
        );
    }

    // --- Stats ---

    public boolean isAuthoritative() {
        return authoritative;
    }

    /**
     * Get the total number of store operations shadowed.
     */
    public long getStoreCount() {
        return storeCount.get();
    }

    /**
     * Get the total number of load operations shadowed.
     */
    public long getLoadCount() {
        return loadCount.get();
    }

    /**
     * Get the number of loads where both Java and Rust found an entry.
     */
    public long getHitCount() {
        return hitCount.get();
    }

    /**
     * Get the number of loads where at least one side did not find an entry.
     */
    public long getMissCount() {
        return missCount.get();
    }

    /**
     * Get the total number of validate operations shadowed.
     */
    public long getValidateCount() {
        return validateCount.get();
    }

    /**
     * Get the hit rate as a ratio between 0.0 and 1.0.
     */
    public double getHitRate() {
        long loads = loadCount.get();
        return loads == 0 ? 0.0 : (double) hitCount.get() / loads;
    }

    /**
     * Returns a summary string of shadow statistics.
     */
    @Override
    public String toString() {
        return String.format(
            "config-cache %s: stores=%d, loads=%d, hits=%d, misses=%d, hitRate=%.1f%%, validates=%d",
            authoritative ? "authoritative" : "shadow",
            storeCount.get(), loadCount.get(), hitCount.get(), missCount.get(),
            getHitRate() * 100, validateCount.get()
        );
    }
}
