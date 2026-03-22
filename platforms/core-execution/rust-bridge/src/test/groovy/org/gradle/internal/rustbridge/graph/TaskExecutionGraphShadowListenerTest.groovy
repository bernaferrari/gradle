package org.gradle.internal.rustbridge.graph

import org.gradle.api.Task
import org.gradle.api.execution.TaskExecutionGraph
import org.gradle.api.execution.TaskExecutionGraphListener
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

class TaskExecutionGraphShadowListenerTest extends Specification {

    def "implements TaskExecutionGraphListener"() {
        given:
        def client = Mock(SubstrateClient)
        def listener = new TaskExecutionGraphShadowListener(client)

        expect:
        listener instanceof TaskExecutionGraphListener
    }

    def "graphPopulated counts tasks correctly"() {
        given:
        def eventStreamStub = Mock(gradle.substrate.v1.BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.getBuildEventStreamStub() >> eventStreamStub
        def listener = new TaskExecutionGraphShadowListener(client)

        def task1 = Mock(Task)
        def task2 = Mock(Task)
        def task3 = Mock(Task)

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> [task1, task2, task3]

        when:
        listener.graphPopulated(graph)

        then:
        1 * eventStreamStub.sendBuildEvent(_)
        listener.getTaskCount() == 3
    }

    def "graphPopulated sets timestamp"() {
        given:
        def eventStreamStub = Mock(gradle.substrate.v1.BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.getBuildEventStreamStub() >> eventStreamStub
        def listener = new TaskExecutionGraphShadowListener(client)

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> [Mock(Task)]

        def beforeMs = System.currentTimeMillis()

        when:
        listener.graphPopulated(graph)

        then:
        listener.getPopulateTimestampMs() >= beforeMs
        listener.getPopulateTimestampMs() <= System.currentTimeMillis()
    }

    def "graphPopulated with empty graph sets count to 0"() {
        given:
        def eventStreamStub = Mock(gradle.substrate.v1.BuildEventStreamServiceGrpc.BuildEventStreamServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.getBuildEventStreamStub() >> eventStreamStub
        def listener = new TaskExecutionGraphShadowListener(client)

        def graph = Mock(TaskExecutionGraph)
        graph.getAllTasks() >> []

        when:
        listener.graphPopulated(graph)

        then:
        listener.getTaskCount() == 0
    }

    def "getTaskCount and getPopulateTimestampMs return defaults before graphPopulated"() {
        given:
        def client = Mock(SubstrateClient)
        def listener = new TaskExecutionGraphShadowListener(client)

        expect:
        listener.getTaskCount() == 0
        listener.getPopulateTimestampMs() == 0L
    }
}
