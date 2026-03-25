package org.gradle.internal.rustbridge.dependency

import org.gradle.api.artifacts.ResolvableDependencies
import org.gradle.api.artifacts.result.ResolvedComponentResult
import org.gradle.api.artifacts.result.ResolutionResult
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class DependencyResolutionShadowListenerTest extends Specification {

    def "constructor sets up client and mismatchReporter fields"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def listener = new DependencyResolutionShadowListener(client, reporter)

        then:
        listener.totalResolutionTimeMs == 0
        listener.resolutionCount == 0
    }

    def "beforeResolve records start time and does not call client"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "compileClasspath"
        }

        when:
        listener.beforeResolve(dependencies)

        then:
        0 * client._
        0 * reporter._
    }

    def "beforeResolve is a no-op when client is null"() {
        given:
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(null, reporter)
        def dependencies = Mock(ResolvableDependencies)

        when:
        listener.beforeResolve(dependencies)

        then:
        noExceptionThrown()
    }

    def "afterResolve records resolution and reports match on success"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)

        def componentA = Mock(ResolvedComponentResult)
        def componentB = Mock(ResolvedComponentResult)
        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([componentA, componentB] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "runtimeClasspath"
            getResolutionResult() >> resolutionResult
        }

        // Call beforeResolve first so the start time is recorded
        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        1 * client.recordResolution("runtimeClasspath", _, 2, true, 0)
        1 * reporter.reportMatch()
        listener.resolutionCount == 1
        listener.totalResolutionTimeMs >= 0
    }

    def "afterResolve reports rust error when client throws exception"() {
        given:
        def client = Mock(RustDependencyResolutionClient) {
            recordResolution(*_) >> { throw new RuntimeException("gRPC failure") }
        }
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)

        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "testRuntimeClasspath"
            getResolutionResult() >> resolutionResult
        }

        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        1 * reporter.reportRustError("dep-resolve:testRuntimeClasspath", _)
    }

    def "afterResolve is a no-op when client is null"() {
        given:
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(null, reporter)
        def dependencies = Mock(ResolvableDependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        0 * reporter._
        noExceptionThrown()
    }

    def "afterResolve handles resolution result extraction failure gracefully"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)

        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "api"
            getResolutionResult() >> { throw new RuntimeException("resolution incomplete") }
        }

        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        1 * client.recordResolution("api", _, 0, true, 0)
        1 * reporter.reportMatch()
    }

    def "resolution count and total time accumulate across multiple resolutions"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter)

        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }

        def depsCompile = Mock(ResolvableDependencies) {
            getName() >> "compileClasspath"
            getResolutionResult() >> resolutionResult
        }
        def depsRuntime = Mock(ResolvableDependencies) {
            getName() >> "runtimeClasspath"
            getResolutionResult() >> resolutionResult
        }

        when:
        listener.beforeResolve(depsCompile)
        listener.afterResolve(depsCompile)

        listener.beforeResolve(depsRuntime)
        listener.afterResolve(depsRuntime)

        then:
        listener.resolutionCount == 2
        listener.totalResolutionTimeMs >= 0
        2 * reporter.reportMatch()
    }

    def "authoritative mode records via strict call"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter, true)

        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "runtimeClasspath"
            getResolutionResult() >> resolutionResult
        }
        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        listener.isAuthoritative()
        1 * client.recordResolutionStrict("runtimeClasspath", _, 0, true, 0)
        1 * reporter.reportMatch()
    }

    def "authoritative mode falls back when strict recording fails"() {
        given:
        def client = Mock(RustDependencyResolutionClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new DependencyResolutionShadowListener(client, reporter, true)

        def resolutionResult = Mock(ResolutionResult) {
            getAllComponents() >> ([] as Set)
            getAllDependencies() >> ([] as Set)
        }
        def dependencies = Mock(ResolvableDependencies) {
            getName() >> "compileClasspath"
            getResolutionResult() >> resolutionResult
        }
        listener.beforeResolve(dependencies)

        when:
        listener.afterResolve(dependencies)

        then:
        1 * client.recordResolutionStrict("compileClasspath", _, 0, true, 0) >> {
            throw new RuntimeException("rpc down")
        }
        1 * reporter.reportRustError("dep-resolve:compileClasspath", _ as RuntimeException)
        1 * client.recordResolution("compileClasspath", _, 0, true, 0)
    }
}
