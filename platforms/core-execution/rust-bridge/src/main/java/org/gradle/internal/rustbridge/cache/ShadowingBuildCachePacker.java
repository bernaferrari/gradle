package org.gradle.internal.rustbridge.cache;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.Arrays;

/**
 * Shadowing wrapper for the build-cache packaging slice.
 */
public class ShadowingBuildCachePacker {

    private static final Logger LOGGER = Logging.getLogger(ShadowingBuildCachePacker.class);

    private final RustBuildCachePackagingClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingBuildCachePacker(
        RustBuildCachePackagingClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    public void shadowPackCacheEntry(String buildId, Object spec, byte[] javaPackaged) {
        if (rustClient == null) {
            return;
        }
        RustBuildCachePackagingClient.PackResult rustResult = rustClient.packCacheEntry(buildId, spec);
        if (!rustResult.isSuccess()) {
            mismatchReporter.reportRustError(
                "cache-packaging:pack:" + buildId,
                new RuntimeException(rustResult.getError())
            );
            LOGGER.debug("[substrate:cache-packaging] pack skipped for {}: {}", buildId, rustResult.getError());
            return;
        }
        byte[] rustBytes = rustResult.getPackagedBytes();
        boolean match = Arrays.equals(javaPackaged, rustBytes);
        if (match) {
            mismatchReporter.reportMatch();
        } else {
            mismatchReporter.reportMismatch(
                "cache-packaging:pack:bytes:" + buildId,
                javaPackaged == null ? "null" : "len=" + javaPackaged.length,
                "len=" + rustBytes.length
            );
        }
    }

    public void shadowPackCacheEntry(String buildId, String keyHex, byte[] javaPackaged) {
        shadowPackCacheEntry(buildId, (Object) keyHex, javaPackaged);
    }
}
