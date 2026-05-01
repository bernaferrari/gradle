package org.gradle.internal.rustbridge.jvmhost

import org.gradle.api.Project
import org.gradle.api.Task
import org.gradle.api.execution.TaskExecutionGraph
import org.gradle.api.tasks.TaskDependency
import spock.lang.Specification

class BuildPlanTaskSelectionCaptureListenerTest extends Specification {

    def "captures selected task contracts from populated task graph"() {
        given:
        def snapshot = new BuildPlanTaskSelectionSnapshot()
        def listener = new BuildPlanTaskSelectionCaptureListener(snapshot)
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
        selected.taskContracts.size() == 2
    }

    private Task task(String path, String name) {
        def project = Stub(Project) {
            getPath() >> ":app"
        }
        Stub(Task) {
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
