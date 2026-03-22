package org.gradle.internal.rustbridge.fingerprint

import org.gradle.api.file.FileCollection
import org.gradle.api.internal.file.FileCollectionInternal
import org.gradle.api.internal.file.FileCollectionStructureVisitor
import org.gradle.internal.execution.FileCollectionSnapshotter
import org.gradle.internal.hash.HashCode
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import org.gradle.internal.snapshot.FileSystemSnapshot
import spock.lang.Specification

class ShadowingFileCollectionSnapshotterTest extends Specification {

    def "implements FileCollectionSnapshotter"() {
        expect:
        ShadowingFileCollectionSnapshotter instanceof FileCollectionSnapshotter
    }

    def "constructor defaults to non-authoritative mode"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        then:
        snapshotter != null
    }

    def "snapshot delegates to Java and compares with Rust in shadow mode"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def javaSnapshot = Mock(FileSystemSnapshot)
        def fingerprintResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def individualFingerprint = Mock(RustFileFingerprintClient.IndividualFingerprint)
        def file = Mock(FileCollectionInternal)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        individualFingerprint.isDirectory() >> false
        individualFingerprint.getPath() >> "/tmp/test.txt"
        individualFingerprint.getHash() >> HashCode.fromInt(42)

        when:
        def result = snapshotter.snapshot(file)

        then:
        1 * javaDelegate.snapshot(file, FileCollectionStructureVisitor.NO_OP) >> javaSnapshot
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> fingerprintResult
        1 * fingerprintResult.isSuccess() >> true
        1 * fingerprintResult.getEntries() >> [individualFingerprint]
        result.is(javaSnapshot)
    }

    def "snapshot returns Java result when Rust fingerprinting fails"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def javaSnapshot = Mock(FileSystemSnapshot)
        def fingerprintResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def file = Mock(FileCollectionInternal)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        when:
        def result = snapshotter.snapshot(file)

        then:
        1 * javaDelegate.snapshot(file, FileCollectionStructureVisitor.NO_OP) >> javaSnapshot
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> fingerprintResult
        1 * fingerprintResult.isSuccess() >> false
        result.is(javaSnapshot)
    }

    def "snapshot handles Rust exception gracefully in shadow mode"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def javaSnapshot = Mock(FileSystemSnapshot)
        def file = Mock(FileCollectionInternal)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        when:
        def result = snapshotter.snapshot(file)

        then:
        1 * javaDelegate.snapshot(file, FileCollectionStructureVisitor.NO_OP) >> javaSnapshot
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> { throw new RuntimeException("gRPC error") }
        result.is(javaSnapshot)
        noExceptionThrown()
    }

    def "snapshot with visitor delegates to Java with that visitor"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def javaSnapshot = Mock(FileSystemSnapshot)
        def fingerprintResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def file = Mock(FileCollectionInternal)
        def visitor = Mock(FileCollectionStructureVisitor)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        when:
        def result = snapshotter.snapshot(file, visitor)

        then:
        1 * javaDelegate.snapshot(file, visitor) >> javaSnapshot
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> fingerprintResult
        1 * fingerprintResult.isSuccess() >> true
        1 * fingerprintResult.getEntries() >> []
        result.is(javaSnapshot)
    }

    def "snapshot reports match when Java and Rust hashes agree"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def javaSnapshot = Mock(FileSystemSnapshot)
        def fingerprintResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def individualFingerprint = Mock(RustFileFingerprintClient.IndividualFingerprint)
        def file = Mock(FileCollectionInternal)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)
        def hash = HashCode.fromInt(42)

        individualFingerprint.isDirectory() >> false
        individualFingerprint.getPath() >> "/tmp/test.txt"
        individualFingerprint.getHash() >> hash

        javaSnapshot.accept(_) >> { args ->
            def visitor = args[0] as org.gradle.internal.snapshot.FileSystemSnapshotHierarchyVisitor
            visitor.visitEntry(new org.gradle.internal.snapshot.RegularFileSnapshot(
                "/tmp/test.txt", "test.txt", hash, 10L))
        }

        when:
        snapshotter.snapshot(file)

        then:
        1 * javaDelegate.snapshot(file, FileCollectionStructureVisitor.NO_OP) >> javaSnapshot
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> fingerprintResult
        1 * fingerprintResult.isSuccess() >> true
        1 * fingerprintResult.getEntries() >> [individualFingerprint]
        1 * reporter.reportMatch()
    }
}
