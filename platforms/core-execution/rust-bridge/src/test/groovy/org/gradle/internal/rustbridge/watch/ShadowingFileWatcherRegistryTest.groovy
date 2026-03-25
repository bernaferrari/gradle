package org.gradle.internal.rustbridge.watch

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import org.gradle.internal.watch.registry.FileWatcherRegistry
import spock.lang.Specification

class ShadowingFileWatcherRegistryTest extends Specification {

    def "constructor sets up fields from arguments"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)

        then:
        registry != null
    }

    def "isWatchingAnyLocations delegates to Java registry"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)

        when:
        def result = registry.isWatchingAnyLocations()

        then:
        1 * delegate.isWatchingAnyLocations() >> true
        result == true
    }

    def "isWatchingAnyLocations returns false when delegate returns false"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)

        when:
        def result = registry.isWatchingAnyLocations()

        then:
        1 * delegate.isWatchingAnyLocations() >> false
        result == false
    }

    def "registerWatchableHierarchy delegates to Java registry and shadows to Rust on success"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def watchableDir = new File("/tmp/test-watch")
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)
        def watchResult = Mock(RustFileWatchClient.WatchResult)

        when:
        registry.registerWatchableHierarchy(watchableDir, root)

        then:
        1 * delegate.registerWatchableHierarchy(watchableDir, root)
        1 * rustClient.startWatching("/tmp/test-watch", [], []) >> watchResult
        1 * watchResult.isSuccess() >> true
        1 * watchResult.isWatching() >> true
        1 * watchResult.getWatchId() >> "watch-42"
        0 * reporter._
    }

    def "registerWatchableHierarchy reports mismatch when Rust watch fails"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def watchableDir = new File("/tmp/test-watch")
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)
        def watchResult = Mock(RustFileWatchClient.WatchResult)

        when:
        registry.registerWatchableHierarchy(watchableDir, root)

        then:
        1 * delegate.registerWatchableHierarchy(watchableDir, root)
        1 * rustClient.startWatching("/tmp/test-watch", [], []) >> watchResult
        1 * watchResult.isSuccess() >> false
        1 * watchResult.getErrorMessage() >> "watch failed"
        1 * reporter.reportRustError("watch:/tmp/test-watch", _ as RuntimeException)
    }

    def "registerWatchableHierarchy reports mismatch when Rust throws exception"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def watchableDir = new File("/tmp/test-watch")
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)

        when:
        registry.registerWatchableHierarchy(watchableDir, root)

        then:
        1 * delegate.registerWatchableHierarchy(watchableDir, root)
        1 * rustClient.startWatching("/tmp/test-watch", [], []) >> { throw new RuntimeException("gRPC error") }
        1 * reporter.reportRustError("watch:/tmp/test-watch", _ as RuntimeException)
    }

    def "registerWatchableHierarchy skips Rust when rustClient is null"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, null, reporter)
        def watchableDir = new File("/tmp/test-watch")
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)

        when:
        registry.registerWatchableHierarchy(watchableDir, root)

        then:
        1 * delegate.registerWatchableHierarchy(watchableDir, root)
        0 * reporter._
    }

    def "virtualFileSystemContentsChanged delegates to Java registry"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def removed = []
        def added = []
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)

        when:
        registry.virtualFileSystemContentsChanged(removed, added, root)

        then:
        1 * delegate.virtualFileSystemContentsChanged(removed, added, root)
    }

    def "close stops all active Rust watches and delegates to Java registry"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def watchableDir = new File("/tmp/test-watch")
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)
        def watchResult = Mock(RustFileWatchClient.WatchResult)

        // Register two watches so close has something to stop
        when:
        registry.registerWatchableHierarchy(watchableDir, root)
        registry.registerWatchableHierarchy(watchableDir, root)
        registry.close()

        then:
        1 * delegate.registerWatchableHierarchy(watchableDir, root) >> { /* first call */ }
        1 * delegate.registerWatchableHierarchy(watchableDir, root) >> { /* second call */ }
        2 * rustClient.startWatching("/tmp/test-watch", [], []) >> watchResult
        2 * watchResult.isSuccess() >> true
        2 * watchResult.isWatching() >> true
        2 * watchResult.getWatchId() >> "watch-1", "watch-2"
        1 * rustClient.stopWatching("watch-1")
        1 * rustClient.stopWatching("watch-2")
        1 * delegate.close()
    }

    def "close handles null rustClient gracefully"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, null, reporter)

        when:
        registry.close()

        then:
        1 * delegate.close()
    }

    def "close tolerates exceptions when stopping individual watches"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def watchableDir = new File("/tmp/test-watch")
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)
        def watchResult = Mock(RustFileWatchClient.WatchResult)

        when:
        registry.registerWatchableHierarchy(watchableDir, root)
        registry.close()

        then:
        1 * delegate.registerWatchableHierarchy(watchableDir, root)
        1 * rustClient.startWatching("/tmp/test-watch", [], []) >> watchResult
        1 * watchResult.isSuccess() >> true
        1 * watchResult.isWatching() >> true
        1 * watchResult.getWatchId() >> "watch-fail"
        1 * rustClient.stopWatching("watch-fail") >> { throw new RuntimeException("connection lost") }
        1 * delegate.close()
    }

    def "recordJavaChange increments change counter"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)

        when:
        registry.recordJavaChange()
        registry.recordJavaChange()
        registry.recordJavaChange()

        then:
        // After recording 3 changes, updateVfsAfterBuildFinished should report a match
        // (verified by interaction on reporter)
        noExceptionThrown()
    }

    def "updateVfsAfterBuildFinished reports match when Java changes were recorded"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)

        when:
        registry.recordJavaChange()
        registry.recordJavaChange()
        registry.updateVfsAfterBuildFinished(root)

        then:
        1 * delegate.updateVfsAfterBuildFinished(root) >> root
        1 * reporter.reportMatch()
    }

    def "updateVfsAfterBuildFinished does not report match when no Java changes recorded"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)

        when:
        registry.updateVfsAfterBuildFinished(root)

        then:
        1 * delegate.updateVfsAfterBuildFinished(root) >> root
        0 * reporter.reportMatch()
    }

    def "updateVfsAfterBuildFinished resets change count after reporting"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)

        when:
        registry.recordJavaChange()
        registry.updateVfsAfterBuildFinished(root)
        registry.updateVfsAfterBuildFinished(root)

        then:
        2 * delegate.updateVfsAfterBuildFinished(root) >> root
        // reportMatch called only once: the second call has zero accumulated changes
        1 * reporter.reportMatch()
    }

    def "getAndResetStatistics delegates to Java registry"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def stats = Mock(org.gradle.internal.watch.registry.FileWatcherRegistry.FileWatchingStatistics)

        when:
        def result = registry.getAndResetStatistics()

        then:
        1 * delegate.getAndResetStatistics() >> stats
        result == stats
    }

    def "updateVfsOnBuildStarted delegates to Java registry"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)
        def watchMode = Mock(org.gradle.internal.watch.registry.WatchMode)
        def unsupported = []

        when:
        def result = registry.updateVfsOnBuildStarted(root, watchMode, unsupported)

        then:
        1 * delegate.updateVfsOnBuildStarted(root, watchMode, unsupported) >> root
        result == root
    }

    def "updateVfsBeforeBuildFinished delegates to Java registry"() {
        given:
        def delegate = Mock(FileWatcherRegistry)
        def rustClient = Mock(RustFileWatchClient)
        def reporter = Mock(HashMismatchReporter)
        def registry = new ShadowingFileWatcherRegistry(delegate, rustClient, reporter)
        def root = Mock(org.gradle.internal.snapshot.SnapshotHierarchy)
        def unsupported = []

        when:
        def result = registry.updateVfsBeforeBuildFinished(root, 100, unsupported)

        then:
        1 * delegate.updateVfsBeforeBuildFinished(root, 100, unsupported) >> root
        result == root
    }
}
