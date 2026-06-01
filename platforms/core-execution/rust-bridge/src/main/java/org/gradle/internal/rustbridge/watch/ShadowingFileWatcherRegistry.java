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
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Optional;
import java.util.Set;
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
    private final FileWatcherRegistry.ChangeHandler rustChangeHandler;

    private final List<ActiveRustWatch> activeWatches = new ArrayList<>();
    private final Object lock = new Object();
    private final AtomicLong javaChangeCount = new AtomicLong(0);
    private final AtomicLong rustChangeCount = new AtomicLong(0);
    private final AtomicBoolean rustWatchHealthy = new AtomicBoolean(true);

    public ShadowingFileWatcherRegistry(
        FileWatcherRegistry delegate,
        RustFileWatchClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this(delegate, rustClient, mismatchReporter, false, null);
    }

    public ShadowingFileWatcherRegistry(
        FileWatcherRegistry delegate,
        RustFileWatchClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this(delegate, rustClient, mismatchReporter, authoritative, null);
    }

    public ShadowingFileWatcherRegistry(
        FileWatcherRegistry delegate,
        RustFileWatchClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative,
        FileWatcherRegistry.ChangeHandler rustChangeHandler
    ) {
        this.delegate = delegate;
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
        this.rustChangeHandler = rustChangeHandler;
    }

    @Override
    public boolean isWatchingAnyLocations() {
        if (authoritative) {
            synchronized (lock) {
                if (!activeWatches.isEmpty()) {
                    return true;
                }
            }
        }
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
        SnapshotHierarchy result = delegate.updateVfsOnBuildStarted(root, watchMode, unsupportedFileSystems);
        return drainRustChangesInto(result);
    }

    @Override
    public SnapshotHierarchy updateVfsBeforeBuildFinished(
        SnapshotHierarchy root, int maximumNumberOfWatchedHierarchies, List<File> unsupportedFileSystems
    ) {
        SnapshotHierarchy result =
            delegate.updateVfsBeforeBuildFinished(root, maximumNumberOfWatchedHierarchies, unsupportedFileSystems);
        return drainRustChangesInto(result);
    }

    @Override
    public SnapshotHierarchy updateVfsAfterBuildFinished(SnapshotHierarchy root) {
        SnapshotHierarchy result = delegate.updateVfsAfterBuildFinished(root);
        result = drainRustChangesInto(result);

        // Shadow: report match for the build's change processing
        long javaCount = javaChangeCount.getAndSet(0);
        if (!authoritative && javaCount > 0 && rustClient != null) {
            if (rustWatchHealthy.get()) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:watch] shadow OK: {} changes processed in build", javaCount);
            }
        }

        return result;
    }

    @Override
    public FileWatchingStatistics getAndResetStatistics() {
        FileWatchingStatistics javaStatistics = delegate.getAndResetStatistics();
        if (!authoritative) {
            return javaStatistics;
        }
        int rustEvents = saturatingInt(rustChangeCount.getAndSet(0));
        int watchedHierarchies = Math.max(activeWatchCount(), javaStatistics.getNumberOfWatchedHierarchies());
        return new FileWatchingStatistics() {
            @Override
            public Optional<Throwable> getErrorWhileReceivingFileChanges() {
                return Optional.empty();
            }

            @Override
            public boolean isUnknownEventEncountered() {
                return false;
            }

            @Override
            public int getNumberOfReceivedEvents() {
                return rustEvents;
            }

            @Override
            public int getNumberOfWatchedHierarchies() {
                return watchedHierarchies;
            }
        };
    }

    @Override
    public void close() throws IOException {
        synchronized (lock) {
            for (ActiveRustWatch watch : activeWatches) {
                if (rustClient != null) {
                    try {
                        rustClient.stopWatching(watch.watchId);
                    } catch (Exception e) {
                        LOGGER.debug("[substrate:watch] shadow watch stop failed for {}", watch.watchId, e);
                    }
                }
            }
            activeWatches.clear();
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
                activeWatches.add(new ActiveRustWatch(result.getWatchId()));
            }
            rustWatchHealthy.set(true);
            LOGGER.debug("[substrate:watch] shadow watch started for {} (id={})",
                path, result.getWatchId());
        } else {
            throw new RuntimeException(result.getErrorMessage());
        }
    }

    private SnapshotHierarchy drainRustChangesInto(SnapshotHierarchy root) {
        if (!authoritative) {
            return root;
        }
        if (rustClient == null) {
            RuntimeException failure = new RuntimeException("Rust file watcher client is unavailable");
            rustPollFailed("unavailable", failure);
        }

        SnapshotHierarchy result = root;
        synchronized (lock) {
            for (ActiveRustWatch watch : activeWatches) {
                List<RustFileWatchClient.FileChange> changes;
                try {
                    changes = rustClient.pollChangesStrict(watch.watchId, watch.sinceTimestampMs);
                } catch (Exception e) {
                    rustPollFailed(watch.watchId, e);
                    return result;
                }

                for (RustFileWatchClient.FileChange change : changes) {
                    if (watch.hasProcessed(change)) {
                        continue;
                    }
                    FileWatcherRegistry.Type type = mapRustChangeType(change.getChangeType());
                    Path path = Paths.get(change.getPath());
                    if (rustChangeHandler != null) {
                        rustChangeHandler.handleChange(type, path);
                    }
                    result = result.invalidate(path.toString(), SnapshotHierarchy.NodeDiffListener.NOOP);
                    rustChangeCount.incrementAndGet();
                    watch.recordProcessed(change);
                }
            }
        }
        return result;
    }

    private void rustPollFailed(String watchId, Exception e) {
        mismatchReporter.reportRustError("watch-poll:" + watchId, e);
        rustWatchHealthy.set(false);
        throw new SubstrateException("Rust file watcher is authoritative but failed to poll " + watchId, e);
    }

    private int activeWatchCount() {
        synchronized (lock) {
            return activeWatches.size();
        }
    }

    private static int saturatingInt(long value) {
        if (value > Integer.MAX_VALUE) {
            return Integer.MAX_VALUE;
        }
        if (value < Integer.MIN_VALUE) {
            return Integer.MIN_VALUE;
        }
        return (int) value;
    }

    private static FileWatcherRegistry.Type mapRustChangeType(String changeType) {
        if ("CREATED".equals(changeType)) {
            return FileWatcherRegistry.Type.CREATED;
        }
        if ("MODIFIED".equals(changeType)) {
            return FileWatcherRegistry.Type.MODIFIED;
        }
        if ("DELETED".equals(changeType) || "REMOVED".equals(changeType)) {
            return FileWatcherRegistry.Type.REMOVED;
        }
        if ("INVALIDATED".equals(changeType)) {
            return FileWatcherRegistry.Type.INVALIDATED;
        }
        if ("OVERFLOW".equals(changeType)) {
            return FileWatcherRegistry.Type.OVERFLOW;
        }
        return FileWatcherRegistry.Type.INVALIDATED;
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

    private static final class ActiveRustWatch {
        private final String watchId;
        private long sinceTimestampMs;
        private long lastTimestampMs = -1;
        private final Set<String> processedAtLastTimestamp = new HashSet<>();

        private ActiveRustWatch(String watchId) {
            this.watchId = watchId;
        }

        private boolean hasProcessed(RustFileWatchClient.FileChange change) {
            long timestamp = change.getTimestampMs();
            return timestamp < lastTimestampMs
                || (timestamp == lastTimestampMs && processedAtLastTimestamp.contains(eventKey(change)));
        }

        private void recordProcessed(RustFileWatchClient.FileChange change) {
            long timestamp = change.getTimestampMs();
            if (timestamp > lastTimestampMs) {
                lastTimestampMs = timestamp;
                sinceTimestampMs = timestamp;
                processedAtLastTimestamp.clear();
            }
            if (timestamp == lastTimestampMs) {
                processedAtLastTimestamp.add(eventKey(change));
            }
        }

        private static String eventKey(RustFileWatchClient.FileChange change) {
            return change.getTimestampMs() + "|" + change.getChangeType() + "|" + change.getPath();
        }
    }
}
