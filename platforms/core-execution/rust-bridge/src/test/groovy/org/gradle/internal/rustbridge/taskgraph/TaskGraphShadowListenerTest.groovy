package org.gradle.internal.rustbridge.taskgraph

import org.gradle.api.Project
import org.gradle.api.Task
import org.gradle.api.execution.TaskExecutionGraph
import org.gradle.api.tasks.TaskDependency
import org.gradle.internal.rustbridge.bootstrap.RustBootstrapClient
import org.gradle.internal.rustbridge.eventstream.BuildIdHolder
import org.gradle.internal.rustbridge.jvmhost.BuildPlanTaskSelectionSnapshot
import spock.lang.Specification

class TaskGraphShadowListenerTest extends Specification {

    def setup() {
        BuildIdHolder.clear()
    }

    def cleanup() {
        BuildIdHolder.clear()
    }

    def "graphPopulated extracts task paths and dependencies and delegates to reporter"() {
        given:
        def reporter = Mock(TaskGraphShadowReporter)
        def listener = new TaskGraphShadowListener(reporter)

        def taskA = task(":app:compileJava", "compileJava")
        def taskB = task(":app:processResources", "processResources")
        def taskC = task(":app:classes", "classes")

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

        def libTask = task(":lib:jar", "jar")
        def utilTask = task(":util:jar", "jar")
        def appTask = task(":app:run", "run")

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

        def compile = task(":app:compileJava", "compileJava")
        def classes = task(":app:classes", "classes")

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

        def compile = task(":app:compileJava", "compileJava")
        def classes = task(":app:classes", "classes")

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> [compile, classes]
        graph.getDependencies(_ as Task) >> { Task task ->
            task.is(classes) ? new LinkedHashSet<Task>([compile]) : new LinkedHashSet<Task>()
        }

        when:
        listener.graphPopulated(graph)

        then:
        1 * bootstrapClient.refreshBuildPlanShadow("build-selected", _) >> { args ->
            assert snapshot.snapshot().populated
            assert snapshot.snapshot().taskPaths == [":app:compileJava", ":app:classes"]
            assert args[1].tasksCount == 2
            true
        }
        1 * reporter.compareExecutionGraph(_, _, "build-selected")

        cleanup:
        BuildIdHolder.clear()
    }

    def "graphPopulated refreshes build-plan shadow with default build id"() {
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
        1 * bootstrapClient.refreshBuildPlanShadow("build", _) >> true
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

    private Task task(String path, String name) {
        def project = Stub(Project) {
            getPath() >> ":app"
        }
        Mock(Task) {
            getPath() >> path
            getName() >> name
            getProject() >> project
            getEnabled() >> true
            getGroup() >> null
            getDescription() >> null
            getInputs() >> null
            getOutputs() >> null
            getDestroyables() >> null
            getLocalState() >> null
            getTaskDependencies() >> Stub(TaskDependency)
            getShouldRunAfter() >> Stub(TaskDependency)
            getMustRunAfter() >> Stub(TaskDependency)
            getFinalizedBy() >> Stub(TaskDependency)
        }
    }
}
