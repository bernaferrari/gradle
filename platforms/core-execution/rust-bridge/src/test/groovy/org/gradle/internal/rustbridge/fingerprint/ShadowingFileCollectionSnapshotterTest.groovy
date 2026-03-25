package org.gradle.internal.rustbridge.fingerprint

import org.gradle.api.internal.file.FileCollectionInternal
import org.gradle.api.internal.file.FileCollectionStructureVisitor
import org.gradle.internal.execution.FileCollectionSnapshotter
import org.gradle.internal.file.FileMetadata
import org.gradle.internal.file.impl.DefaultFileMetadata
import org.gradle.internal.hash.HashCode
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import org.gradle.internal.snapshot.FileSystemSnapshot
import org.gradle.internal.snapshot.RegularFileSnapshot
import org.gradle.internal.snapshot.SnapshotVisitResult
import spock.lang.Specification

class ShadowingFileCollectionSnapshotterTest extends Specification {

    def "constructor defaults to non-authoritative mode"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        then:
        !snapshotter.isAuthoritative()
    }

    def "four-arg constructor sets authoritative mode"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter, true)

        then:
        snapshotter.isAuthoritative()
    }

    def "snapshot returns java result and skips rust when file collection is empty"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def javaSnapshot = Mock(FileSystemSnapshot)
        def file = emptyFileCollection()
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        when:
        def result = snapshotter.snapshot(file)

        then:
        1 * javaDelegate.snapshot(file, FileCollectionStructureVisitor.NO_OP) >> javaSnapshot
        0 * rustClient._
        result.is(javaSnapshot)
    }

    def "snapshot invokes rust fingerprinting when collection has files"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def javaSnapshot = Mock(FileSystemSnapshot)
        def rustResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def tmp = File.createTempFile("shadowing-fp", ".txt")
        tmp.deleteOnExit()
        tmp.text = "x"
        def file = singleFileCollection(tmp.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        when:
        def result = snapshotter.snapshot(file)

        then:
        1 * javaDelegate.snapshot(file, FileCollectionStructureVisitor.NO_OP) >> javaSnapshot
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> rustResult
        1 * rustResult.isSuccess() >> true
        1 * rustResult.getEntries() >> []
        result.is(javaSnapshot)
    }

    def "snapshot reports match when java and rust file hashes agree"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def javaSnapshot = Mock(FileSystemSnapshot)
        def rustResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def rustEntry = Mock(RustFileFingerprintClient.IndividualFingerprint)
        def hash = HashCode.fromBytes("match".bytes)
        def tmp = File.createTempFile("shadowing-fp-match", ".txt")
        tmp.deleteOnExit()
        tmp.text = "x"
        def file = singleFileCollection(tmp.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        rustEntry.isDirectory() >> false
        rustEntry.getPath() >> tmp.absolutePath
        rustEntry.getHash() >> hash

        javaSnapshot.accept(_) >> { args ->
            def visitor = args[0]
            visitor.visitEntry(new RegularFileSnapshot(
                tmp.absolutePath,
                tmp.name,
                hash,
                DefaultFileMetadata.file(tmp.lastModified(), tmp.length(), FileMetadata.AccessType.DIRECT)
            ))
            SnapshotVisitResult.CONTINUE
        }

        when:
        snapshotter.snapshot(file)

        then:
        1 * javaDelegate.snapshot(file, FileCollectionStructureVisitor.NO_OP) >> javaSnapshot
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> rustResult
        1 * rustResult.isSuccess() >> true
        1 * rustResult.getEntries() >> [rustEntry]
        1 * reporter.reportMatch()
    }

    def "snapshot handles rust exception gracefully"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def javaSnapshot = Mock(FileSystemSnapshot)
        def tmp = File.createTempFile("shadowing-fp-error", ".txt")
        tmp.deleteOnExit()
        tmp.text = "x"
        def file = singleFileCollection(tmp.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter)

        when:
        def result = snapshotter.snapshot(file)

        then:
        1 * javaDelegate.snapshot(file, FileCollectionStructureVisitor.NO_OP) >> javaSnapshot
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> { throw new RuntimeException("grpc down") }
        noExceptionThrown()
        result.is(javaSnapshot)
    }

    private FileCollectionInternal emptyFileCollection() {
        def file = Mock(FileCollectionInternal)
        file.visitStructure(_) >> { args ->
            def visitor = args[0] as FileCollectionStructureVisitor
            visitor.visitCollection(null, [])
        }
        file
    }

    private FileCollectionInternal singleFileCollection(String path) {
        def file = Mock(FileCollectionInternal)
        file.visitStructure(_) >> { args ->
            def visitor = args[0] as FileCollectionStructureVisitor
            visitor.visitCollection(null, [new File(path)])
        }
        file
    }
}
