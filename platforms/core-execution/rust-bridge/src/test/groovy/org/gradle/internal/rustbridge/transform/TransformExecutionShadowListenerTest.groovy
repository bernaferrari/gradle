package org.gradle.internal.rustbridge.transform

import org.gradle.api.Describable
import org.gradle.api.internal.artifacts.transform.TransformExecutionListener
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

class TransformExecutionShadowListenerTest extends Specification {

    def "implements TransformExecutionListener"() {
        expect:
        TransformExecutionShadowListener instanceof TransformExecutionListener
    }

    def "constructor accepts SubstrateClient and initializes fields"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true

        when:
        def listener = new TransformExecutionShadowListener(client)

        then:
        listener != null
        listener.totalTransformTimeMs == 0
        listener.transformCountCompleted == 0
        listener.transformTimeByType.isEmpty()
    }

    def "beforeTransformExecution is a no-op when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new TransformExecutionShadowListener(client)
        def transform = Mock(Describable)
        def subject = Mock(Describable)

        when:
        listener.beforeTransformExecution(transform, subject)

        then:
        0 * client.getBuildOperationsStub()
        listener.transformCountCompleted == 0
    }

    def "afterTransformExecution is a no-op when client is noop"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new TransformExecutionShadowListener(client)
        def transform = Mock(Describable)
        def subject = Mock(Describable)

        when:
        listener.afterTransformExecution(transform, subject)

        then:
        0 * client.getBuildOperationsStub()
        listener.transformCountCompleted == 0
        listener.totalTransformTimeMs == 0
    }

    def "beforeTransformExecution delegates to client when not noop"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new TransformExecutionShadowListener(client)

        def transform = Mock(Describable)
        transform.getDisplayName() >> "Desugar"
        def subject = Mock(Describable)
        subject.getDisplayName() >> "classes.jar"

        when:
        listener.beforeTransformExecution(transform, subject)

        then:
        1 * stub.startOperation({ req ->
            req.operationId == "transform:1"
            req.displayName.contains("Desugar")
            req.displayName.contains("classes.jar")
            req.operationType == "ARTIFACT_TRANSFORM"
            req.metadataMap["transform"] == "Desugar"
            req.metadataMap["subject"] == "classes.jar"
        })
    }

    def "afterTransformExecution delegates to client and tracks stats"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new TransformExecutionShadowListener(client)

        // First call before to register the start time
        def transform = Mock(Describable)
        transform.getDisplayName() >> "Desugar"
        def subject = Mock(Describable)
        subject.getDisplayName() >> "classes.jar"

        when:
        listener.beforeTransformExecution(transform, subject)
        listener.afterTransformExecution(transform, subject)

        then:
        1 * stub.startOperation(_)
        1 * stub.completeOperation({ req ->
            req.operationId == "transform:1"
            req.success
            req.outcome == "SUCCESS"
        })
        listener.transformCountCompleted == 1
        listener.totalTransformTimeMs >= 0
        listener.transformTimeByType["Desugar"] >= 0
    }

    def "beforeTransformExecution catches exception and does not propagate"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> { throw new RuntimeException("connection failed") }
        def listener = new TransformExecutionShadowListener(client)

        def transform = Mock(Describable)
        transform.getDisplayName() >> "FailingTransform"
        def subject = Mock(Describable)

        when:
        listener.beforeTransformExecution(transform, subject)

        then:
        noExceptionThrown()
    }

    def "afterTransformExecution catches exception and does not propagate"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> { throw new RuntimeException("connection failed") }
        def listener = new TransformExecutionShadowListener(client)

        // Set up start time by calling beforeTransformExecution which will also fail,
        // but that's ok - the afterTransformExecution should also handle errors gracefully.
        def transform = Mock(Describable)
        transform.getDisplayName() >> "FailingTransform"
        def subject = Mock(Describable)

        when:
        listener.beforeTransformExecution(transform, subject)
        listener.afterTransformExecution(transform, subject)

        then:
        noExceptionThrown()
    }

    def "getTransformTimeByType returns a defensive copy"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new TransformExecutionShadowListener(client)

        when:
        def times1 = listener.transformTimeByType
        def times2 = listener.transformTimeByType

        then:
        times1 != times2
        times1 == times2
    }

    def "tracks multiple transform executions by type"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new TransformExecutionShadowListener(client)

        def desugar = Mock(Describable)
        desugar.getDisplayName() >> "Desugar"
        def subject = Mock(Describable)
        subject.getDisplayName() >> "classes.jar"

        def strip = Mock(Describable)
        strip.getDisplayName() >> "StripDebugInfo"
        def subject2 = Mock(Describable)
        subject2.getDisplayName() >> "app.jar"

        when:
        listener.beforeTransformExecution(desugar, subject)
        listener.afterTransformExecution(desugar, subject)
        listener.beforeTransformExecution(strip, subject2)
        listener.afterTransformExecution(strip, subject2)
        listener.beforeTransformExecution(desugar, subject)
        listener.afterTransformExecution(desugar, subject)

        then:
        listener.transformCountCompleted == 3
        listener.transformTimeByType.size() == 2
        listener.transformTimeByType.containsKey("Desugar")
        listener.transformTimeByType.containsKey("StripDebugInfo")
        listener.totalTransformTimeMs >= 0
    }

    def "multiple beforeTransformExecution calls increment operation id"() {
        given:
        def stub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> stub
        def listener = new TransformExecutionShadowListener(client)

        def transform = Mock(Describable)
        transform.getDisplayName() >> "Extract"
        def subject = Mock(Describable)

        when:
        listener.beforeTransformExecution(transform, subject)
        listener.beforeTransformExecution(transform, subject)
        listener.beforeTransformExecution(transform, subject)

        then:
        3 * stub.startOperation(_)
    }
}
