package org.gradle.internal.rustbridge.shadow

import org.gradle.internal.rustbridge.cache.BuildCacheOrchestrationClient
import spock.lang.Specification

class ShadowingBuildCacheKeyComputerTest extends Specification {

    def reporter = Mock(HashMismatchReporter)
    def cacheOrchestration = Mock(BuildCacheOrchestrationClient)
    static final String JAVA_KEY = "0123456789abcdef0123456789abcdef"
    static final String RUST_KEY = "fedcba9876543210fedcba9876543210"

    def "returns java key when cache orchestration is null"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(null, reporter, false)

        when:
        def result = computer.computeAndCompare(JAVA_KEY, ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == JAVA_KEY
        0 * reporter._
    }

    def "returns java key when Rust computation fails"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, false)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >>
            BuildCacheOrchestrationClient.CacheKeyResult.error("connection refused")

        when:
        def result = computer.computeAndCompare(JAVA_KEY, ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == JAVA_KEY
        1 * reporter.reportRustError("cache-key::test", _)
    }

    def "reports match and returns java key in shadow mode when keys match"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, false)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >>
            new BuildCacheOrchestrationClient.CacheKeyResult(
                RUST_KEY.bytes, JAVA_KEY, true, "")

        when:
        def result = computer.computeAndCompare(JAVA_KEY, ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == JAVA_KEY
        1 * reporter.reportMatch()
    }

    def "reports mismatch and returns java key in shadow mode when keys differ"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, false)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >>
            new BuildCacheOrchestrationClient.CacheKeyResult(
                RUST_KEY.bytes, RUST_KEY, true, "")

        when:
        def result = computer.computeAndCompare(JAVA_KEY, ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == JAVA_KEY
        1 * reporter.reportMismatch("cache-key::test", _, _)
    }

    def "returns Rust key when authoritative and Rust succeeds"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, true)
        cacheOrchestration.computeCacheKeyStrict(_, _, _, _, _) >>
            new BuildCacheOrchestrationClient.CacheKeyResult(
                RUST_KEY.bytes, RUST_KEY, true, "")

        when:
        def result = computer.computeAndCompare(JAVA_KEY, ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == RUST_KEY
        // Still reports mismatch in authoritative mode for observability
        1 * reporter.reportMismatch("cache-key::test", _, _)
    }

    def "returns Rust key when authoritative and keys match"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, true)
        cacheOrchestration.computeCacheKeyStrict(_, _, _, _, _) >>
            new BuildCacheOrchestrationClient.CacheKeyResult(
                JAVA_KEY.bytes, JAVA_KEY, true, "")

        when:
        def result = computer.computeAndCompare(JAVA_KEY, ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == JAVA_KEY
        1 * reporter.reportMatch()
    }

    def "falls back to java key when authoritative but Rust fails"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, true)
        cacheOrchestration.computeCacheKeyStrict(_, _, _, _, _) >>
            BuildCacheOrchestrationClient.CacheKeyResult.error("daemon crash")

        when:
        def result = computer.computeAndCompare(JAVA_KEY, ":test", "impl",
            [:], [:], ["output1"])

        then:
        result == JAVA_KEY
        1 * reporter.reportRustError("cache-key::test", _)
    }

    def "uses strict cache-key call in authoritative mode"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, true)

        when:
        def result = computer.computeAndCompare(JAVA_KEY, ":test", "impl", [:], [:], ["output1"])

        then:
        result == RUST_KEY
        1 * cacheOrchestration.computeCacheKeyStrict(_, _, _, _, _) >>
            new BuildCacheOrchestrationClient.CacheKeyResult(
                RUST_KEY.bytes, RUST_KEY, true, "")
        0 * cacheOrchestration.computeCacheKey(_, _, _, _, _)
    }

    def "reports rust error and returns java key when cache orchestration throws"() {
        given:
        def computer = new ShadowingBuildCacheKeyComputer(cacheOrchestration, reporter, false)
        cacheOrchestration.computeCacheKey(_, _, _, _, _) >> { throw new RuntimeException("boom") }

        when:
        def result = computer.computeAndCompare(JAVA_KEY, ":test", "impl", [:], [:], ["output1"])

        then:
        result == JAVA_KEY
        1 * reporter.reportRustError("cache-key::test", _ as RuntimeException)
    }
}
