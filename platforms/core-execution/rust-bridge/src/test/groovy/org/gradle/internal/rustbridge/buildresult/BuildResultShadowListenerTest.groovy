package org.gradle.internal.rustbridge.buildresult

import org.gradle.initialization.RootBuildLifecycleListener
import org.gradle.internal.rustbridge.history.RustExecutionHistoryClient
import org.gradle.internal.rustbridge.metrics.RustBuildMetricsClient
import spock.lang.Specification

class BuildResultShadowListenerTest extends Specification {

    def "implements RootBuildLifecycleListener"() {
        given:
        def client = Mock(RustBuildResultClient)
        def listener = new BuildResultShadowListener(client)

        expect:
        listener instanceof RootBuildLifecycleListener
    }

    def "single-arg constructor sets client and null metrics/history"() {
        given:
        def client = Mock(RustBuildResultClient)
        def listener = new BuildResultShadowListener(client)

        expect:
        listener != null
    }

    def "two-arg constructor with null metrics client sets null metricsRecorder"() {
        given:
        def client = Mock(RustBuildResultClient)
        def listener = new BuildResultShadowListener(client, null)

        expect:
        listener != null
    }

    def "three-arg constructor with all nulls works"() {
        given:
        def client = Mock(RustBuildResultClient)
        def listener = new BuildResultShadowListener(client, null, null)

        expect:
        listener != null
    }

    def "three-arg constructor with metrics client creates metricsRecorder"() {
        given:
        def client = Mock(RustBuildResultClient)
        def metricsClient = Mock(RustBuildMetricsClient)
        def historyClient = Mock(RustExecutionHistoryClient)
        def listener = new BuildResultShadowListener(client, metricsClient, historyClient)

        expect:
        listener != null
    }

    def "beforeComplete with null failure does not call reportBuildFailure"() {
        given:
        def client = Mock(RustBuildResultClient)
        def listener = new BuildResultShadowListener(client)

        when:
        listener.beforeComplete(null)

        then:
        0 * client.reportBuildFailure(*_)
    }

    def "beforeComplete with failure calls reportBuildFailure with message"() {
        given:
        def client = Mock(RustBuildResultClient)
        def listener = new BuildResultShadowListener(client)
        def failure = new RuntimeException("something went wrong")

        when:
        listener.beforeComplete(failure)

        then:
        1 * client.reportBuildFailure("build", "build_failed", "something went wrong", _)
    }

    def "beforeComplete with failure that has null message uses class name"() {
        given:
        def client = Mock(RustBuildResultClient)
        def listener = new BuildResultShadowListener(client)
        def failure = new RuntimeException((String) null)

        when:
        listener.beforeComplete(failure)

        then:
        1 * client.reportBuildFailure("build", "build_failed", "java.lang.RuntimeException", _)
    }

    def "afterStart with metrics client calls recordBuildStart"() {
        given:
        def client = Mock(RustBuildResultClient)
        def metricsClient = Mock(RustBuildMetricsClient)
        def listener = new BuildResultShadowListener(client, metricsClient)

        when:
        listener.afterStart()

        then:
        1 * metricsClient.recordTimer("build", "build.start", 0)
    }

    def "afterStart without metrics client is a no-op"() {
        given:
        def client = Mock(RustBuildResultClient)
        def listener = new BuildResultShadowListener(client)

        when:
        listener.afterStart()

        then:
        noExceptionThrown()
    }

    def "beforeComplete with metrics recorder calls recordBuildEnd"() {
        given:
        def client = Mock(RustBuildResultClient)
        def metricsClient = Mock(RustBuildMetricsClient)
        def listener = new BuildResultShadowListener(client, metricsClient)

        when:
        listener.beforeComplete(null)

        then:
        1 * metricsClient.recordTimer("build", "build.end", _)
        1 * metricsClient.recordCounter("build", "build.end", 1)
        1 * metricsClient.getPerformanceSummary("build") >> null
    }

    def "beforeComplete with metrics recorder on failure calls recordBuildEnd with false"() {
        given:
        def client = Mock(RustBuildResultClient)
        def metricsClient = Mock(RustBuildMetricsClient)
        def listener = new BuildResultShadowListener(client, metricsClient)
        def failure = new RuntimeException("boom")

        when:
        listener.beforeComplete(failure)

        then:
        1 * metricsClient.recordTimer("build", "build.end", _)
        1 * metricsClient.recordCounter("build", "build.end", 1)
        1 * metricsClient.getPerformanceSummary("build") >> null
    }

    def "beforeComplete with history client logs stats when entries exist"() {
        given:
        def client = Mock(RustBuildResultClient)
        def historyClient = Mock(RustExecutionHistoryClient)
        def listener = new BuildResultShadowListener(client, null, historyClient)
        def stats = Mock(gradle.substrate.v1.GetHistoryStatsResponse)
        stats.getEntryCount() >> 5
        stats.getTotalBytesStored() >> 10240
        stats.getHitRate() >> 0.85
        stats.getStores() >> 100
        stats.getRemoves() >> 10

        when:
        listener.beforeComplete(null)

        then:
        1 * historyClient.getStats() >> stats
    }

    def "beforeComplete with history client does not log when no entries"() {
        given:
        def client = Mock(RustBuildResultClient)
        def historyClient = Mock(RustExecutionHistoryClient)
        def listener = new BuildResultShadowListener(client, null, historyClient)
        def stats = Mock(gradle.substrate.v1.GetHistoryStatsResponse)
        stats.getEntryCount() >> 0

        when:
        listener.beforeComplete(null)

        then:
        1 * historyClient.getStats() >> stats
    }

    def "beforeComplete swallows exceptions from client"() {
        given:
        def client = Mock(RustBuildResultClient)
        def listener = new BuildResultShadowListener(client)
        def failure = new RuntimeException("boom")

        when:
        listener.beforeComplete(failure)

        then:
        1 * client.reportBuildFailure(*_) >> { throw new RuntimeException("gRPC error") }
        noExceptionThrown()
    }

    def "beforeComplete swallows exceptions from history client"() {
        given:
        def client = Mock(RustBuildResultClient)
        def historyClient = Mock(RustExecutionHistoryClient)
        def listener = new BuildResultShadowListener(client, null, historyClient)

        when:
        listener.beforeComplete(null)

        then:
        1 * historyClient.getStats() >> { throw new RuntimeException("history error") }
        noExceptionThrown()
    }

    def "beforeComplete with all three clients exercises full path"() {
        given:
        def client = Mock(RustBuildResultClient)
        def metricsClient = Mock(RustBuildMetricsClient)
        def historyClient = Mock(RustExecutionHistoryClient)
        def listener = new BuildResultShadowListener(client, metricsClient, historyClient)
        def stats = Mock(gradle.substrate.v1.GetHistoryStatsResponse)
        stats.getEntryCount() >> 0

        when:
        listener.beforeComplete(null)

        then:
        0 * client.reportBuildFailure(*_)
        1 * metricsClient.recordTimer("build", "build.end", _)
        1 * metricsClient.recordCounter("build", "build.end", 1)
        1 * metricsClient.getPerformanceSummary("build") >> null
        1 * historyClient.getStats() >> stats
    }
}
