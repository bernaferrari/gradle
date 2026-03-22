package org.gradle.internal.rustbridge.buildops

import org.gradle.internal.operations.BuildOperationDescriptor
import org.gradle.internal.operations.BuildOperationListener
import org.gradle.internal.operations.OperationFinishEvent
import org.gradle.internal.operations.OperationIdentifier
import org.gradle.internal.operations.OperationStartEvent
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

class BuildOperationShadowListenerTest extends Specification {

    def "implements BuildOperationListener"() {
        expect:
        BuildOperationShadowListener instanceof BuildOperationListener
    }

    def "constructor accepts SubstrateClient"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true

        when:
        def listener = new BuildOperationShadowListener(client)

        then:
        listener != null
        listener.totalOperations == 0
        listener.totalDurationMs == 0
        listener.failureCount == 0
    }

    def "started is a no-op when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new BuildOperationShadowListener(client)

        when:
        listener.started(null, null)

        then:
        0 * client.getBuildOperationsStub()
    }

    def "finished is a no-op when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new BuildOperationShadowListener(client)

        when:
        listener.finished(null, null)

        then:
        0 * client.getBuildOperationsStub()
        listener.totalOperations == 0
        listener.totalDurationMs == 0
        listener.failureCount == 0
    }

    def "progress is a no-op"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new BuildOperationShadowListener(client)

        when:
        listener.progress(null, null)

        then:
        0 * client._
        listener.totalOperations == 0
    }

    def "started delegates to client when not noop"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new BuildOperationShadowListener(client)

        def id = new OperationIdentifier(42)
        def parentId = new OperationIdentifier(1)
        def op = BuildOperationDescriptor.displayName("Test Op")
            .name("TestType")
            .build(id, parentId)
        def startEvent = new OperationStartEvent(1000L)

        when:
        listener.started(op, startEvent)

        then:
        1 * stub.startOperation(_)
    }

    def "started handles null id and parentId gracefully"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new BuildOperationShadowListener(client)

        def op = BuildOperationDescriptor.displayName("No Parent Op")
            .name("NoParentType")
            .build()
        def startEvent = new OperationStartEvent(500L)

        when:
        listener.started(op, startEvent)

        then:
        1 * stub.startOperation(_)
    }

    def "started catches exception and does not propagate"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> { throw new RuntimeException("connection failed") }
        def listener = new BuildOperationShadowListener(client)

        def op = BuildOperationDescriptor.displayName("Failing Op")
            .name("FailingType")
            .build()
        def startEvent = new OperationStartEvent(1000L)

        when:
        listener.started(op, startEvent)

        then:
        noExceptionThrown()
    }

    def "finished delegates to client and tracks stats for successful operation"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new BuildOperationShadowListener(client)

        def id = new OperationIdentifier(1)
        def op = BuildOperationDescriptor.displayName("Compile task")
            .name("CompileJava")
            .build(id, null)
        def finishEvent = new OperationFinishEvent(1000L, 1500L, null, null)

        when:
        listener.finished(op, finishEvent)

        then:
        1 * stub.completeOperation(_)
        listener.totalOperations == 1
        listener.totalDurationMs == 500L
        listener.failureCount == 0
        listener.countsByType["CompileJava"] == 1L
        listener.countsByType["CompileJava"] != null
    }

    def "finished tracks failure count when operation fails"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new BuildOperationShadowListener(client)

        def id = new OperationIdentifier(2)
        def op = BuildOperationDescriptor.displayName("Failing task")
            .name("TestTask")
            .build(id, null)
        def failure = new RuntimeException("boom")
        def finishEvent = new OperationFinishEvent(2000L, 2100L, failure, null)

        when:
        listener.finished(op, finishEvent)

        then:
        1 * stub.completeOperation(_)
        listener.totalOperations == 1
        listener.failureCount == 1
    }

    def "finished aggregates counts and durations by type"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new BuildOperationShadowListener(client)

        def op1 = BuildOperationDescriptor.displayName("Compile 1")
            .name("CompileJava")
            .build(new OperationIdentifier(1), null)
        def op2 = BuildOperationDescriptor.displayName("Compile 2")
            .name("CompileJava")
            .build(new OperationIdentifier(2), null)
        def op3 = BuildOperationDescriptor.displayName("Test")
            .name("TestJava")
            .build(new OperationIdentifier(3), null)

        when:
        listener.finished(op1, new OperationFinishEvent(0L, 100L, null, null))
        listener.finished(op2, new OperationFinishEvent(0L, 200L, null, null))
        listener.finished(op3, new OperationFinishEvent(0L, 50L, null, null))

        then:
        listener.totalOperations == 3
        listener.totalDurationMs == 350L
        listener.countsByType["CompileJava"] == 2L
        listener.countsByType["TestJava"] == 1L
        listener.getCountsByType().size() == 2
    }

    def "finished catches exception and does not propagate"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> { throw new RuntimeException("connection failed") }
        def listener = new BuildOperationShadowListener(client)

        def op = BuildOperationDescriptor.displayName("Failing Op")
            .name("FailingType")
            .build()
        def finishEvent = new OperationFinishEvent(0L, 100L, null, null)

        when:
        listener.finished(op, finishEvent)

        then:
        noExceptionThrown()
    }

    def "finished tracks slowest operations"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new BuildOperationShadowListener(client)

        def fastOp = BuildOperationDescriptor.displayName("Fast Op")
            .name("FastType")
            .build(new OperationIdentifier(1), null)
        def slowOp = BuildOperationDescriptor.displayName("Slow Op")
            .name("SlowType")
            .build(new OperationIdentifier(2), null)

        when:
        listener.finished(fastOp, new OperationFinishEvent(0L, 10L, null, null))
        listener.finished(slowOp, new OperationFinishEvent(0L, 5000L, null, null))

        then:
        listener.slowestOps["Slow Op"] == 5000L
        listener.slowestOps["Fast Op"] == 10L
        listener.slowestOps.size() == 2
    }

    def "slowest ops is capped at 10 entries"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new BuildOperationShadowListener(client)

        when: "add 12 operations with increasing durations"
        12.times { i ->
            def op = BuildOperationDescriptor.displayName("Op ${i}")
                .name("SomeType")
                .build(new OperationIdentifier(i), null)
            listener.finished(op, new OperationFinishEvent(0L, (i + 1) * 100L, null, null))
        }

        then: "only 10 are kept (the slowest)"
        listener.slowestOps.size() <= 10
    }

    def "getSlowestOps returns a copy"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new BuildOperationShadowListener(client)

        when:
        def ops1 = listener.slowestOps
        def ops2 = listener.slowestOps

        then:
        ops1 != ops2
        ops1 == ops2
    }

    def "getCountsByType returns a copy"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new BuildOperationShadowListener(client)

        when:
        def counts1 = listener.countsByType
        def counts2 = listener.countsByType

        then:
        counts1 != counts2
        counts1 == counts2
    }
}
