package org.gradle.internal.rustbridge.filehashcache;

import com.google.protobuf.ByteString;
import gradle.substrate.v1.FileInfo;
import gradle.substrate.v1.GetFileHashCacheStatsRequest;
import gradle.substrate.v1.GetFileHashCacheStatsResponse;
import gradle.substrate.v1.GetFileInfoRequest;
import gradle.substrate.v1.GetFileInfoResponse;
import gradle.substrate.v1.InvalidateFileInfoRequest;
import gradle.substrate.v1.InvalidateFileInfoResponse;
import gradle.substrate.v1.PutFileInfoRequest;
import gradle.substrate.v1.PutFileInfoResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.Optional;

/**
 * Thin gRPC wrapper client for the Rust {@code FileHashCacheService}.
 *
 * <p>Modeled exactly on {@link org.gradle.internal.rustbridge.hash.RustGrpcFileHasher}:
 * thin wrapper around the stub obtained via {@code SubstrateClient.getFileHashCacheStub()},
 * centralizes HashCode &lt;-&gt; bytes (ByteString) conversion, maps all gRPC / proto errors
 * and no-op cases to cache misses (or put=false) for safe shadow usage.
 *
 * <p>This is the Java-side bridge for the persistent File Hash Cache bigger slice.
 * It targets the same semantic space as {@code CrossBuildFileHashCache} (FILE_HASHES and CHECKSUMS kinds)
 * plus resource snapshotter caches.
 *
 * <p>Fail-closed by design: errors never throw from the thin client (surface as misses); callers
 * (ShadowingFileHashCache + wiring) enforce authoritative fail-closed.
 */
public class RustFileHashCacheClient {

    private static final Logger LOGGER = Logging.getLogger(RustFileHashCacheClient.class);

    private final SubstrateClient client;

    public RustFileHashCacheClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of a GetFileInfo call.
     */
    public static class FileInfoResult {
        private final boolean hit;
        private final Optional<FileInfoData> info;
        private final String errorMessage;

        public FileInfoResult(boolean hit, Optional<FileInfoData> info, String errorMessage) {
            this.hit = hit;
            this.info = info;
            this.errorMessage = errorMessage != null ? errorMessage : "";
        }

        public boolean isHit() {
            return hit;
        }

        public Optional<FileInfoData> getInfo() {
            return info;
        }

        public String getErrorMessage() {
            return errorMessage;
        }

        public boolean isSuccess() {
            return errorMessage.isEmpty();
        }
    }

    /**
     * Serializable view of cached file info (content hash + fs metadata).
     */
    public static class FileInfoData {
        private final HashCode hash;
        private final long length;
        private final long lastModified;

        public FileInfoData(HashCode hash, long length, long lastModified) {
            this.hash = hash;
            this.length = length;
            this.lastModified = lastModified;
        }

        public HashCode getHash() {
            return hash;
        }

        public long getLength() {
            return length;
        }

        public long getLastModified() {
            return lastModified;
        }
    }

    /**
     * Stats returned by GetStats.
     */
    public static class CacheStats {
        public final long hits;
        public final long misses;
        public final long entries;
        public final long bytes;

        public CacheStats(long hits, long misses, long entries, long bytes) {
            this.hits = hits;
            this.misses = misses;
            this.entries = entries;
            this.bytes = bytes;
        }
    }

    /**
     * Retrieve cached FileInfo for the given path + fs identity + kind.
     *
     * @param path absolute path
     * @param length file length at snapshot time
     * @param lastModified mtime
     * @param kind "FILE_HASHES" or "CHECKSUMS" (or other namespaces)
     */
    public FileInfoResult getFileInfo(String path, long length, long lastModified, String kind) {
        if (client.isNoop()) {
            return new FileInfoResult(false, Optional.empty(), "Substrate not available");
        }
        try {
            GetFileInfoRequest request = GetFileInfoRequest.newBuilder()
                .setPath(path)
                .setLength(length)
                .setLastModified(lastModified)
                .setKind(kind == null || kind.isEmpty() ? "FILE_HASHES" : kind)
                .build();

            GetFileInfoResponse response = client.getFileHashCacheStub().getFileInfo(request);

            if (!response.getError().isEmpty()) {
                LOGGER.debug("[substrate:file-hash-cache] getFileInfo error for {}: {}", path, response.getError());
                return new FileInfoResult(false, Optional.empty(), response.getError());
            }

            if (response.getHit() && response.hasInfo()) {
                FileInfo proto = response.getInfo();
                FileInfoData data = new FileInfoData(
                    HashCode.fromBytes(proto.getHash().toByteArray()),
                    proto.getLength(),
                    proto.getLastModified()
                );
                return new FileInfoResult(true, Optional.of(data), "");
            }

            return new FileInfoResult(false, Optional.empty(), "");
        } catch (Exception e) {
            LOGGER.debug("[substrate:file-hash-cache] getFileInfo gRPC call failed for {}", path, e);
            return new FileInfoResult(false, Optional.empty(), "gRPC error: " + e.getMessage());
        }
    }

    /**
     * Store FileInfo for a path.
     */
    public boolean putFileInfo(String path, HashCode hash, long length, long lastModified, String kind) {
        if (client.isNoop()) {
            return false;
        }
        try {
            FileInfo protoInfo = FileInfo.newBuilder()
                .setHash(ByteString.copyFrom(hash.toByteArray()))
                .setLength(length)
                .setLastModified(lastModified)
                .build();

            PutFileInfoRequest request = PutFileInfoRequest.newBuilder()
                .setPath(path)
                .setInfo(protoInfo)
                .setKind(kind == null || kind.isEmpty() ? "FILE_HASHES" : kind)
                .build();

            PutFileInfoResponse response = client.getFileHashCacheStub().putFileInfo(request);
            if (!response.getSuccess() && !response.getError().isEmpty()) {
                LOGGER.debug("[substrate:file-hash-cache] putFileInfo failed for {}: {}", path, response.getError());
            }
            return response.getSuccess();
        } catch (Exception e) {
            LOGGER.debug("[substrate:file-hash-cache] putFileInfo gRPC call failed for {}", path, e);
            return false;
        }
    }

    /**
     * Invalidate any cached entry for the path (kind-agnostic for now; Rust impl may scope by key).
     */
    public boolean invalidate(String path) {
        if (client.isNoop()) {
            return false;
        }
        try {
            InvalidateFileInfoRequest request = InvalidateFileInfoRequest.newBuilder()
                .setPath(path)
                .build();

            InvalidateFileInfoResponse response = client.getFileHashCacheStub().invalidateFileInfo(request);
            return response.getSuccess();
        } catch (Exception e) {
            LOGGER.debug("[substrate:file-hash-cache] invalidate gRPC call failed for {}", path, e);
            return false;
        }
    }

    /**
     * Retrieve current stats from the Rust cache implementation.
     */
    public CacheStats getStats() {
        if (client.isNoop()) {
            return new CacheStats(0, 0, 0, 0);
        }
        try {
            GetFileHashCacheStatsResponse response = client.getFileHashCacheStub()
                .getStats(GetFileHashCacheStatsRequest.newBuilder().build());
            return new CacheStats(
                response.getHits(),
                response.getMisses(),
                response.getEntries(),
                response.getBytes()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:file-hash-cache] getStats gRPC call failed", e);
            return new CacheStats(0, 0, 0, 0);
        }
    }
}
