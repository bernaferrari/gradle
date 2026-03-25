package org.gradle.internal.rustbridge.watch;

import gradle.substrate.v1.FileChangeEvent;
import gradle.substrate.v1.FileWatchServiceGrpc;
import gradle.substrate.v1.GetWatchStatsRequest;
import gradle.substrate.v1.GetWatchStatsResponse;
import gradle.substrate.v1.PollChangesRequest;
import gradle.substrate.v1.StartWatchingRequest;
import gradle.substrate.v1.StartWatchingResponse;
import gradle.substrate.v1.StopWatchingRequest;
import gradle.substrate.v1.StopWatchingResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.List;

/**
 * Client for the Rust file watching service.
 * Manages filesystem watchers and polls for change events.
 */
public class RustFileWatchClient {

    private static final Logger LOGGER = Logging.getLogger(RustFileWatchClient.class);

    private final SubstrateClient client;

    public RustFileWatchClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Start watching a directory tree.
     *
     * @param rootPath the root directory to watch
     * @param includePatterns glob patterns to include
     * @param excludePatterns glob patterns to exclude
     * @return watch result with watch ID
     */
    public WatchResult startWatching(
        String rootPath,
        List<String> includePatterns,
        List<String> excludePatterns
    ) {
        if (client.isNoop()) {
            return WatchResult.error("Substrate not available");
        }

        try {
            StartWatchingRequest.Builder builder = StartWatchingRequest.newBuilder()
                .setRootPath(rootPath);

            if (includePatterns != null) {
                builder.addAllIncludePatterns(includePatterns);
            }
            if (excludePatterns != null) {
                builder.addAllExcludePatterns(excludePatterns);
            }

            StartWatchingResponse response = client.getFileWatchStub()
                .startWatching(builder.build());

            if (response.getWatching()) {
                LOGGER.debug("[substrate:watch] watching {} (id={}, {} files)",
                    rootPath, response.getWatchId(), response.getFilesWatched());
                return new WatchResult(
                    response.getWatchId(),
                    response.getWatching(),
                    response.getFilesWatched(),
                    true,
                    ""
                );
            } else {
                return WatchResult.error("Rust watcher returned not-watching");
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:watch] startWatching failed", e);
            return WatchResult.error("gRPC error: " + e.getMessage());
        }
    }

    /**
     * Stop watching a directory tree.
     */
    public boolean stopWatching(String watchId) {
        if (client.isNoop()) {
            return false;
        }

        try {
            StopWatchingResponse response = client.getFileWatchStub()
                .stopWatching(StopWatchingRequest.newBuilder()
                    .setWatchId(watchId)
                    .build());

            LOGGER.debug("[substrate:watch] stopped watching {}", watchId);
            return response.getStopped();
        } catch (Exception e) {
            LOGGER.debug("[substrate:watch] stopWatching failed", e);
            return false;
        }
    }

    /**
     * Poll for file changes since a given timestamp.
     *
     * @param watchId the watch session ID
     * @param sinceTimestampMs the timestamp to poll from
     * @return list of change events
     */
    public List<FileChange> pollChanges(String watchId, long sinceTimestampMs) {
        if (client.isNoop()) {
            return Collections.emptyList();
        }

        try {
            Iterator<FileChangeEvent> events = client.getFileWatchStub()
                .pollChanges(PollChangesRequest.newBuilder()
                    .setWatchId(watchId)
                    .setSinceTimestampMs(sinceTimestampMs)
                    .build());

            List<FileChange> changes = new ArrayList<>();
            while (events.hasNext()) {
                FileChangeEvent event = events.next();
                changes.add(new FileChange(
                    event.getPath(),
                    event.getChangeType(),
                    event.getTimestampMs(),
                    event.getFileSize(),
                    event.getIsDirectory()
                ));
            }

            if (!changes.isEmpty()) {
                LOGGER.debug("[substrate:watch] polled {} changes from {}",
                    changes.size(), watchId);
            }

            return Collections.unmodifiableList(changes);
        } catch (Exception e) {
            LOGGER.debug("[substrate:watch] pollChanges failed", e);
            return Collections.emptyList();
        }
    }

    /**
     * Get statistics for a watch session.
     */
    public WatchStats getWatchStats(String watchId) {
        if (client.isNoop()) {
            return WatchStats.empty();
        }

        try {
            GetWatchStatsResponse response = client.getFileWatchStub()
                .getWatchStats(GetWatchStatsRequest.newBuilder()
                    .setWatchId(watchId)
                    .build());

            return new WatchStats(
                response.getFilesWatched(),
                response.getChangesDetected(),
                response.getLastPollTimeMs(),
                response.getWatchStartTimeMs()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:watch] getWatchStats failed", e);
            return WatchStats.empty();
        }
    }

    /**
     * Result of starting a watch.
     */
    public static class WatchResult {
        private final String watchId;
        private final boolean watching;
        private final long filesWatched;
        private final boolean success;
        private final String errorMessage;

        private WatchResult(String watchId, boolean watching, long filesWatched,
                            boolean success, String errorMessage) {
            this.watchId = watchId;
            this.watching = watching;
            this.filesWatched = filesWatched;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public static WatchResult error(String message) {
            return new WatchResult("", false, 0, false, message);
        }

        public String getWatchId() { return watchId; }
        public boolean isWatching() { return watching; }
        public long getFilesWatched() { return filesWatched; }
        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }
    }

    /**
     * A file change event from the Rust watcher.
     */
    public static class FileChange {
        private final String path;
        private final String changeType;
        private final long timestampMs;
        private final long fileSize;
        private final boolean isDirectory;

        private FileChange(String path, String changeType, long timestampMs,
                          long fileSize, boolean isDirectory) {
            this.path = path;
            this.changeType = changeType;
            this.timestampMs = timestampMs;
            this.fileSize = fileSize;
            this.isDirectory = isDirectory;
        }

        public String getPath() { return path; }
        public String getChangeType() { return changeType; }
        public long getTimestampMs() { return timestampMs; }
        public long getFileSize() { return fileSize; }
        public boolean isDirectory() { return isDirectory; }

        @Override
        public String toString() {
            return changeType + " " + path;
        }
    }

    /**
     * Watch session statistics.
     */
    public static class WatchStats {
        private final long filesWatched;
        private final long changesDetected;
        private final long lastPollTimeMs;
        private final long watchStartTimeMs;

        private WatchStats(long filesWatched, long changesDetected,
                          long lastPollTimeMs, long watchStartTimeMs) {
            this.filesWatched = filesWatched;
            this.changesDetected = changesDetected;
            this.lastPollTimeMs = lastPollTimeMs;
            this.watchStartTimeMs = watchStartTimeMs;
        }

        public static WatchStats empty() {
            return new WatchStats(0, 0, 0, 0);
        }

        public long getFilesWatched() { return filesWatched; }
        public long getChangesDetected() { return changesDetected; }
        public long getLastPollTimeMs() { return lastPollTimeMs; }
        public long getWatchStartTimeMs() { return watchStartTimeMs; }
    }
}
