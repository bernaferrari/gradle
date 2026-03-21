package org.gradle.internal.rustbridge.gc;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.List;

/**
 * Client for the Rust garbage collection service.
 * Provides GC operations for substrate-managed stores.
 */
public class RustGarbageCollectionClient {

    private static final Logger LOGGER = Logging.getLogger(RustGarbageCollectionClient.class);

    private final SubstrateClient client;

    public RustGarbageCollectionClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of a garbage collection operation.
     */
    public static class GcResult {
        private final int entriesRemoved;
        private final long bytesRecovered;
        private final int entriesRemaining;

        private GcResult(int entriesRemoved, long bytesRecovered, int entriesRemaining) {
            this.entriesRemoved = entriesRemoved;
            this.bytesRecovered = bytesRecovered;
            this.entriesRemaining = entriesRemaining;
        }

        public int getEntriesRemoved() { return entriesRemoved; }
        public long getBytesRecovered() { return bytesRecovered; }
        public int getEntriesRemaining() { return entriesRemaining; }

        @Override
        public String toString() {
            return String.format("GcResult{removed=%d, recovered=%d bytes, remaining=%d}",
                entriesRemoved, bytesRecovered, entriesRemaining);
        }
    }

    /**
     * Storage statistics for a managed store.
     */
    public static class StorageStat {
        private final String storeName;
        private final long entries;
        private final long totalBytes;
        private final long oldestEntryMs;
        private final long newestEntryMs;

        private StorageStat(String storeName, long entries, long totalBytes,
                           long oldestEntryMs, long newestEntryMs) {
            this.storeName = storeName;
            this.entries = entries;
            this.totalBytes = totalBytes;
            this.oldestEntryMs = oldestEntryMs;
            this.newestEntryMs = newestEntryMs;
        }

        public String getStoreName() { return storeName; }
        public long getEntries() { return entries; }
        public long getTotalBytes() { return totalBytes; }
        public long getOldestEntryMs() { return oldestEntryMs; }
        public long getNewestEntryMs() { return newestEntryMs; }

        public String formatBytes() {
            if (totalBytes < 1024) return totalBytes + " B";
            if (totalBytes < 1024 * 1024) return String.format("%.1f KB", totalBytes / 1024.0);
            return String.format("%.1f MB", totalBytes / (1024.0 * 1024.0));
        }
    }

    /**
     * Run garbage collection on the build cache.
     */
    public GcResult gcBuildCache(long maxAgeMs, long maxTotalBytes, boolean dryRun) {
        if (client.isNoop()) {
            return null;
        }

        try {
            gradle.substrate.v1.GcBuildCacheRequest request =
                gradle.substrate.v1.GcBuildCacheRequest.newBuilder()
                    .setMaxAgeMs(maxAgeMs)
                    .setMaxTotalBytes(maxTotalBytes)
                    .setDryRun(dryRun)
                    .build();

            gradle.substrate.v1.GcBuildCacheResponse response =
                client.getGarbageCollectionStub().gcBuildCache(request);

            return new GcResult(
                response.getEntriesRemoved(),
                response.getBytesRecovered(),
                response.getEntriesRemaining()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:gc] build cache GC failed: {}", e.getMessage());
            return null;
        }
    }

    /**
     * Run garbage collection on execution history.
     */
    public GcResult gcExecutionHistory(long maxAgeMs, int maxEntries, boolean dryRun) {
        if (client.isNoop()) {
            return null;
        }

        try {
            gradle.substrate.v1.GcExecutionHistoryRequest request =
                gradle.substrate.v1.GcExecutionHistoryRequest.newBuilder()
                    .setMaxAgeMs(maxAgeMs)
                    .setMaxEntries(maxEntries)
                    .setDryRun(dryRun)
                    .build();

            gradle.substrate.v1.GcExecutionHistoryResponse response =
                client.getGarbageCollectionStub().gcExecutionHistory(request);

            return new GcResult(
                response.getEntriesRemoved(),
                response.getBytesRecovered(),
                response.getEntriesRemaining()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:gc] execution history GC failed: {}", e.getMessage());
            return null;
        }
    }

    /**
     * Run garbage collection on configuration cache.
     */
    public GcResult gcConfigCache(long maxAgeMs, int maxEntries, boolean dryRun) {
        if (client.isNoop()) {
            return null;
        }

        try {
            gradle.substrate.v1.GcConfigCacheRequest request =
                gradle.substrate.v1.GcConfigCacheRequest.newBuilder()
                    .setMaxAgeMs(maxAgeMs)
                    .setMaxEntries(maxEntries)
                    .setDryRun(dryRun)
                    .build();

            gradle.substrate.v1.GcConfigCacheResponse response =
                client.getGarbageCollectionStub().gcConfigCache(request);

            return new GcResult(
                response.getEntriesRemoved(),
                response.getBytesRecovered(),
                response.getEntriesRemaining()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:gc] config cache GC failed: {}", e.getMessage());
            return null;
        }
    }

    /**
     * Get storage statistics for all managed stores.
     */
    public List<StorageStat> getStorageStats() {
        if (client.isNoop()) {
            return new ArrayList<>();
        }

        try {
            gradle.substrate.v1.GetStorageStatsRequest request =
                gradle.substrate.v1.GetStorageStatsRequest.newBuilder().build();

            gradle.substrate.v1.GetStorageStatsResponse response =
                client.getGarbageCollectionStub().getStorageStats(request);

            List<StorageStat> stats = new ArrayList<>();
            for (gradle.substrate.v1.StorageStats stat : response.getStatsList()) {
                stats.add(new StorageStat(
                    stat.getStoreName(),
                    stat.getEntries(),
                    stat.getTotalBytes(),
                    stat.getOldestEntryMs(),
                    stat.getNewestEntryMs()
                ));
            }
            return stats;
        } catch (Exception e) {
            LOGGER.debug("[substrate:gc] getStorageStats failed: {}", e.getMessage());
            return new ArrayList<>();
        }
    }
}
