package org.gradle.internal.rustbridge.shadow;

import org.gradle.api.logging.Logging;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.cache.BuildCacheOrchestrationClient;
import org.slf4j.Logger;

import java.util.List;
import java.util.Map;

/**
 * Shadow adapter that compares Java-computed cache keys with Rust-computed cache keys.
 * In shadow mode, Java remains authoritative. In authoritative mode, Rust key is used
 * when available and Java is the fallback.
 */
public class ShadowingBuildCacheKeyComputer {

    private static final Logger LOGGER = Logging.getLogger(ShadowingBuildCacheKeyComputer.class);

    private final BuildCacheOrchestrationClient cacheOrchestration;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public ShadowingBuildCacheKeyComputer(
        BuildCacheOrchestrationClient cacheOrchestration,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.cacheOrchestration = cacheOrchestration;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    /**
     * Compare cache keys computed by Java and Rust for the same inputs.
     * Always returns the Java-computed key.
     *
     * @param javaKeyString the Java-computed cache key (authoritative)
     * @param workIdentity the work identity
     * @param implHash the implementation hash
     * @param inputPropertyHashes input property name-to-hash mapping
     * @param inputFileHashes input file name-to-hash mapping
     * @param outputNames output property names
     * @return the Java-computed cache key string
     */
    public String computeAndCompare(
        String javaKeyString,
        String workIdentity,
        String implHash,
        Map<String, String> inputPropertyHashes,
        Map<String, String> inputFileHashes,
        List<String> outputNames
    ) {
        if (cacheOrchestration == null) {
            return javaKeyString;
        }

        try {
            BuildCacheOrchestrationClient.CacheKeyResult rustResult = authoritative
                ? cacheOrchestration.computeCacheKeyStrict(
                    workIdentity, implHash, inputPropertyHashes, inputFileHashes, outputNames)
                : cacheOrchestration.computeCacheKey(
                    workIdentity, implHash, inputPropertyHashes, inputFileHashes, outputNames);

            if (rustResult.isSuccess()) {
                if (javaKeyString.equals(rustResult.getKeyString())) {
                    mismatchReporter.reportMatch();
                    LOGGER.debug("[substrate:cache-key] shadow OK: {} matches", workIdentity);
                } else {
                    mismatchReporter.reportMismatch(
                        "cache-key:" + workIdentity,
                        HashCode.fromString(javaKeyString),
                        rustResult.getKeyBytes()
                    );
                }
                if (authoritative) {
                    LOGGER.info("[substrate:cache-key] authoritative: using Rust key for {}", workIdentity);
                    return rustResult.getKeyString();
                }
            } else {
                mismatchReporter.reportRustError(
                    "cache-key:" + workIdentity,
                    new RuntimeException(rustResult.getErrorMessage())
                );
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("cache-key:" + workIdentity, e);
            LOGGER.debug("[substrate:cache-key] shadow comparison failed", e);
        }

        return javaKeyString;
    }
}
