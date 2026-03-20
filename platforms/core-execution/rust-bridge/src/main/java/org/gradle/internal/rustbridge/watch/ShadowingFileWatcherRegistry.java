package org.gradle.internal.rustbridge.watch;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.io.File;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * A file watcher registry that shadows Java file watching against the Rust
 * FileWatchService. Both the Java watcher and the Rust watcher receive the
 * same watch registrations and change events are compared.
 *
 * <p>Always returns Java results. Mismatches are reported but do not affect
 * build correctness.</p>
 */
public class ShadowingFileWatcherRegistry {

    private static final Logger LOGGER = Logging.getLogger(ShadowingFileWatcherRegistry.class);

    private final RustFileWatchClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    /** Map of root path -> Rust watch ID. */
    private final Map<String, String> activeWatches = new HashMap<>();

    public ShadowingFileWatcherRegistry(
        RustFileWatchClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Start watching a directory. Also registers with the Rust watcher for shadow comparison.
     *
     * @param rootPath the root directory to watch
     * @param includePatterns glob patterns to include
     * @param excludePatterns glob patterns to exclude
     */
    public void startWatching(
        String rootPath,
        List<String> includePatterns,
        List<String> excludePatterns
    ) {
        // Register with Rust watcher
        if (rustClient != null) {
            try {
                RustFileWatchClient.WatchResult result =
                    rustClient.startWatching(rootPath, includePatterns, excludePatterns);

                if (result.isSuccess() && result.isWatching()) {
                    activeWatches.put(rootPath, result.getWatchId());
                    LOGGER.debug("[substrate:watch] shadow watch started for {} (id={})",
                        rootPath, result.getWatchId());
                } else {
                    mismatchReporter.reportRustError(
                        "watch:" + rootPath,
                        new RuntimeException(result.getErrorMessage())
                    );
                }
            } catch (Exception e) {
                mismatchReporter.reportRustError("watch:" + rootPath, e);
                LOGGER.debug("[substrate:watch] shadow watch start failed for {}", rootPath, e);
            }
        }
    }

    /**
     * Stop watching a directory.
     */
    public void stopWatching(String rootPath) {
        String watchId = activeWatches.remove(rootPath);
        if (watchId != null && rustClient != null) {
            try {
                rustClient.stopWatching(watchId);
                LOGGER.debug("[substrate:watch] shadow watch stopped for {} (id={})",
                    rootPath, watchId);
            } catch (Exception e) {
                LOGGER.debug("[substrate:watch] shadow watch stop failed for {}", rootPath, e);
            }
        }
    }

    /**
     * Stop all active watches.
     */
    public void stopAll() {
        for (Map.Entry<String, String> entry : activeWatches.entrySet()) {
            if (rustClient != null) {
                try {
                    rustClient.stopWatching(entry.getValue());
                } catch (Exception e) {
                    LOGGER.debug("[substrate:watch] shadow watch stop failed for {}", entry.getKey(), e);
                }
            }
        }
        activeWatches.clear();
    }

    /**
     * Poll for changes and compare with Java-detected changes.
     *
     * @param rootPath the root directory that was watched
     * @param sinceTimestampMs the timestamp to poll from
     * @param javaChanges the Java-detected change set (list of paths)
     */
    public void shadowCompareChanges(
        String rootPath,
        long sinceTimestampMs,
        List<File> javaChanges
    ) {
        String watchId = activeWatches.get(rootPath);
        if (watchId == null || rustClient == null) {
            return;
        }

        try {
            List<RustFileWatchClient.FileChange> rustChanges =
                rustClient.pollChanges(watchId, sinceTimestampMs);

            if (rustChanges.isEmpty() && javaChanges.isEmpty()) {
                mismatchReporter.reportMatch();
                return;
            }

            // Compare the sets of changed paths
            Map<String, RustFileWatchClient.FileChange> rustMap = new HashMap<>();
            for (RustFileWatchClient.FileChange change : rustChanges) {
                rustMap.put(change.getPath(), change);
            }

            boolean allMatch = true;
            for (File javaChange : javaChanges) {
                String javaPath = javaChange.getAbsolutePath();
                if (rustMap.containsKey(javaPath)) {
                    rustMap.remove(javaPath);
                } else {
                    allMatch = false;
                }
            }

            // Check for changes only detected by Rust
            if (!rustMap.isEmpty()) {
                allMatch = false;
            }

            if (allMatch) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:watch] shadow OK: {} changes matched", javaChanges.size());
            } else {
                mismatchReporter.reportRustError(
                    "watch-changes:" + rootPath,
                    new RuntimeException(
                        "change detection mismatch: java=" + javaChanges.size()
                            + " rust=" + rustChanges.size()
                    )
                );
                LOGGER.debug("[substrate:watch] shadow MISMATCH for {}: java={} rust={}",
                    rootPath, javaChanges.size(), rustChanges.size());
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("watch-changes:" + rootPath, e);
            LOGGER.debug("[substrate:watch] shadow comparison failed for {}", rootPath, e);
        }
    }

    /**
     * Get the number of active Rust watches.
     */
    public int getActiveWatchCount() {
        return activeWatches.size();
    }
}
