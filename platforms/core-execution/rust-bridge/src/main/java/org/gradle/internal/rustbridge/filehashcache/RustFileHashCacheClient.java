package org.gradle.internal.rustbridge.filehashcache;

import org.gradle.api.logging.Logging;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.Optional;

/**
 * Compile-safe facade for the Rust file-hash-cache slice.
 *
 * <p>The Java bridge does not currently have generated protobuf classes for a
 * FileHashCacheService. Until the proto contract exists, this client behaves as
 * a cache miss/no-op so shadow mode can keep Java authoritative.</p>
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
        if (!client.isNoop()) {
            LOGGER.debug("[substrate:file-hash-cache] protobuf contract is not available; returning miss for {}", path);
        }
        return new FileInfoResult(false, Optional.empty(), "");
    }

    public boolean putFileInfo(String path, HashCode hash, long length, long lastModified, String kind) {
        if (!client.isNoop()) {
            LOGGER.debug("[substrate:file-hash-cache] protobuf contract is not available; skipping put for {}", path);
        }
        return false;
    }

    public boolean invalidate(String path) {
        if (!client.isNoop()) {
            LOGGER.debug("[substrate:file-hash-cache] protobuf contract is not available; skipping invalidate for {}", path);
        }
        return false;
    }

    public CacheStats getStats() {
        return new CacheStats(0, 0, 0, 0);
    }
}
