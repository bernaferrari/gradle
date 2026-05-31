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
 * Thin client for the Rust file-hash-cache slice.
 */
public class RustFileHashCacheClient {

    private static final Logger LOGGER = Logging.getLogger(RustFileHashCacheClient.class);

    private final SubstrateClient client;

    public RustFileHashCacheClient(SubstrateClient client) {
        this.client = client;
    }

    public static class FileInfoResult {
        private final boolean hit;
        private final Optional<FileInfoData> info;
        private final String errorMessage;

        public FileInfoResult(boolean hit, Optional<FileInfoData> info, String errorMessage) {
            this.hit = hit;
            this.info = info;
            this.errorMessage = errorMessage == null ? "" : errorMessage;
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

    public FileInfoResult getFileInfo(String path, long length, long lastModified, String kind) {
        if (client.isNoop()) {
            return new FileInfoResult(false, Optional.empty(), "");
        }
        try {
            GetFileInfoResponse response = client.getFileHashCacheStub().getFileInfo(GetFileInfoRequest.newBuilder()
                .setPath(path == null ? "" : path)
                .setLength(length)
                .setLastModified(lastModified)
                .setKind(normalizeKind(kind))
                .build());
            if (!response.getError().isEmpty()) {
                return new FileInfoResult(false, Optional.empty(), response.getError());
            }
            if (!response.getHit() || !response.hasInfo()) {
                return new FileInfoResult(false, Optional.empty(), "");
            }
            FileInfo info = response.getInfo();
            FileInfoData data = new FileInfoData(
                HashCode.fromBytes(info.getHash().toByteArray()),
                info.getLength(),
                info.getLastModified()
            );
            return new FileInfoResult(true, Optional.of(data), "");
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate:file-hash-cache] get failed for {}", path, e);
            return new FileInfoResult(false, Optional.empty(), e.getMessage());
        }
    }

    public boolean putFileInfo(String path, HashCode hash, long length, long lastModified, String kind) {
        if (client.isNoop() || hash == null) {
            return false;
        }
        try {
            PutFileInfoResponse response = client.getFileHashCacheStub().putFileInfo(PutFileInfoRequest.newBuilder()
                .setPath(path == null ? "" : path)
                .setKind(normalizeKind(kind))
                .setInfo(FileInfo.newBuilder()
                    .setHash(ByteString.copyFrom(hash.toByteArray()))
                    .setLength(length)
                    .setLastModified(lastModified)
                    .build())
                .build());
            if (!response.getSuccess()) {
                LOGGER.debug("[substrate:file-hash-cache] put failed for {}: {}", path, response.getError());
            }
            return response.getSuccess();
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate:file-hash-cache] put failed for {}", path, e);
            return false;
        }
    }

    public boolean invalidate(String path) {
        if (client.isNoop()) {
            return false;
        }
        try {
            InvalidateFileInfoResponse response = client.getFileHashCacheStub().invalidateFileInfo(
                InvalidateFileInfoRequest.newBuilder()
                    .setPath(path == null ? "" : path)
                    .build()
            );
            return response.getSuccess();
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate:file-hash-cache] invalidate failed for {}", path, e);
            return false;
        }
    }

    public CacheStats getStats() {
        if (client.isNoop()) {
            return new CacheStats(0, 0, 0, 0);
        }
        try {
            GetFileHashCacheStatsResponse response = client.getFileHashCacheStub().getStats(
                GetFileHashCacheStatsRequest.newBuilder().build()
            );
            return new CacheStats(
                response.getHits(),
                response.getMisses(),
                response.getEntries(),
                response.getBytes()
            );
        } catch (RuntimeException e) {
            LOGGER.debug("[substrate:file-hash-cache] stats failed", e);
            return new CacheStats(0, 0, 0, 0);
        }
    }

    private static String normalizeKind(String kind) {
        return kind == null || kind.isEmpty() ? "FILE_HASHES" : kind;
    }
}
