package org.gradle.internal.rustbridge.watch;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateException;
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
 * A {@link FileWatcherRegistry} that delegates operations to the Java registry
 * while shadowing watch registrations against the Rust FileWatchService.
 *
 * <p>All results come from the Java registry. Rust shadowing is fire-and-forget
 * and never affects build correctness in shadow mode. In authoritative mode,
 * Rust watch startup failure aborts registration instead of silently falling
 * back to the Java watcher.</p>
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
        String path = watchableHierarchy.getAbsolutePath();
        if (authoritative) {
            startRustWatchOrFail(path);
            delegate.registerWatchableHierarchy(watchableHierarchy, root);
            return;
        }

        delegate.registerWatchableHierarchy(watchableHierarchy, root);

        if (rustClient != null) {
            try {
                startRustWatch(path);
            } catch (Exception e) {
                onRustWatchFailure(path, e, false);
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
            if (rustWatchHealthy.get()) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:watch] shadow OK: {} changes processed in build", javaCount);
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

    private void startRustWatchOrFail(String path) {
        if (rustClient == null) {
            RuntimeException failure = new RuntimeException("Rust file watcher client is unavailable");
            onRustWatchFailure(path, failure, true);
            throw new SubstrateException("Rust file watcher is authoritative but unavailable for " + path, failure);
        }
        try {
            startRustWatch(path);
        } catch (Exception e) {
            onRustWatchFailure(path, e, true);
            throw new SubstrateException("Rust file watcher is authoritative but failed to watch " + path, e);
        }
    }

    private void startRustWatch(String path) {
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
            throw new RuntimeException(result.getErrorMessage());
        }
    }

    private void onRustWatchFailure(String path, Exception e, boolean failClosed) {
        mismatchReporter.reportRustError("watch:" + path, e);
        if (rustWatchHealthy.compareAndSet(true, false)) {
            if (failClosed) {
                LOGGER.info("[substrate:watch] authoritative Rust watcher failed for {}", path);
            } else {
                LOGGER.debug("[substrate:watch] shadow watch start failed for {}", path, e);
            }
        }
    }
}
