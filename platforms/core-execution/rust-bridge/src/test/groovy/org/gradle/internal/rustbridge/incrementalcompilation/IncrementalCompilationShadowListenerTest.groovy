package org.gradle.internal.rustbridge.incrementalcompilation

import org.gradle.internal.rustbridge.incremental.RustIncrementalCompilationClient
import spock.lang.Specification

class IncrementalCompilationShadowListenerTest extends Specification {

    def "registerSourceSet increments counter when Rust accepts"() {
        given:
        def client = Mock(RustIncrementalCompilationClient)
        def listener = new IncrementalCompilationShadowListener(client)

        when:
        listener.registerSourceSet("build-1", "ss-1", "main", ["/src/main/java"], ["/build/classes/main"], "abc123")

        then:
        1 * client.registerSourceSet("build-1", "ss-1", "main", ["/src/main/java"], ["/build/classes/main"], "abc123") >> true
        listener.getSourceSetsRegistered() == 1
    }

    def "registerSourceSet does not increment when Rust rejects"() {
        given:
        def client = Mock(RustIncrementalCompilationClient)
        def listener = new IncrementalCompilationShadowListener(client)

        when:
        listener.registerSourceSet("build-1", "ss-1", "main", ["/src/main/java"], ["/build/classes/main"], "abc123")

        then:
        1 * client.registerSourceSet("build-1", "ss-1", "main", ["/src/main/java"], ["/build/classes/main"], "abc123") >> false
        listener.getSourceSetsRegistered() == 0
    }

    def "recordCompilation increments counter and tracks time"() {
        given:
        def client = Mock(RustIncrementalCompilationClient)
        def listener = new IncrementalCompilationShadowListener(client)

        when:
        listener.recordCompilation("build-1", "ss-1", "Foo.java", "Foo.class", "hash1", "hash2", ["Bar.java"], 50L)

        then:
        1 * client.recordCompilation("build-1", "ss-1", "Foo.java", "Foo.class", "hash1", "hash2", ["Bar.java"], 50L) >> true
        listener.getCompilationUnitsRecorded() == 1
        listener.getTotalCompileTimeMs() == 50L
    }

    def "recordCompilation does not increment on Rust failure"() {
        given:
        def client = Mock(RustIncrementalCompilationClient)
        def listener = new IncrementalCompilationShadowListener(client)

        when:
        listener.recordCompilation("build-1", "ss-1", "Foo.java", "Foo.class", "hash1", "hash2", ["Bar.java"], 50L)

        then:
        1 * client.recordCompilation("build-1", "ss-1", "Foo.java", "Foo.class", "hash1", "hash2", ["Bar.java"], 50L) >> false
        listener.getCompilationUnitsRecorded() == 0
        listener.getTotalCompileTimeMs() == 0L
    }

    def "reportChangedFiles queries Rust rebuild set"() {
        given:
        def client = Mock(RustIncrementalCompilationClient)
        def listener = new IncrementalCompilationShadowListener(client)
        def response = Mock(gradle.substrate.v1.GetRebuildSetResponse)
        response.getTotalSources() >> 10
        response.getMustRecompileCount() >> 3
        response.getUpToDateCount() >> 7
        response.getDecisionsCount() >> 10
        response.getDecisionsList() >> []

        when:
        listener.reportChangedFiles("build-1", "ss-1", ["Foo.java", "Bar.java"])

        then:
        1 * client.getRebuildSet("build-1", "ss-1", ["Foo.java", "Bar.java"]) >> response
        listener.getRebuildSetQueries() == 1
    }

    def "reportChangedFiles handles Rust failure gracefully"() {
        given:
        def client = Mock(RustIncrementalCompilationClient)
        def listener = new IncrementalCompilationShadowListener(client)

        when:
        listener.reportChangedFiles("build-1", "ss-1", ["Foo.java"])

        then:
        1 * client.getRebuildSet("build-1", "ss-1", ["Foo.java"]) >> { throw new RuntimeException("gRPC connection failed") }
        noExceptionThrown()
        listener.getRebuildSetQueries() == 1
    }

    def "queryIncrementalState queries Rust state"() {
        given:
        def client = Mock(RustIncrementalCompilationClient)
        def listener = new IncrementalCompilationShadowListener(client)
        def stateInfo = Mock(gradle.substrate.v1.IncrementalStateInfo)
        stateInfo.getTotalCompiled() >> 25
        def response = Mock(gradle.substrate.v1.GetIncrementalStateResponse)
        response.getState() >> stateInfo

        when:
        listener.queryIncrementalState("build-1", "ss-1")

        then:
        1 * client.getIncrementalState("build-1", "ss-1") >> response
        listener.getIncrementalStateQueries() == 1
    }
}
