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
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> { throw new RuntimeException("connection failed") }
        def listener = new OutputChangeShadowListener(client)

        when:
        listener.invalidateCachesFor(["build/classes/java/main", "build/resources/main"])

        then:
        noExceptionThrown()
        listener.getInvalidationCount() == 1
        listener.getTotalPathsInvalidated() == 2
    }

    def "invalidateCachesFor with empty iterable increments count by 1 but paths by 0"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> { throw new RuntimeException("connection failed") }
        def listener = new OutputChangeShadowListener(client)

        when:
        listener.invalidateCachesFor([])

        then:
        noExceptionThrown()
        listener.getInvalidationCount() == 1
        listener.getTotalPathsInvalidated() == 0
    }

    def "multiple calls accumulate counters"() {
        given:
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> { throw new RuntimeException("connection failed") }
        def listener = new OutputChangeShadowListener(client)

        when:
        listener.invalidateCachesFor(["build/classes/java/main"])
        listener.invalidateCachesFor(["build/resources/main", "build/generated"])
        listener.invalidateCachesFor(["output.jar"])

        then:
        noExceptionThrown()
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
        def client = Mock(SubstrateClient)
        client.isNoop() >> false
        client.getBuildOperationsStub() >> { throw new RuntimeException("connection failed") }
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
