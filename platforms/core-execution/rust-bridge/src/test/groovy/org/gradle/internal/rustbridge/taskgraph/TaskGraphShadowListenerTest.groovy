package org.gradle.internal.rustbridge.taskgraph

import org.gradle.api.Task
import org.gradle.api.execution.TaskExecutionGraph
import spock.lang.Specification

class TaskGraphShadowListenerTest extends Specification {

    def "graphPopulated extracts task paths and dependencies and delegates to reporter"() {
        given:
        def reporter = Mock(TaskGraphShadowReporter)
        def listener = new TaskGraphShadowListener(reporter)

        def taskA = Mock(Task)
        def taskB = Mock(Task)
        def taskC = Mock(Task)
        taskA.getPath() >> ":app:compileJava"
        taskB.getPath() >> ":app:processResources"
        taskC.getPath() >> ":app:classes"

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> [taskA, taskB, taskC]
        graph.getDependencies(taskA) >> ([] as Set<Task>)
        graph.getDependencies(taskB) >> ([] as Set<Task>)
        graph.getDependencies(taskC) >> ([taskA, taskB] as Set<Task>)

        when:
        listener.graphPopulated(graph)

        then:
        1 * reporter.compareExecutionGraph(
            [":app:compileJava", ":app:processResources", ":app:classes"],
            _,
            "build"
        )

        def capturedDeps = _
        capturedDeps[":app:compileJava"] == []
        capturedDeps[":app:processResources"] == []
        capturedDeps[":app:classes"].containsAll([":app:compileJava", ":app:processResources"])
    }

    def "graphPopulated handles empty graph"() {
        given:
        def reporter = Mock(TaskGraphShadowReporter)
        def listener = new TaskGraphShadowListener(reporter)

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> []

        when:
        listener.graphPopulated(graph)

        then:
        1 * reporter.compareExecutionGraph([], [:], "build")
    }

    def "graphPopulated handles task with multiple dependencies"() {
        given:
        def reporter = Mock(TaskGraphShadowReporter)
        def listener = new TaskGraphShadowListener(reporter)

        def libTask = Mock(Task)
        def utilTask = Mock(Task)
        def appTask = Mock(Task)
        libTask.getPath() >> ":lib:jar"
        utilTask.getPath() >> ":util:jar"
        appTask.getPath() >> ":app:run"

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> [libTask, utilTask, appTask]
        graph.getDependencies(libTask) >> ([] as Set<Task>)
        graph.getDependencies(utilTask) >> ([] as Set<Task>)
        graph.getDependencies(appTask) >> ([libTask, utilTask] as Set<Task>)

        when:
        listener.graphPopulated(graph)

        then:
        1 * reporter.compareExecutionGraph(
            [":lib:jar", ":util:jar", ":app:run"],
            { deps ->
                deps[":lib:jar"] == [] &&
                deps[":util:jar"] == [] &&
                deps[":app:run"].containsAll([":lib:jar", ":util:jar"])
            } as Map,
            "build"
        )
    }

    def "constructor stores reporter reference"() {
        given:
        def reporter = Mock(TaskGraphShadowReporter)

        when:
        def listener = new TaskGraphShadowListener(reporter)

        then:
        listener instanceof org.gradle.api.execution.TaskExecutionGraphListener
        noExceptionThrown()
    }
}
