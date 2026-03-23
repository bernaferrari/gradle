package org.gradle.internal.rustbridge.buildinit

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class ShadowingBuildInitTrackerTest extends Specification {

    def "compareInitStatus reports match when status and settings count agree"() {
        given:
        def rustResponse = gradle.substrate.v1.GetBuildInitStatusResponse.newBuilder()
            .setInitialized(true)
            .setSettingsDetailsCount(5)
            .build()
        def rustClient = Mock(RustBuildInitClient) {
            getBuildInitStatus("build-1") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildInitTracker(rustClient, reporter)

        when:
        tracker.compareInitStatus("build-1", true, 5)

        then:
        1 * reporter.reportMatch()
        0 * reporter.reportMismatch(_, _, _)
    }

    def "compareInitStatus reports mismatch when initialized differs"() {
        given:
        def rustResponse = gradle.substrate.v1.GetBuildInitStatusResponse.newBuilder()
            .setInitialized(false)
            .setSettingsDetailsCount(0)
            .build()
        def rustClient = Mock(RustBuildInitClient) {
            getBuildInitStatus("build-1") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildInitTracker(rustClient, reporter)

        when:
        tracker.compareInitStatus("build-1", true, 5)

        then:
        1 * reporter.reportMismatch("build-init:status:build-1", "true", "false")
    }

    def "compareInitStatus reports mismatch when settings count differs"() {
        given:
        def rustResponse = gradle.substrate.v1.GetBuildInitStatusResponse.newBuilder()
            .setInitialized(true)
            .setSettingsDetailsCount(3)
            .build()
        def rustClient = Mock(RustBuildInitClient) {
            getBuildInitStatus("build-1") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildInitTracker(rustClient, reporter)

        when:
        tracker.compareInitStatus("build-1", true, 5)

        then:
        1 * reporter.reportMismatch("build-init:settingsCount:build-1", "5", "3")
    }

    def "compareInitStatus reports Rust error"() {
        given:
        def rustClient = Mock(RustBuildInitClient) {
            getBuildInitStatus(_) >> { throw new RuntimeException("daemon crashed") }
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildInitTracker(rustClient, reporter)

        when:
        tracker.compareInitStatus("build-1", true, 3)

        then:
        1 * reporter.reportRustError("build-init:status:build-1", _ as RuntimeException)
    }
}
