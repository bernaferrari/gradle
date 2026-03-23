package org.gradle.internal.rustbridge.buildlayout

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class ShadowingBuildLayoutTrackerTest extends Specification {

    def "compareProjectList reports match when lists agree"() {
        given:
        def rustClient = Mock(RustBuildLayoutClient) {
            listProjects("build-1") >> [":", ":app", ":lib"]
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildLayoutTracker(rustClient, reporter)

        when:
        tracker.compareProjectList("build-1", [":", ":app", ":lib"])

        then:
        1 * reporter.reportMatch()
        0 * reporter.reportMismatch(_, _, _)
    }

    def "compareProjectList reports mismatch when sizes differ"() {
        given:
        def rustClient = Mock(RustBuildLayoutClient) {
            listProjects("build-1") >> [":"]
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildLayoutTracker(rustClient, reporter)

        when:
        tracker.compareProjectList("build-1", [":", ":app"])

        then:
        1 * reporter.reportMismatch("build-layout:projects:build-1", "2", "1")
    }

    def "compareProjectList reports mismatch when paths differ"() {
        given:
        def rustClient = Mock(RustBuildLayoutClient) {
            listProjects("build-1") >> [":", ":core", ":app"]
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildLayoutTracker(rustClient, reporter)

        when:
        tracker.compareProjectList("build-1", [":", ":lib", ":app"])

        then:
        1 * reporter.reportMismatch("build-layout:projects:build-1", ":,:lib,:app", ":,:core,:app")
    }

    def "compareProjectList reports Rust error"() {
        given:
        def rustClient = Mock(RustBuildLayoutClient) {
            listProjects(_) >> { throw new RuntimeException("connection lost") }
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildLayoutTracker(rustClient, reporter)

        when:
        tracker.compareProjectList("build-1", [":"])

        then:
        1 * reporter.reportRustError("build-layout:projects:build-1", _ as RuntimeException)
    }

    def "compareBuildFilePath reports match when paths agree"() {
        given:
        def rustResponse = gradle.substrate.v1.GetBuildFilePathResponse.newBuilder()
            .setBuildFilePath("/project/build.gradle.kts").build()
        def rustClient = Mock(RustBuildLayoutClient) {
            getBuildFilePath("build-1", ":app") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildLayoutTracker(rustClient, reporter)

        when:
        tracker.compareBuildFilePath("build-1", ":app", "/project/build.gradle.kts")

        then:
        1 * reporter.reportMatch()
    }

    def "compareBuildFilePath reports mismatch when paths differ"() {
        given:
        def rustResponse = gradle.substrate.v1.GetBuildFilePathResponse.newBuilder()
            .setBuildFilePath("/project/build.gradle").build()
        def rustClient = Mock(RustBuildLayoutClient) {
            getBuildFilePath("build-1", ":app") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildLayoutTracker(rustClient, reporter)

        when:
        tracker.compareBuildFilePath("build-1", ":app", "/project/build.gradle.kts")

        then:
        1 * reporter.reportMismatch("build-layout:buildFile:build-1::app",
            "/project/build.gradle.kts", "/project/build.gradle")
    }

    def "compareBuildFilePath reports Rust error"() {
        given:
        def rustClient = Mock(RustBuildLayoutClient) {
            getBuildFilePath(_, _) >> { throw new RuntimeException("timeout") }
        }
        def reporter = Mock(HashMismatchReporter)
        def tracker = new ShadowingBuildLayoutTracker(rustClient, reporter)

        when:
        tracker.compareBuildFilePath("build-1", ":app", "/project/build.gradle.kts")

        then:
        1 * reporter.reportRustError("build-layout:buildFile:build-1::app", _ as RuntimeException)
    }
}
