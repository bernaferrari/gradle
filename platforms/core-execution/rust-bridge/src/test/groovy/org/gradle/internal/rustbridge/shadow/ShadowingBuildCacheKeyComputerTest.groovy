package org.gradle.internal.rustbridge.shadow

import org.gradle.internal.rustbridge.cache.BuildCacheOrchestrationClient
import spock.lang.Specification

class ShadowingBuildCacheKeyComputerTest extends Specification {

    def reporter = Mock(HashMismatchReporter)
    def cacheOrchestration = Mock(BuildCacheOrchestrationClient)

    def "returns java key when cache orchestration is null"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(null, reporter, false)

        when:
        def result = computer.computeAndCompare("java-key", ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == "java-key"
        0 * reporter._
    }

    def "returns java key when Rust computation fails"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, false)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >>
            BuildCacheOrchestrationClient.CacheKeyResult.error("connection refused")

        when:
        def result = computer.computeAndCompare("java-key", ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == "java-key"
        1 * reporter.reportRustError("cache-key::test", _)
    }

    def "reports match and returns java key in shadow mode when keys match"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, false)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >>
            new BuildCacheOrchestrationClient.CacheKeyResult(
                "rust-key".bytes, "java-key", true, "")

        when:
        def result = computer.computeAndCompare("java-key", ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == "java-key"
        1 * reporter.reportMatch()
    }

    def "reports mismatch and returns java key in shadow mode when keys differ"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, false)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >>
            new BuildCacheOrchestrationClient.CacheKeyResult(
                "rust-key".bytes, "rust-key", true, "")

        when:
        def result = computer.computeAndCompare("java-key", ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == "java-key"
        1 * reporter.reportMismatch("cache-key::test", _, _)
    }

    def "returns Rust key when authoritative and Rust succeeds"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, true)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >>
            new BuildCacheOrchestrationClient.CacheKeyResult(
                "rust-key".bytes, "rust-key", true, "")

        when:
        def result = computer.computeAndCompare("java-key", ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == "rust-key"
        // Still reports mismatch in authoritative mode for observability
        1 * reporter.reportMismatch("cache-key::test", _, _)
    }

    def "returns Rust key when authoritative and keys match"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, true)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >>
            new BuildCacheOrchestrationClient.CacheKeyResult(
                "java-key".bytes, "java-key", true, "")

        when:
        def result = computer.computeAndCompare("java-key", ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == "java-key"
        1 * reporter.reportMatch()
    }

    def "falls back to java key when authoritative but Rust fails"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, true)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >>
            BuildCacheOrchestrationClient.CacheKeyResult.error("daemon crash")

        when:
        def result = computer.computeAndCompare("java-key", ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == "java-key"
        1 * reporter.reportRustError("cache-key::test", _)
    }
}
