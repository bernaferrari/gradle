package org.gradle.internal.rustbridge.taskgraph

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class TaskGraphShadowReporterTest extends Specification {

    def "constructor stores dependencies"() {
        given:
        def rustClient = Mock(RustTaskGraphClient)
        def mismatchReporter = Mock(HashMismatchReporter)

        when:
        def reporter = new TaskGraphShadowReporter(rustClient, mismatchReporter)

        then:
        reporter != null
        noExceptionThrown()
    }

    def "compareExecutionGraph with empty taskPaths returns immediately"() {
        given:
        def rustClient = Mock(RustTaskGraphClient)
        def mismatchReporter = Mock(HashMismatchReporter)
        def reporter = new TaskGraphShadowReporter(rustClient, mismatchReporter)

        when:
        reporter.compareExecutionGraph([], [:], "build-123")

        then:
        0 * rustClient._
        0 * mismatchReporter._
    }

    def "compareExecutionGraph with null rustClient returns immediately"() {
        given:
        def mismatchReporter = Mock(HashMismatchReporter)
        def reporter = new TaskGraphShadowReporter(null, mismatchReporter)

        when:
        reporter.compareExecutionGraph([":app:compileJava"], [":app:compileJava": []], "build-123")

        then:
        0 * mismatchReporter._
        noExceptionThrown()
    }

    def "compareExecutionGraph registers all tasks with Rust"() {
        given:
        def rustClient = Mock(RustTaskGraphClient)
        def mismatchReporter = Mock(HashMismatchReporter)
        def reporter = new TaskGraphShadowReporter(rustClient, mismatchReporter)

        def taskPaths = [":app:compileJava", ":app:processResources", ":app:classes"]
        def taskDeps = [
            ":app:compileJava": [],
            ":app:processResources": [],
            ":app:classes": [":app:compileJava", ":app:processResources"]
        ]

        def node1 = gradle.substrate.v1.ExecutionNode.newBuilder()
            .setTaskPath(":app:compileJava").build()
        def node2 = gradle.substrate.v1.ExecutionNode.newBuilder()
            .setTaskPath(":app:processResources").build()
        def node3 = gradle.substrate.v1.ExecutionNode.newBuilder()
            .setTaskPath(":app:classes").build()
        def planResult = RustTaskGraphClient.ExecutionPlanResult.success(
            [node1, node2, node3], 3, 3, 0, false)

        rustClient.registerTask(_, _, _, _) >> true
        rustClient.resolveExecutionPlan(_) >> planResult

        when:
        reporter.compareExecutionGraph(taskPaths, taskDeps, "build-123")

        then:
        1 * rustClient.registerTask(":app:compileJava", [], true, "Task")
        1 * rustClient.registerTask(":app:processResources", [], true, "Task")
        1 * rustClient.registerTask(":app:classes", [":app:compileJava", ":app:processResources"], true, "Task")
    }

    def "compareExecutionGraph reports match when Rust returns same order"() {
        given:
        def rustClient = Mock(RustTaskGraphClient)
        def mismatchReporter = Mock(HashMismatchReporter)
        def reporter = new TaskGraphShadowReporter(rustClient, mismatchReporter)

        def taskPaths = [":a", ":b", ":c"]
        def taskDeps = [
            ":a": [],
            ":b": [":a"],
            ":c": [":b"]
        ]

        def node1 = gradle.substrate.v1.ExecutionNode.newBuilder().setTaskPath(":a").build()
        def node2 = gradle.substrate.v1.ExecutionNode.newBuilder().setTaskPath(":b").build()
        def node3 = gradle.substrate.v1.ExecutionNode.newBuilder().setTaskPath(":c").build()
        def planResult = RustTaskGraphClient.ExecutionPlanResult.success(
            [node1, node2, node3], 3, 3, 0, false)

        rustClient.registerTask(_, _, _, _) >> true
        rustClient.resolveExecutionPlan(_) >> planResult

        when:
        reporter.compareExecutionGraph(taskPaths, taskDeps, "build-123")

        then:
        1 * mismatchReporter.reportMatch()
    }

    def "compareExecutionGraph handles Rust failure gracefully"() {
        given:
        def rustClient = Mock(RustTaskGraphClient)
        def mismatchReporter = Mock(HashMismatchReporter)
        def reporter = new TaskGraphShadowReporter(rustClient, mismatchReporter)

        def taskPaths = [":a", ":b"]
        def taskDeps = [":a": [], ":b": [":a"]]

        def errorResult = RustTaskGraphClient.ExecutionPlanResult.error("connection refused")
        rustClient.registerTask(_, _, _, _) >> true
        rustClient.resolveExecutionPlan(_) >> errorResult

        when:
        reporter.compareExecutionGraph(taskPaths, taskDeps, "build-123")

        then:
        0 * mismatchReporter.reportMatch()
        noExceptionThrown()
    }

    def "compareExecutionGraph reports match for different but valid topological order"() {
        given:
        def rustClient = Mock(RustTaskGraphClient)
        def mismatchReporter = Mock(HashMismatchReporter)
        def reporter = new TaskGraphShadowReporter(rustClient, mismatchReporter)

        // Java order: :a, :b, :c (all independent)
        def taskPaths = [":a", ":b", ":c"]
        def taskDeps = [
            ":a": [],
            ":b": [],
            ":c": []
        ]

        // Rust returns a different but still valid order (no deps to violate)
        def node1 = gradle.substrate.v1.ExecutionNode.newBuilder().setTaskPath(":c").build()
        def node2 = gradle.substrate.v1.ExecutionNode.newBuilder().setTaskPath(":a").build()
        def node3 = gradle.substrate.v1.ExecutionNode.newBuilder().setTaskPath(":b").build()
        def planResult = RustTaskGraphClient.ExecutionPlanResult.success(
            [node1, node2, node3], 3, 3, 0, false)

        rustClient.registerTask(_, _, _, _) >> true
        rustClient.resolveExecutionPlan(_) >> planResult

        when:
        reporter.compareExecutionGraph(taskPaths, taskDeps, "build-123")

        then:
        1 * mismatchReporter.reportMatch()
    }
}
