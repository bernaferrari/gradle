package org.gradle.internal.rustbridge.shadow

import spock.lang.Specification

class BuildFinishMismatchLoggerTest extends Specification {

    def "logs summary on beforeComplete with no-op when no comparisons"() {
        given:
        def reporter = Mock(HashMismatchReporter)
        def logger = new BuildFinishMismatchLogger(reporter)

        when:
        logger.beforeComplete(null)

        then:
        1 * reporter.logSummary()
    }

    def "logs summary on beforeComplete even with failure"() {
        given:
        def reporter = Mock(HashMismatchReporter)
        def logger = new BuildFinishMismatchLogger(reporter)
        def failure = new RuntimeException("build failed")

        when:
        logger.beforeComplete(failure)

        then:
        1 * reporter.logSummary()
    }

    def "afterStart is a no-op"() {
        given:
        def reporter = Mock(HashMismatchReporter)
        def logger = new BuildFinishMismatchLogger(reporter)

        when:
        logger.afterStart()

        then:
        0 * reporter._
    }
}
