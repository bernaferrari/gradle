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

    private final AtomicLong storeCount = new AtomicLong(0);
    private final AtomicLong loadCount = new AtomicLong(0);
    private final AtomicLong hitCount = new AtomicLong(0);
    private final AtomicLong missCount = new AtomicLong(0);
    private final AtomicLong validateCount = new AtomicLong(0);

    public ConfigurationCacheShadowListener(
        RustConfigCacheClient client,
        HashMismatchReporter mismatchReporter
    ) {
        this.client = client;
        this.mismatchReporter = mismatchReporter;
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

        try {
            boolean stored = client.storeConfigCache(cacheKey, serializedConfig, entryCount, inputHashes);

            if (stored) {
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
        } catch (Exception e) {
            mismatchReporter.reportRustError(
                "config-cache:store:" + cacheKey,
                new RuntimeException("Rust store failed: " + e.getMessage(), e)
            );
            LOGGER.debug("[substrate:config-cache] shadow store error for key={}: {}",
                cacheKey, e.getMessage());
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

        try {
            RustConfigCacheClient.CacheLoadResult rustResult = client.loadConfigCache(cacheKey);

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
        } catch (Exception e) {
            mismatchReporter.reportRustError(
                "config-cache:load:" + cacheKey,
                new RuntimeException("Rust load failed: " + e.getMessage(), e)
            );
            LOGGER.debug("[substrate:config-cache] shadow load error for key={}: {}",
                cacheKey, e.getMessage());
        }
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

        try {
            RustConfigCacheClient.ValidationResult rustResult = client.validateConfig(cacheKey, inputHashes);

            if (javaValid == rustResult.isValid()) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:config-cache] shadow validate OK: key={}, valid={}",
                    cacheKey, javaValid);
            } else {
                mismatchReporter.reportMismatch(
                    "config-cache:validate:" + cacheKey,
                    HashCode.fromString(javaValid ? "VALID" : "INVALID"),
                    HashCode.fromString(rustResult.isValid() ? "VALID" : "INVALID")
                );
                LOGGER.debug("[substrate:config-cache] shadow validate MISMATCH: key={}, java={}, rust={}, reason={}",
                    cacheKey, javaValid, rustResult.isValid(), rustResult.getReason());
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError(
                "config-cache:validate:" + cacheKey,
                new RuntimeException("Rust validate failed: " + e.getMessage(), e)
            );
            LOGGER.debug("[substrate:config-cache] shadow validate error for key={}: {}",
                cacheKey, e.getMessage());
        }
    }

    // --- Stats ---

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
            "config-cache shadow: stores=%d, loads=%d, hits=%d, misses=%d, hitRate=%.1f%%, validates=%d",
            storeCount.get(), loadCount.get(), hitCount.get(), missCount.get(),
            getHitRate() * 100, validateCount.get()
        );
    }
}
