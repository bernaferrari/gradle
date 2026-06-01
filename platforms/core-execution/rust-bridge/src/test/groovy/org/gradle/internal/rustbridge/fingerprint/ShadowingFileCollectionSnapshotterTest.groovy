package org.gradle.internal.rustbridge.fingerprint

import org.gradle.api.internal.file.FileCollectionInternal
import org.gradle.api.internal.file.FileCollectionStructureVisitor
import org.gradle.api.internal.file.FileTreeInternal
import org.gradle.internal.execution.FileCollectionSnapshotter
import org.gradle.internal.file.FileMetadata
import org.gradle.internal.file.impl.DefaultFileMetadata
import org.gradle.internal.hash.HashCode
import org.gradle.internal.rustbridge.SubstrateException
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import org.gradle.internal.snapshot.DirectorySnapshot
import org.gradle.internal.snapshot.FileSystemSnapshot
import org.gradle.internal.snapshot.MissingFileSnapshot
import org.gradle.internal.snapshot.RegularFileSnapshot
import org.gradle.internal.snapshot.SnapshotVisitResult
import org.junit.Assume
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

    def "authoritative snapshot builds regular file snapshots from rust without java delegate"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def rustResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def rustEntry = Mock(RustFileFingerprintClient.IndividualFingerprint)
        def hash = HashCode.fromBytes("authoritative-md5".bytes)
        def tmp = File.createTempFile("authoritative-fp", ".txt")
        tmp.deleteOnExit()
        tmp.text = "x"
        def file = singleFileCollection(tmp.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter, true)

        rustEntry.isDirectory() >> false
        rustEntry.getPath() >> tmp.absolutePath
        rustEntry.getHash() >> hash
        rustEntry.getLastModified() >> 123L
        rustEntry.getSize() >> 1L

        when:
        def result = snapshotter.snapshot(file)

        then:
        0 * javaDelegate._
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> rustResult
        1 * rustResult.isSuccess() >> true
        1 * rustResult.getEntries() >> [rustEntry]

        and:
        def roots = result.roots().toList()
        roots.size() == 1
        roots[0] instanceof RegularFileSnapshot
        roots[0].absolutePath == tmp.absolutePath
        roots[0].hash == hash
    }

    def "authoritative snapshot marks file symlink snapshots as accessed via symlink"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def rustResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def rustEntry = Mock(RustFileFingerprintClient.IndividualFingerprint)
        def hash = HashCode.fromBytes("symlink-md5---".bytes)
        def dir = File.createTempDir("authoritative-fp-symlink", "")
        def target = new File(dir, "target.txt")
        def link = new File(dir, "link.txt")
        target.text = "linked"
        createSymlinkOrSkip(link, target)
        def file = singleFileCollection(link.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter, true)

        rustEntry.isDirectory() >> false
        rustEntry.getPath() >> link.absolutePath
        rustEntry.getHash() >> hash
        rustEntry.getLastModified() >> 789L
        rustEntry.getSize() >> target.length()

        when:
        def result = snapshotter.snapshot(file)

        then:
        0 * javaDelegate._
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> rustResult
        1 * rustResult.isSuccess() >> true
        1 * rustResult.getEntries() >> [rustEntry]

        and:
        def roots = result.roots().toList()
        roots.size() == 1
        roots[0] instanceof RegularFileSnapshot
        roots[0].absolutePath == link.absolutePath
        roots[0].accessType == FileMetadata.AccessType.VIA_SYMLINK
        roots[0].hash == hash
    }

    def "authoritative snapshot represents broken symlink as missing via symlink without rust hash"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def dir = File.createTempDir("authoritative-fp-broken-symlink", "")
        def missingTarget = new File(dir, "missing.txt")
        def link = new File(dir, "broken.txt")
        createSymlinkOrSkip(link, missingTarget)
        def file = singleFileCollection(link.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter, true)

        when:
        def result = snapshotter.snapshot(file)

        then:
        0 * javaDelegate._
        0 * rustClient._

        and:
        def roots = result.roots().toList()
        roots.size() == 1
        roots[0] instanceof MissingFileSnapshot
        roots[0].absolutePath == link.absolutePath
        roots[0].accessType == FileMetadata.AccessType.VIA_SYMLINK
    }

    def "authoritative snapshot builds directory snapshots from rust file hashes"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def rustResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def rustEntry = Mock(RustFileFingerprintClient.IndividualFingerprint)
        def hash = HashCode.fromBytes("directory-md5---".bytes)
        def dir = File.createTempDir("authoritative-fp-dir", "")
        def child = new File(dir, "child.txt")
        child.text = "x"
        child.deleteOnExit()
        dir.deleteOnExit()
        def file = singleFileCollection(dir.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter, true)

        rustEntry.isDirectory() >> false
        rustEntry.getPath() >> child.absolutePath
        rustEntry.getHash() >> hash
        rustEntry.getLastModified() >> 123L
        rustEntry.getSize() >> 1L

        when:
        def result = snapshotter.snapshot(file)

        then:
        0 * javaDelegate._
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> rustResult
        1 * rustResult.isSuccess() >> true
        1 * rustResult.getEntries() >> [rustEntry]

        and:
        def roots = result.roots().toList()
        roots.size() == 1
        roots[0] instanceof DirectorySnapshot
        roots[0].children.size() == 1
        roots[0].children[0] instanceof RegularFileSnapshot
        roots[0].children[0].absolutePath == child.absolutePath
        roots[0].children[0].hash == hash
    }

    def "authoritative snapshot supports directory symlink roots"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def rustResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def rustEntry = Mock(RustFileFingerprintClient.IndividualFingerprint)
        def hash = HashCode.fromBytes("dir-symlink-md5".bytes)
        def dir = File.createTempDir("authoritative-fp-dir-symlink", "")
        def target = new File(dir, "target")
        def link = new File(dir, "link")
        target.mkdirs()
        new File(target, "child.txt").text = "linked child"
        createSymlinkOrSkip(link, target)
        def linkChild = new File(link, "child.txt")
        def file = singleFileCollection(link.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter, true)

        rustEntry.isDirectory() >> false
        rustEntry.getPath() >> linkChild.absolutePath
        rustEntry.getHash() >> hash
        rustEntry.getLastModified() >> 321L
        rustEntry.getSize() >> linkChild.length()

        when:
        def result = snapshotter.snapshot(file)

        then:
        0 * javaDelegate._
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> rustResult
        1 * rustResult.isSuccess() >> true
        1 * rustResult.getEntries() >> [rustEntry]

        and:
        def roots = result.roots().toList()
        roots.size() == 1
        roots[0] instanceof DirectorySnapshot
        roots[0].absolutePath == link.absolutePath
        roots[0].accessType == FileMetadata.AccessType.VIA_SYMLINK
        roots[0].children.size() == 1
        roots[0].children[0] instanceof RegularFileSnapshot
        roots[0].children[0].absolutePath == linkChild.absolutePath
        roots[0].children[0].hash == hash
    }

    def "authoritative snapshot fails closed on directory symlink cycle"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def dir = File.createTempDir("authoritative-fp-dir-cycle", "")
        def loop = new File(dir, "loop")
        createSymlinkOrSkip(loop, dir)
        def file = singleFileCollection(dir.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter, true)

        when:
        snapshotter.snapshot(file)

        then:
        0 * javaDelegate._
        0 * rustClient._
        def e = thrown(SubstrateException)
        e.message.contains("directory symlink cycle")
    }

    def "authoritative snapshot hashes file-tree-backed backing files with rust"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def rustResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def rustEntry = Mock(RustFileFingerprintClient.IndividualFingerprint)
        def hash = HashCode.fromBytes("archive-md5----".bytes)
        def backingFile = File.createTempFile("authoritative-fp-file-tree-backed", ".zip")
        backingFile.deleteOnExit()
        backingFile.text = "zip bytes"
        def file = fileTreeBackedByFileCollection(backingFile.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter, true)

        rustEntry.isDirectory() >> false
        rustEntry.getPath() >> backingFile.absolutePath
        rustEntry.getHash() >> hash
        rustEntry.getLastModified() >> 456L
        rustEntry.getSize() >> backingFile.length()

        when:
        def result = snapshotter.snapshot(file)

        then:
        0 * javaDelegate._
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> rustResult
        1 * rustResult.isSuccess() >> true
        1 * rustResult.getEntries() >> [rustEntry]

        and:
        def roots = result.roots().toList()
        roots.size() == 1
        roots[0] instanceof RegularFileSnapshot
        roots[0].absolutePath == backingFile.absolutePath
        roots[0].hash == hash
    }

    def "authoritative snapshot fails closed when rust omits requested file"() {
        given:
        def javaDelegate = Mock(FileCollectionSnapshotter)
        def rustClient = Mock(RustFileFingerprintClient)
        def reporter = Mock(HashMismatchReporter)
        def rustResult = Mock(RustFileFingerprintClient.FingerprintResult)
        def tmp = File.createTempFile("authoritative-fp-missing", ".txt")
        tmp.deleteOnExit()
        tmp.text = "x"
        def file = singleFileCollection(tmp.absolutePath)
        def snapshotter = new ShadowingFileCollectionSnapshotter(javaDelegate, rustClient, reporter, true)

        when:
        snapshotter.snapshot(file)

        then:
        0 * javaDelegate._
        1 * rustClient.fingerprintFiles(_, "ABSOLUTE_PATH", []) >> rustResult
        1 * rustResult.isSuccess() >> true
        1 * rustResult.getEntries() >> []
        def e = thrown(SubstrateException)
        e.message.contains("returned no entry")
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

    private FileCollectionInternal fileTreeBackedByFileCollection(String path) {
        def file = Mock(FileCollectionInternal)
        file.visitStructure(_) >> { args ->
            def visitor = args[0] as FileCollectionStructureVisitor
            visitor.visitFileTreeBackedByFile(new File(path), Mock(FileTreeInternal), null)
        }
        file
    }

    private static void createSymlinkOrSkip(File link, File target) {
        try {
            java.nio.file.Files.createSymbolicLink(link.toPath(), target.toPath())
        } catch (UnsupportedOperationException | IOException | SecurityException e) {
            Assume.assumeNoException(e)
        }
        Assume.assumeTrue(java.nio.file.Files.isSymbolicLink(link.toPath()))
    }
}
