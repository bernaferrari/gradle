package org.gradle.internal.rustbridge.taskgraph

import org.gradle.api.Task
import org.gradle.api.execution.TaskExecutionGraph
import org.gradle.internal.rustbridge.bootstrap.RustBootstrapClient
import org.gradle.internal.rustbridge.eventstream.BuildIdHolder
import org.gradle.internal.rustbridge.jvmhost.BuildPlanTaskSelectionSnapshot
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
        graph.getDependencies(_ as Task) >> { Task task ->
            task.is(taskC) ? new LinkedHashSet<Task>([taskA, taskB]) : new LinkedHashSet<Task>()
        }

        when:
        listener.graphPopulated(graph)

        then:
        1 * reporter.compareExecutionGraph(
            [":app:compileJava", ":app:processResources", ":app:classes"],
            { deps ->
                deps[":app:compileJava"] == [] &&
                    deps[":app:processResources"] == [] &&
                    deps[":app:classes"].containsAll([":app:compileJava", ":app:processResources"])
            },
            "build"
        )
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
        graph.getDependencies(_ as Task) >> { Task task ->
            task.is(appTask) ? new LinkedHashSet<Task>([libTask, utilTask]) : new LinkedHashSet<Task>()
        }

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

    def "graphPopulated records selected task graph snapshot"() {
        given:
        def reporter = Mock(TaskGraphShadowReporter)
        def snapshot = new BuildPlanTaskSelectionSnapshot()
        def listener = new TaskGraphShadowListener(reporter, snapshot)

        def compile = Mock(Task)
        def classes = Mock(Task)
        compile.getPath() >> ":app:compileJava"
        classes.getPath() >> ":app:classes"

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> [compile, classes]
        graph.getDependencies(_ as Task) >> { Task task ->
            task.is(classes) ? new LinkedHashSet<Task>([compile]) : new LinkedHashSet<Task>()
        }

        when:
        listener.graphPopulated(graph)

        then:
        def selected = snapshot.snapshot()
        selected.populated
        selected.taskPaths == [":app:compileJava", ":app:classes"]
        selected.getDependencies(":app:classes") == [":app:compileJava"]
        1 * reporter.compareExecutionGraph(_, _, "build")
    }

    def "graphPopulated refreshes build-plan shadow after selected graph snapshot"() {
        given:
        def reporter = Mock(TaskGraphShadowReporter)
        def snapshot = new BuildPlanTaskSelectionSnapshot()
        def bootstrapClient = Mock(RustBootstrapClient)
        def listener = new TaskGraphShadowListener(reporter, snapshot, bootstrapClient)
        BuildIdHolder.setBuildId("build-selected")

        def compile = Mock(Task)
        def classes = Mock(Task)
        compile.getPath() >> ":app:compileJava"
        classes.getPath() >> ":app:classes"

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> [compile, classes]
        graph.getDependencies(_ as Task) >> { Task task ->
            task.is(classes) ? new LinkedHashSet<Task>([compile]) : new LinkedHashSet<Task>()
        }

        when:
        listener.graphPopulated(graph)

        then:
        1 * bootstrapClient.refreshBuildPlanShadow("build-selected") >> {
            assert snapshot.snapshot().populated
            assert snapshot.snapshot().taskPaths == [":app:compileJava", ":app:classes"]
            true
        }
        1 * reporter.compareExecutionGraph(_, _, "build-selected")

        cleanup:
        BuildIdHolder.clear()
    }

    def "graphPopulated does not refresh build-plan shadow without bootstrap build id"() {
        given:
        def reporter = Mock(TaskGraphShadowReporter)
        def snapshot = new BuildPlanTaskSelectionSnapshot()
        def bootstrapClient = Mock(RustBootstrapClient)
        def listener = new TaskGraphShadowListener(reporter, snapshot, bootstrapClient)
        BuildIdHolder.clear()

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> []

        when:
        listener.graphPopulated(graph)

        then:
        0 * bootstrapClient.refreshBuildPlanShadow(_)
        1 * reporter.compareExecutionGraph([], [:], "build")
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
