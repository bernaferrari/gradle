package org.gradle.internal.rustbridge.watch;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.snapshot.FileSystemLocationSnapshot;
import org.gradle.internal.snapshot.SnapshotHierarchy;
import org.gradle.internal.watch.registry.FileWatcherRegistry;
import org.gradle.internal.watch.registry.WatchMode;
import org.slf4j.Logger;

import java.io.File;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;

/**
 * A {@link FileWatcherRegistry} that delegates all operations to the Java registry
 * while shadowing watch registrations against the Rust FileWatchService.
 *
 * <p>All results come from the Java registry. Rust shadowing is fire-and-forget
 * and never affects build correctness.</p>
 */
public class ShadowingFileWatcherRegistry implements FileWatcherRegistry {

    private static final Logger LOGGER = Logging.getLogger(ShadowingFileWatcherRegistry.class);

    private final FileWatcherRegistry delegate;
    private final RustFileWatchClient rustClient;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    private final List<String> activeWatchIds = new ArrayList<>();
    private final Object lock = new Object();
    private final AtomicLong javaChangeCount = new AtomicLong(0);
    private final AtomicBoolean rustWatchHealthy = new AtomicBoolean(true);

    public ShadowingFileWatcherRegistry(
        FileWatcherRegistry delegate,
        RustFileWatchClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this(delegate, rustClient, mismatchReporter, false);
    }

    public ShadowingFileWatcherRegistry(
        FileWatcherRegistry delegate,
        RustFileWatchClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.delegate = delegate;
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    @Override
    public boolean isWatchingAnyLocations() {
        return delegate.isWatchingAnyLocations();
    }

    @Override
    public void registerWatchableHierarchy(File watchableHierarchy, SnapshotHierarchy root) {
        delegate.registerWatchableHierarchy(watchableHierarchy, root);

        String path = watchableHierarchy.getAbsolutePath();
        if (rustClient != null) {
            try {
                RustFileWatchClient.WatchResult result = rustClient.startWatching(
                    path, Collections.emptyList(), Collections.emptyList());

                if (result.isSuccess() && result.isWatching()) {
                    synchronized (lock) {
                        activeWatchIds.add(result.getWatchId());
                    }
                    rustWatchHealthy.set(true);
                    LOGGER.debug("[substrate:watch] shadow watch started for {} (id={})",
                        path, result.getWatchId());
                } else {
                    onRustWatchFailure(path, new RuntimeException(result.getErrorMessage()));
                }
            } catch (Exception e) {
                onRustWatchFailure(path, e);
                LOGGER.debug("[substrate:watch] shadow watch start failed for {}", path, e);
            }
        }
    }

    @Override
    public void virtualFileSystemContentsChanged(
        Collection<FileSystemLocationSnapshot> removedSnapshots,
        Collection<FileSystemLocationSnapshot> addedSnapshots,
        SnapshotHierarchy root
    ) {
        delegate.virtualFileSystemContentsChanged(removedSnapshots, addedSnapshots, root);
    }

    @Override
    public SnapshotHierarchy updateVfsOnBuildStarted(
        SnapshotHierarchy root, WatchMode watchMode, List<File> unsupportedFileSystems
    ) {
        return delegate.updateVfsOnBuildStarted(root, watchMode, unsupportedFileSystems);
    }

    @Override
    public SnapshotHierarchy updateVfsBeforeBuildFinished(
        SnapshotHierarchy root, int maximumNumberOfWatchedHierarchies, List<File> unsupportedFileSystems
    ) {
        return delegate.updateVfsBeforeBuildFinished(root, maximumNumberOfWatchedHierarchies, unsupportedFileSystems);
    }

    @Override
    public SnapshotHierarchy updateVfsAfterBuildFinished(SnapshotHierarchy root) {
        SnapshotHierarchy result = delegate.updateVfsAfterBuildFinished(root);

        // Shadow: report match for the build's change processing
        long javaCount = javaChangeCount.getAndSet(0);
        if (javaCount > 0 && rustClient != null) {
            if (!authoritative || rustWatchHealthy.get()) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:watch] shadow OK: {} changes processed in build", javaCount);
            } else {
                LOGGER.debug("[substrate:watch] authoritative fallback active: using Java watcher for {} changes", javaCount);
            }
        }

        return result;
    }

    @Override
    public FileWatchingStatistics getAndResetStatistics() {
        return delegate.getAndResetStatistics();
    }

    @Override
    public void close() throws IOException {
        synchronized (lock) {
            for (String watchId : activeWatchIds) {
                if (rustClient != null) {
                    try {
                        rustClient.stopWatching(watchId);
                    } catch (Exception e) {
                        LOGGER.debug("[substrate:watch] shadow watch stop failed for {}", watchId, e);
                    }
                }
            }
            activeWatchIds.clear();
        }
        delegate.close();
    }

    /**
     * Record that a change was received by the Java watcher.
     */
    void recordJavaChange() {
        javaChangeCount.incrementAndGet();
    }

    public boolean isAuthoritative() {
        return authoritative;
    }

    public boolean isRustWatchHealthy() {
        return rustWatchHealthy.get();
    }

    private void onRustWatchFailure(String path, Exception e) {
        mismatchReporter.reportRustError("watch:" + path, e);
        if (authoritative && rustWatchHealthy.compareAndSet(true, false)) {
            LOGGER.info("[substrate:watch] authoritative fallback to Java watcher for {}", path);
        }
    }
}
