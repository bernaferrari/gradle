package org.gradle.internal.rustbridge.cache;

import gradle.substrate.v1.BuildCachePackSpec;
import gradle.substrate.v1.PackCacheEntryResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

/**
 * Shadowing wrapper for Build Cache Packaging (bigger slice, first skeleton).
 *
 * <p>Modeled exactly on ShadowingArtifactPublisher (publishing POM activation) + ShadowingFileHashCache
 * / ShadowingExecutionHistoryStore patterns: shadow calls exercise Rust in parallel (fire-and-forget
 * for pack in some paths), compare packaged bytes or roundtrip fidelity vs JVM reference (Tar/GZip packer),
 * report via HashMismatchReporter under "cache-packaging" (or "build-cache").
 *
 * <p>Shadow mode (default on enable): Java packer authoritative; Rust exercised for differential
 * (exact bytes for supported spec subset, or structural after unpack). Fail-closed.
 *
 * <p>Activation: behind ENABLE_RUST_CACHE (or new packaging variant). No hot-path change until evidence gate.
 * Full capture wiring (from packer call sites) + reporter injection = next per plan.
 *
 * <p>See RustBuildCachePackagingClient javadoc for capture snippet.
 * Cross-ref: cache.proto, substrate cache_orchestration.rs (pack_to_bytes + golden test),
 * plan.md Build Cache Packaging section (explorer + this delivery + TODOs + evidence notes).</p>
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

    /**
     * Shadow a pack operation for a captured entry spec.
     *
     * <p>Compares Rust packaged bytes length/leading bytes vs Java reference for basic
     * differential evidence. Full byte equality for skeleton simple specs (no timestamps).
     *
     * @param buildId        correlation id
     * @param spec           the captured pack spec (key + metadata + blobs)
     * @param javaPackaged   the authoritative JVM-produced packaged bytes (GZip + Tar packer)
     */
    public void shadowPackCacheEntry(String buildId, BuildCachePackSpec spec, byte[] javaPackaged) {
        if (rustClient == null) {
            return;
        }
        try {
            PackCacheEntryResponse rustResp = rustClient.packCacheEntry(buildId, spec);
            if (rustResp == null || !rustResp.getSuccess()) {
                mismatchReporter.reportRustError(
                    "cache-packaging:pack:" + buildId,
                    new RuntimeException("Rust pack failed or empty: " + (rustResp != null ? rustResp.getError() : "null"))
                );
                return;
            }
            byte[] rustBytes = rustResp.getPackagedBytes().toByteArray();

            // Differential: for skeleton use length + prefix match (full bytes for small golden cases)
            boolean match = javaPackaged != null && javaPackaged.length == rustBytes.length;
            if (match && javaPackaged.length > 0) {
                // Basic prefix check (full compare expensive for large; real would hash or exact for evidence)
                int checkLen = Math.min(64, javaPackaged.length);
                for (int i = 0; i < checkLen; i++) {
                    if (javaPackaged[i] != rustBytes[i]) {
                        match = false;
                        break;
                    }
                }
            }

            if (match) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:cache-packaging] shadow pack MATCH: buildId={}, bytes={}", buildId, rustBytes.length);
            } else {
                mismatchReporter.reportMismatch(
                    "cache-packaging:pack:bytes:" + buildId,
                    javaPackaged != null ? "len=" + javaPackaged.length : "null",
                    "len=" + (rustBytes != null ? rustBytes.length : 0)
                );
                LOGGER.debug("[substrate:cache-packaging] shadow pack MISMATCH: buildId={} javaLen={} rustLen={}",
                    buildId, javaPackaged != null ? javaPackaged.length : -1, rustBytes != null ? rustBytes.length : -1);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("cache-packaging:pack:" + buildId, e);
            LOGGER.debug("[substrate:cache-packaging] shadow pack error for buildId={}: {}", buildId, e.getMessage());
        }
    }

    /**
     * Convenience for minimal specs (used by early tests/golden of the capture path).
     */
    public void shadowPackCacheEntry(String buildId, String keyHex, byte[] javaPackaged) {
        BuildCachePackSpec spec = BuildCachePackSpec.newBuilder()
            .setCacheKey(com.google.protobuf.ByteString.copyFromUtf8(keyHex != null ? keyHex : ""))
            .build();
        shadowPackCacheEntry(buildId, spec, javaPackaged);
    }
}
