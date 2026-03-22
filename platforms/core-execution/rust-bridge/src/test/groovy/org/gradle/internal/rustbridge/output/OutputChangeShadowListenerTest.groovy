package org.gradle.internal.rustbridge.output

import org.gradle.internal.execution.OutputChangeListener
import org.gradle.internal.rustbridge.SubstrateClient
import spock.lang.Specification

class OutputChangeShadowListenerTest extends Specification {

    def "implements OutputChangeListener"() {
        given:
        def client = Mock(SubstrateClient)
        def listener = new OutputChangeShadowListener(client)

        expect:
        listener instanceof OutputChangeListener
    }

    def "invalidateCachesFor increments counters"() {
        given:
        def buildOpsStub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> buildOpsStub
        def listener = new OutputChangeShadowListener(client)

        when:
        listener.invalidateCachesFor(["build/classes/java/main", "build/resources/main"])

        then:
        1 * buildOpsStub.startOperation(_)
        1 * buildOpsStub.completeOperation(_)
        listener.getInvalidationCount() == 1
        listener.getTotalPathsInvalidated() == 2
    }

    def "invalidateCachesFor with empty iterable increments count by 1 but paths by 0"() {
        given:
        def buildOpsStub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> buildOpsStub
        def listener = new OutputChangeShadowListener(client)

        when:
        listener.invalidateCachesFor([])

        then:
        1 * buildOpsStub.startOperation(_)
        1 * buildOpsStub.completeOperation(_)
        listener.getInvalidationCount() == 1
        listener.getTotalPathsInvalidated() == 0
    }

    def "multiple calls accumulate counters"() {
        given:
        def buildOpsStub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> buildOpsStub
        def listener = new OutputChangeShadowListener(client)

        when:
        listener.invalidateCachesFor(["build/classes/java/main"])
        listener.invalidateCachesFor(["build/resources/main", "build/generated"])
        listener.invalidateCachesFor(["output.jar"])

        then:
        3 * buildOpsStub.startOperation(_)
        3 * buildOpsStub.completeOperation(_)
        listener.getInvalidationCount() == 3
        listener.getTotalPathsInvalidated() == 4
    }

    def "noop client skips Rust calls"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> true
        def listener = new OutputChangeShadowListener(client)

        when:
        listener.invalidateCachesFor(["build/classes/java/main"])

        then:
        0 * client.getBuildOperationsStub()
        listener.getInvalidationCount() == 0
        listener.getTotalPathsInvalidated() == 0
    }

    def "getInvalidationCount and getTotalPathsInvalidated return correct values"() {
        given:
        def buildOpsStub = Mock(gradle.substrate.v1.BuildOperationsServiceGrpc.BuildOperationsServiceBlockingStub)
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> buildOpsStub
        def listener = new OutputChangeShadowListener(client)

        expect:
        listener.getInvalidationCount() == 0
        listener.getTotalPathsInvalidated() == 0

        when:
        listener.invalidateCachesFor(["a", "b", "c"])

        then:
        listener.getInvalidationCount() == 1
        listener.getTotalPathsInvalidated() == 3
    }
}
