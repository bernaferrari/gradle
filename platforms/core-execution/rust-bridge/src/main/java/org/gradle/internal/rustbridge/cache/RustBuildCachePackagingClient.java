package org.gradle.internal.rustbridge.cache;

import gradle.substrate.v1.BuildCachePackSpec;
import gradle.substrate.v1.BuildCachePackagingServiceGrpc;
import gradle.substrate.v1.PackCacheEntryRequest;
import gradle.substrate.v1.PackCacheEntryResponse;
import gradle.substrate.v1.UnpackCacheEntryRequest;
import gradle.substrate.v1.UnpackCacheEntryResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

/**
 * Thin Java client for the Rust BuildCachePackagingService (bigger slice activation).
 *
 * <p>Build Cache Packaging bigger slice (first skeleton + basic packaging path,
 * next after Publishing per 99-tool discovery + plan.md beads vtq8):
 * "Java captures effective entry (key + metadata + content blobs) → Rust deterministic
 * packager produces canonical tar.gz (reproducible layout, mtime=0, matching Tar/GZip packer
 * expectations for same input)".
 *
 * <p>Thin methods enable the capture spec → Rust package path immediately (testable via
 * unit or differential like publishing golden). Full capture from BuildCacheEntryPacker /
 * DefaultBuildCacheController sites lands in follow-ups (modeled on RustArtifactPublishingClient
 * javadoc capture snippets + ConfigurationCacheShadowListener).
 *
 * <p>Usage (shadow-only; JVM packer authoritative):
 * <pre>
 * BuildCachePackSpec spec = BuildCachePackSpec.newBuilder()
 *     .setCacheKey(...) // from BuildCacheKey
 *     .putAllOriginMetadata(...) // from OriginMetadataFactory writer props
 *     .putAllFileContents(...) // captured blobs (future: from snapshot tree walk)
 *     .build();
 * PackCacheEntryResponse resp = client.packCacheEntry(buildId, spec);
 * // In shadow: byte-compare resp.packagedBytes vs Java GZip(TarPacker) output; report via HashMismatchReporter("cache-packaging")
 * </pre>
 *
 * <p>Flag guarded (ENABLE_RUST_CACHE or dedicated packaging variant). No behavior change.
 * See cache.proto (additive service), cache_orchestration.rs (Rust skeleton + golden roundtrip test),
 * ShadowingBuildCachePacker, plan.md "Build Cache Packaging" section.</p>
 */
public class RustBuildCachePackagingClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildCachePackagingClient.class);

    private final SubstrateClient client;

    public RustBuildCachePackagingClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Pack a captured cache entry spec into canonical packaged bytes (deterministic tar.gz).
     * Skeleton path live and testable.
     */
    public PackCacheEntryResponse packCacheEntry(String buildId, BuildCachePackSpec spec) {
        if (client.isNoop()) {
            return PackCacheEntryResponse.getDefaultInstance();
        }
        try {
            // Real gRPC path now live (additive SubstrateClient.getCachePackagingStub wired).
            // buildId is correlation key only (for reporter/logs); proto request carries the spec.
            // Modeled exactly on RustArtifactPublishingClient usage of getArtifactPublishingStub().
            PackCacheEntryRequest request = PackCacheEntryRequest.newBuilder()
                .setSpec(spec != null ? spec : BuildCachePackSpec.getDefaultInstance())
                .build();
            PackCacheEntryResponse response = client.getCachePackagingStub().packCacheEntry(request);
            LOGGER.debug("[substrate:cache-packaging] packCacheEntry via gRPC: buildId={}, success={}", buildId, response.getSuccess());
            return response;
        } catch (Exception e) {
            LOGGER.debug("[substrate:cache-packaging] packCacheEntry failed", e);
            PackCacheEntryResponse.Builder b = PackCacheEntryResponse.newBuilder().setSuccess(false);
            if (e.getMessage() != null) b.setError(e.getMessage());
            return b.build();
        }
    }

    /**
     * Unpack packaged bytes back to spec (for roundtrip/diff in shadow).
     */
    public UnpackCacheEntryResponse unpackCacheEntry(String buildId, byte[] packagedBytes) {
        if (client.isNoop()) {
            return UnpackCacheEntryResponse.getDefaultInstance();
        }
        try {
            // Real gRPC path (additive stub).
            UnpackCacheEntryRequest request = UnpackCacheEntryRequest.newBuilder()
                .setPackagedBytes(com.google.protobuf.ByteString.copyFrom(packagedBytes != null ? packagedBytes : new byte[0]))
                .build();
            UnpackCacheEntryResponse response = client.getCachePackagingStub().unpackCacheEntry(request);
            LOGGER.debug("[substrate:cache-packaging] unpackCacheEntry via gRPC: buildId={}, success={}", buildId, response.getSuccess());
            return response;
        } catch (Exception e) {
            LOGGER.debug("[substrate:cache-packaging] unpackCacheEntry failed", e);
            UnpackCacheEntryResponse.Builder b = UnpackCacheEntryResponse.newBuilder().setSuccess(false);
            if (e.getMessage() != null) b.setError(e.getMessage());
            return b.build();
        }
    }
}
