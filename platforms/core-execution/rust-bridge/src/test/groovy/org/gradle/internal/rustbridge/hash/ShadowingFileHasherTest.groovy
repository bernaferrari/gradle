package org.gradle.internal.rustbridge.hash

import org.gradle.internal.hash.FileHasher
import org.gradle.internal.hash.HashCode
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification


class ShadowingFileHasherTest extends Specification {

    def "implements FileHasher interface"() {
        expect:
        ShadowingFileHasher instanceof FileHasher
    }

    def "constructor stores all three delegates"() {
        given:
        def javaDelegate = Mock(FileHasher)
        def rustDelegate = Mock(FileHasher)
        def reporter = Mock(HashMismatchReporter)

        when:
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter)

        then:
        hasher.javaDelegate.is(javaDelegate)
        hasher.rustDelegate.is(rustDelegate)
        hasher.mismatchReporter.is(reporter)
    }

    def "hash(File) returns Java result and reports match"() {
        given:
        def javaHash = HashCode.fromInt(42)
        def rustHash = HashCode.fromInt(42)
        def javaDelegate = Mock(FileHasher) {
            hash(_) >> javaHash
        }
        def rustDelegate = Mock(FileHasher) {
            hash(_) >> rustHash
        }
        def reporter = Mock(HashMismatchReporter)
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter)
        def file = new File("/tmp/test.txt")

        when:
        def result = hasher.hash(file)

        then:
        result.is(javaHash)
        1 * reporter.reportMatch()
        0 * reporter.reportMismatch(_, _, _)
        0 * reporter.reportRustError(_, _)
    }

    def "hash(File) reports mismatch when hashes differ"() {
        given:
        def javaHash = HashCode.fromInt(42)
        def rustHash = HashCode.fromInt(99)
        def javaDelegate = Mock(FileHasher) {
            hash(_) >> javaHash
        }
        def rustDelegate = Mock(FileHasher) {
            hash(_) >> rustHash
        }
        def reporter = Mock(HashMismatchReporter)
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter)
        def file = new File("/tmp/test.txt")

        when:
        def result = hasher.hash(file)

        then:
        result.is(javaHash)
        1 * reporter.reportMismatch(file.getAbsolutePath(), javaHash, rustHash)
        0 * reporter.reportMatch()
    }

    def "hash(File) reports Rust error and still returns Java result"() {
        given:
        def javaHash = HashCode.fromInt(42)
        def javaDelegate = Mock(FileHasher) {
            hash(_) >> javaHash
        }
        def rustDelegate = Mock(FileHasher) {
            hash(_) >> { throw new RuntimeException("Rust daemon unavailable") }
        }
        def reporter = Mock(HashMismatchReporter)
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter)
        def file = new File("/tmp/test.txt")

        when:
        def result = hasher.hash(file)

        then:
        result.is(javaHash)
        1 * reporter.reportRustError(file.getAbsolutePath(), _ as RuntimeException)
        0 * reporter.reportMatch()
        0 * reporter.reportMismatch(_, _, _)
    }

    def "hash(File, long, long) returns Java result and reports match"() {
        given:
        def javaHash = HashCode.fromInt(42)
        def rustHash = HashCode.fromInt(42)
        def javaDelegate = Mock(FileHasher) {
            hash(_, _, _) >> javaHash
        }
        def rustDelegate = Mock(FileHasher) {
            hash(_, _, _) >> rustHash
        }
        def reporter = Mock(HashMismatchReporter)
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter)
        def file = new File("/tmp/test.txt")

        when:
        def result = hasher.hash(file, 1024L, 9999L)

        then:
        result.is(javaHash)
        1 * reporter.reportMatch()
    }

    def "hash(File, long, long) reports mismatch when hashes differ"() {
        given:
        def javaHash = HashCode.fromInt(10)
        def rustHash = HashCode.fromInt(20)
        def javaDelegate = Mock(FileHasher) {
            hash(_, _, _) >> javaHash
        }
        def rustDelegate = Mock(FileHasher) {
            hash(_, _, _) >> rustHash
        }
        def reporter = Mock(HashMismatchReporter)
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter)
        def file = new File("/tmp/test.txt")

        when:
        def result = hasher.hash(file, 1024L, 9999L)

        then:
        result.is(javaHash)
        1 * reporter.reportMismatch(file.getAbsolutePath(), javaHash, rustHash)
    }

    def "hash(File, long, long) reports Rust error and still returns Java result"() {
        given:
        def javaHash = HashCode.fromInt(42)
        def javaDelegate = Mock(FileHasher) {
            hash(_, _, _) >> javaHash
        }
        def rustDelegate = Mock(FileHasher) {
            hash(_, _, _) >> { throw new RuntimeException("connection reset") }
        }
        def reporter = Mock(HashMismatchReporter)
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter)
        def file = new File("/tmp/test.txt")

        when:
        def result = hasher.hash(file, 2048L, 12345L)

        then:
        result.is(javaHash)
        1 * reporter.reportRustError(file.getAbsolutePath(), _ as RuntimeException)
        0 * reporter.reportMatch()
        0 * reporter.reportMismatch(_, _, _)
    }

    // ---- Authoritative mode tests ----

    def "four-arg constructor stores authoritative flag"() {
        given:
        def javaDelegate = Mock(FileHasher)
        def rustDelegate = Mock(FileHasher)
        def reporter = Mock(HashMismatchReporter)

        when:
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter, true)

        then:
        hasher.isAuthoritative()
    }

    def "three-arg constructor defaults to non-authoritative"() {
        given:
        def javaDelegate = Mock(FileHasher)
        def rustDelegate = Mock(FileHasher)
        def reporter = Mock(HashMismatchReporter)

        when:
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter)

        then:
        !hasher.isAuthoritative()
    }

    def "authoritative hash(File) returns Rust result"() {
        given:
        def javaHash = HashCode.fromInt(10)
        def rustHash = HashCode.fromInt(20)
        def javaDelegate = Mock(FileHasher) {
            hash(_) >> javaHash
        }
        def rustDelegate = Mock(FileHasher) {
            hash(_) >> rustHash
        }
        def reporter = Mock(HashMismatchReporter)
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter, true)
        def file = new File("/tmp/test.txt")

        when:
        def result = hasher.hash(file)

        then:
        result.is(rustHash)
        1 * reporter.reportMismatch(file.getAbsolutePath(), javaHash, rustHash)
    }

    def "authoritative hash(File) falls back to Java on Rust failure"() {
        given:
        def javaHash = HashCode.fromInt(42)
        def javaDelegate = Mock(FileHasher) {
            hash(_) >> javaHash
        }
        def rustDelegate = Mock(FileHasher) {
            hash(_) >> { throw new RuntimeException("Rust unavailable") }
        }
        def reporter = Mock(HashMismatchReporter)
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter, true)
        def file = new File("/tmp/test.txt")

        when:
        def result = hasher.hash(file)

        then:
        result.is(javaHash)
        1 * reporter.reportRustError(file.getAbsolutePath(), _)
    }

    def "authoritative hash(File, long, long) returns Rust result"() {
        given:
        def javaHash = HashCode.fromInt(10)
        def rustHash = HashCode.fromInt(20)
        def javaDelegate = Mock(FileHasher) {
            hash(_, _, _) >> javaHash
        }
        def rustDelegate = Mock(FileHasher) {
            hash(_, _, _) >> rustHash
        }
        def reporter = Mock(HashMismatchReporter)
        def hasher = new ShadowingFileHasher(javaDelegate, rustDelegate, reporter, true)
        def file = new File("/tmp/test.txt")

        when:
        def result = hasher.hash(file, 1024L, 9999L)

        then:
        result.is(rustHash)
    }
}
