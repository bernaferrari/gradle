package org.gradle.internal.rustbridge.gc

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class ShadowingGarbageCollectorTest extends Specification {

    def "compareBuildCacheGc reports match when entries removed agree"() {
        given:
        def rustResult = new RustGarbageCollectionClient.GcResult(10, 5000L, 90)
        def rustClient = Mock(RustGarbageCollectionClient) {
            gcBuildCache(86400000L, 1073741824L, false) >> rustResult
        }
        def reporter = Mock(HashMismatchReporter)
        def gc = new ShadowingGarbageCollector(rustClient, reporter)

        when:
        gc.compareBuildCacheGc(86400000L, 1073741824L, false, 10, 5000L)

        then:
        1 * reporter.reportMatch()
        0 * reporter.reportMismatch(_, _, _)
    }

    def "compareBuildCacheGc reports mismatch when entries removed differ"() {
        given:
        def rustResult = new RustGarbageCollectionClient.GcResult(8, 5000L, 92)
        def rustClient = Mock(RustGarbageCollectionClient) {
            gcBuildCache(86400000L, 1073741824L, false) >> rustResult
        }
        def reporter = Mock(HashMismatchReporter)
        def gc = new ShadowingGarbageCollector(rustClient, reporter)

        when:
        gc.compareBuildCacheGc(86400000L, 1073741824L, false, 10, 5000L)

        then:
        1 * reporter.reportMismatch("gc:build-cache:entriesRemoved", "10", "8")
    }

    def "compareBuildCacheGc reports mismatch when bytes recovered differ"() {
        given:
        def rustResult = new RustGarbageCollectionClient.GcResult(10, 4000L, 90)
        def rustClient = Mock(RustGarbageCollectionClient) {
            gcBuildCache(86400000L, 1073741824L, false) >> rustResult
        }
        def reporter = Mock(HashMismatchReporter)
        def gc = new ShadowingGarbageCollector(rustClient, reporter)

        when:
        gc.compareBuildCacheGc(86400000L, 1073741824L, false, 10, 5000L)

        then:
        1 * reporter.reportMismatch("gc:build-cache:bytesRecovered", "5000", "4000")
    }

    def "compareBuildCacheGc reports Rust error when null returned"() {
        given:
        def rustClient = Mock(RustGarbageCollectionClient) {
            gcBuildCache(_, _, _) >> null
        }
        def reporter = Mock(HashMismatchReporter)
        def gc = new ShadowingGarbageCollector(rustClient, reporter)

        when:
        gc.compareBuildCacheGc(86400000L, 1073741824L, false, 10, 5000L)

        then:
        1 * reporter.reportRustError("gc:build-cache", _ as RuntimeException)
    }

    def "compareExecutionHistoryGc reports match when entries agree"() {
        given:
        def rustResult = new RustGarbageCollectionClient.GcResult(20, 1000L, 80)
        def rustClient = Mock(RustGarbageCollectionClient) {
            gcExecutionHistory(86400000L, 10000, true) >> rustResult
        }
        def reporter = Mock(HashMismatchReporter)
        def gc = new ShadowingGarbageCollector(rustClient, reporter)

        when:
        gc.compareExecutionHistoryGc(86400000L, 10000, true, 20)

        then:
        1 * reporter.reportMatch()
    }

    def "compareExecutionHistoryGc reports mismatch"() {
        given:
        def rustResult = new RustGarbageCollectionClient.GcResult(15, 800L, 85)
        def rustClient = Mock(RustGarbageCollectionClient) {
            gcExecutionHistory(86400000L, 10000, true) >> rustResult
        }
        def reporter = Mock(HashMismatchReporter)
        def gc = new ShadowingGarbageCollector(rustClient, reporter)

        when:
        gc.compareExecutionHistoryGc(86400000L, 10000, true, 20)

        then:
        1 * reporter.reportMismatch("gc:execution-history:entriesRemoved", "20", "15")
    }

    def "compareConfigCacheGc reports match when entries agree"() {
        given:
        def rustResult = new RustGarbageCollectionClient.GcResult(5, 2000L, 15)
        def rustClient = Mock(RustGarbageCollectionClient) {
            gcConfigCache(604800000L, 1000, false) >> rustResult
        }
        def reporter = Mock(HashMismatchReporter)
        def gc = new ShadowingGarbageCollector(rustClient, reporter)

        when:
        gc.compareConfigCacheGc(604800000L, 1000, false, 5)

        then:
        1 * reporter.reportMatch()
    }

    def "compareConfigCacheGc reports mismatch"() {
        given:
        def rustResult = new RustGarbageCollectionClient.GcResult(3, 1200L, 17)
        def rustClient = Mock(RustGarbageCollectionClient) {
            gcConfigCache(604800000L, 1000, false) >> rustResult
        }
        def reporter = Mock(HashMismatchReporter)
        def gc = new ShadowingGarbageCollector(rustClient, reporter)

        when:
        gc.compareConfigCacheGc(604800000L, 1000, false, 5)

        then:
        1 * reporter.reportMismatch("gc:config-cache:entriesRemoved", "5", "3")
    }

    def "compareBuildCacheGc reports Rust error on exception"() {
        given:
        def rustClient = Mock(RustGarbageCollectionClient) {
            gcBuildCache(_, _, _) >> { throw new RuntimeException("rpc failed") }
        }
        def reporter = Mock(HashMismatchReporter)
        def gc = new ShadowingGarbageCollector(rustClient, reporter)

        when:
        gc.compareBuildCacheGc(86400000L, 1073741824L, false, 10, 5000L)

        then:
        1 * reporter.reportRustError("gc:build-cache", _ as RuntimeException)
    }
}
