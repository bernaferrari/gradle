package org.gradle.internal.rustbridge.history

import org.gradle.internal.execution.history.AfterExecutionState
import org.gradle.internal.execution.history.ExecutionHistoryStore
import org.gradle.internal.execution.history.PreviousExecutionState
import spock.lang.Specification

class ShadowingExecutionHistoryStoreTest extends Specification {

    def "implements ExecutionHistoryStore"() {
        expect:
        ExecutionHistoryStore.isAssignableFrom(ShadowingExecutionHistoryStore)
    }

    def "three-arg constructor defaults authoritative to false"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer)

        then:
        store instanceof ExecutionHistoryStore
        store.getStats().getStores() == 0
        store.getStats().getLoads() == 0
    }

    def "four-arg constructor stores authoritative flag"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)

        when:
        def shadowStore = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, true)

        then:
        shadowStore instanceof ExecutionHistoryStore

        when: "authoritative load - Rust returns null, Java returns empty"
        shadowStore.load("some-key")

        then: "Rust is queried first in authoritative mode"
        1 * rustClient.load("some-key") >> null
        1 * javaDelegate.load("some-key") >> Optional.empty()
    }

    def "store writes to both Java and Rust"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def executionState = Mock(AfterExecutionState)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)
        store.store("task-key", executionState)

        then:
        1 * serializer.serialize(executionState) >> new byte[0]
        1 * javaDelegate.store("task-key", executionState)
        1 * rustClient.store("task-key", _ as byte[]) >> true

        and: "stats reflect the successful store"
        store.getStats().getStores() == 1
        store.getStats().getErrors() == 0
    }

    def "store records error when Rust store fails"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def executionState = Mock(AfterExecutionState)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)
        store.store("task-key", executionState)

        then:
        1 * serializer.serialize(executionState) >> new byte[0]
        1 * javaDelegate.store("task-key", executionState)
        1 * rustClient.store("task-key", _ as byte[]) >> false

        and: "error is recorded since Rust returned false"
        store.getStats().getStores() == 0
        store.getStats().getErrors() == 1
    }

    def "store handles Rust exception gracefully"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def executionState = Mock(AfterExecutionState)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)
        store.store("task-key", executionState)

        then:
        1 * serializer.serialize(executionState) >> new byte[0]
        1 * javaDelegate.store("task-key", executionState)
        1 * rustClient.store("task-key", _ as byte[]) >> { throw new RuntimeException("connection refused") }

        and: "error is recorded"
        store.getStats().getErrors() == 1
    }

    def "load in shadow mode verifies against Rust when Java has entry"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def prevState = Mock(PreviousExecutionState)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)
        def result = store.load("task-key")

        then: "Java is the source of truth in shadow mode"
        1 * javaDelegate.load("task-key") >> Optional.of(prevState)
        1 * rustClient.load("task-key") >> null

        and: "result comes from Java"
        result.isPresent()
        result.get() == prevState

        and: "stats reflect a miss from Rust"
        store.getStats().getLoads() == 1
        store.getStats().getRustMisses() == 1
    }

    def "load in shadow mode records Rust hit"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def prevState = Mock(PreviousExecutionState)
        def rustEntry = Mock(RustExecutionHistoryClient.HistoryEntry)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)
        def result = store.load("task-key")

        then:
        1 * javaDelegate.load("task-key") >> Optional.of(prevState)
        1 * rustClient.load("task-key") >> rustEntry

        and:
        result.isPresent()
        store.getStats().getRustHits() == 1
    }

    def "load in authoritative mode returns from Rust when available"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def prevState = Mock(PreviousExecutionState)
        def rustEntry = Mock(RustExecutionHistoryClient.HistoryEntry)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, true)
        def result = store.load("task-key")

        then:
        1 * rustClient.load("task-key") >> rustEntry
        1 * rustEntry.getSerializedState() >> new byte[0]
        1 * serializer.deserialize(_ as byte[]) >> prevState
        0 * javaDelegate.load(_)

        and:
        result.isPresent()
        result.get() == prevState
        store.getStats().getRustHits() == 1
    }

    def "load in authoritative mode falls back to Java when Rust returns null"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def prevState = Mock(PreviousExecutionState)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, true)
        def result = store.load("task-key")

        then:
        1 * rustClient.load("task-key") >> null
        1 * javaDelegate.load("task-key") >> Optional.of(prevState)

        and:
        result.isPresent()
        result.get() == prevState
        store.getStats().getRustMisses() == 1
    }

    def "load in authoritative mode falls back to Java when Rust throws"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def prevState = Mock(PreviousExecutionState)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, true)
        def result = store.load("task-key")

        then:
        1 * rustClient.load("task-key") >> { throw new RuntimeException("gRPC error") }
        1 * javaDelegate.load("task-key") >> Optional.of(prevState)

        and:
        result.isPresent()
        store.getStats().getRustErrors() == 0  // load errors are not counted in rustErrorCount; only store errors
    }

    def "load in authoritative mode falls back to Java when deserialization fails"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def prevState = Mock(PreviousExecutionState)
        def rustEntry = Mock(RustExecutionHistoryClient.HistoryEntry)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, true)
        def result = store.load("task-key")

        then:
        1 * rustClient.load("task-key") >> rustEntry
        1 * rustEntry.getSerializedState() >> new byte[0]
        1 * serializer.deserialize(_ as byte[]) >> null
        1 * javaDelegate.load("task-key") >> Optional.of(prevState)

        and:
        result.isPresent()
        result.get() == prevState
    }

    def "remove delegates to both Java and Rust"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)
        store.remove("task-key")

        then:
        1 * javaDelegate.remove("task-key")
        1 * rustClient.remove("task-key") >> true
    }

    def "remove handles Rust exception gracefully"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)
        store.remove("task-key")

        then:
        1 * javaDelegate.remove("task-key")
        1 * rustClient.remove("task-key") >> { throw new RuntimeException("connection refused") }

        and: "no exception propagates"
        noExceptionThrown()
    }

    // ---- ShadowStats inner class tests ----

    def "ShadowStats reports correct values"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def executionState = Mock(AfterExecutionState)
        def prevState = Mock(PreviousExecutionState)

        when:
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)

        then:
        store.getStats().getStores() == 0
        store.getStats().getLoads() == 0
        store.getStats().getRustHits() == 0
        store.getStats().getRustMisses() == 0
        store.getStats().getErrors() == 0

        when: "perform a store"
        store.store("key1", executionState)

        then:
        1 * serializer.serialize(executionState) >> new byte[0]
        1 * javaDelegate.store("key1", executionState)
        1 * rustClient.store("key1", _ as byte[]) >> true

        and:
        store.getStats().getStores() == 1

        when: "perform a load"
        store.load("key1")

        then:
        1 * javaDelegate.load("key1") >> Optional.of(prevState)
        1 * rustClient.load("key1") >> null

        and:
        store.getStats().getLoads() == 1
        store.getStats().getRustMisses() == 1
    }

    def "ShadowStats errorRate is zero when no stores"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)

        expect:
        store.getStats().getErrorRate() == 0.0
    }

    def "ShadowStats errorRate reflects errors over stores"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def executionState = Mock(AfterExecutionState)
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)

        when:
        store.store("key", executionState)

        then:
        1 * serializer.serialize(executionState) >> new byte[0]
        1 * javaDelegate.store("key", executionState)
        1 * rustClient.store("key", _ as byte[]) >> false

        expect: "1 error out of 0 successful stores -- errorRate is errors/stores where stores is the field, not successful stores"
        store.getStats().getErrorRate() == Double.POSITIVE_INFINITY
    }

    def "ShadowStats hitRate is zero when no loads"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)

        expect:
        store.getStats().getHitRate() == 0.0
    }

    def "ShadowStats hitRate reflects rustHits over loads"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def prevState = Mock(PreviousExecutionState)
        def rustEntry = Mock(RustExecutionHistoryClient.HistoryEntry)
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)

        when:
        store.load("key")

        then:
        1 * javaDelegate.load("key") >> Optional.of(prevState)
        1 * rustClient.load("key") >> rustEntry

        expect:
        store.getStats().getHitRate() == 1.0
    }

    def "ShadowStats toString contains formatted fields"() {
        given:
        def javaDelegate = Mock(ExecutionHistoryStore)
        def rustClient = Mock(RustExecutionHistoryClient)
        def serializer = Mock(ShadowingExecutionHistoryStore.ExecutionHistorySerializer)
        def store = new ShadowingExecutionHistoryStore(javaDelegate, rustClient, serializer, false)

        expect:
        store.getStats().toString() == "stores=0, loads=0, rustHits=0, rustMisses=0, errors=0, hitRate=0.0%"
    }
}
