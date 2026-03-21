package org.gradle.internal.rustbridge.shadow

import spock.lang.Specification

class HashMismatchReporterTest extends Specification {

    def "logSummary is no-op when no comparisons have been made"() {
        given:
        def reporter = new HashMismatchReporter(true)

        when:
        reporter.logSummary() // should not throw

        then:
        noExceptionThrown()
    }

    def "tracks matches and mismatches"() {
        given:
        def reporter = new HashMismatchReporter(false)

        when:
        reporter.reportMatch()
        reporter.reportMatch()
        reporter.reportMismatch("path1", null, null)

        then:
        def summary = reporter.getSummary()
        summary.totalComparisons == 3
        summary.matchCount == 2
        summary.mismatchCount == 1
        summary.hasMismatches()
    }

    def "tracks Rust errors"() {
        given:
        def reporter = new HashMismatchReporter(false)

        when:
        reporter.reportRustError("path1", new RuntimeException("timeout"))

        then:
        reporter.getSummary().rustErrorCount == 1
    }

    def "tracks Java errors"() {
        given:
        def reporter = new HashMismatchReporter(false)

        when:
        reporter.reportJavaError("path1", new RuntimeException("IO error"))

        then:
        reporter.getSummary().javaErrorCount == 1
    }
}
