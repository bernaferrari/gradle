package org.gradle.internal.rustbridge.execution

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class ShadowingExecutionPlanAdvisorTest extends Specification {

    def "constructor defaults to non-authoritative mode"() {
        given:
        def rustClient = Mock(ExecutionPlanClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def advisor = new ShadowingExecutionPlanAdvisor(rustClient, reporter)

        then:
        !advisor.isAuthoritative()
    }

    def "three-arg constructor sets authoritative mode"() {
        given:
        def rustClient = Mock(ExecutionPlanClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def advisor = new ShadowingExecutionPlanAdvisor(rustClient, reporter, true)

        then:
        advisor.isAuthoritative()
    }

    def "shadow mode keeps java prediction as effective result"() {
        given:
        def rustClient = Mock(ExecutionPlanClient)
        def reporter = Mock(HashMismatchReporter)
        def advisor = new ShadowingExecutionPlanAdvisor(rustClient, reporter, false)

        when:
        def result = advisor.advisePredictionOrFallback(":app:test", "UP_TO_DATE", 10L)

        then:
        1 * rustClient.predictOutcome(_) >> ExecutionPlanClient.Prediction.FROM_CACHE
        1 * reporter.reportMismatch("execution-plan::app:test", "UP_TO_DATE", "FROM_CACHE")
        result.prediction == "UP_TO_DATE"
        result.source == "java-shadow"
        !result.rustSource
    }

    def "authoritative mode uses rust prediction when available"() {
        given:
        def rustClient = Mock(ExecutionPlanClient)
        def reporter = Mock(HashMismatchReporter)
        def advisor = new ShadowingExecutionPlanAdvisor(rustClient, reporter, true)

        when:
        def result = advisor.advisePredictionOrFallback(":app:test", "EXECUTE", 4L)

        then:
        1 * rustClient.predictOutcome(_) >> ExecutionPlanClient.Prediction.FROM_CACHE
        1 * reporter.reportMismatch("execution-plan::app:test", "EXECUTE", "FROM_CACHE")
        result.prediction == "FROM_CACHE"
        result.source == "rust"
        result.rustSource
    }

    def "authoritative mode falls back to java prediction on rust error"() {
        given:
        def rustClient = Mock(ExecutionPlanClient)
        def reporter = Mock(HashMismatchReporter)
        def advisor = new ShadowingExecutionPlanAdvisor(rustClient, reporter, true)

        when:
        def result = advisor.advisePredictionOrFallback(":app:test", "EXECUTE", 2L)

        then:
        1 * rustClient.predictOutcome(_) >> { throw new RuntimeException("socket closed") }
        1 * reporter.reportRustError("execution-plan::app:test", _ as Exception)
        result.prediction == "EXECUTE"
        result.source == "java-fallback"
        !result.rustSource
    }
}
