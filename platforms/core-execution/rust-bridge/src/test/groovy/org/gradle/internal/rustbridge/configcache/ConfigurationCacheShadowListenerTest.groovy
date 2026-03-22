package org.gradle.internal.rustbridge.configcache

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class ConfigurationCacheShadowListenerTest extends Specification {

    def "shadowStore reports match when Rust store succeeds"() {
        given:
        def client = Mock(RustConfigCacheClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new ConfigurationCacheShadowListener(client, reporter)
        def configBytes = "config-data".getBytes()

        when:
        listener.shadowStore("key-1", configBytes, 5, ["hash-a", "hash-b"])

        then:
        1 * client.storeConfigCache("key-1", configBytes, 5, ["hash-a", "hash-b"]) >> true
        1 * reporter.reportMatch()

        and: "stats reflect the store"
        listener.getStoreCount() == 1
    }

    def "shadowStore reports mismatch when Rust store fails"() {
        given:
        def client = Mock(RustConfigCacheClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new ConfigurationCacheShadowListener(client, reporter)
        def configBytes = "config-data".getBytes()

        when:
        listener.shadowStore("key-1", configBytes, 3, ["hash-a"])

        then:
        1 * client.storeConfigCache("key-1", configBytes, 3, ["hash-a"]) >> false
        1 * reporter.reportMismatch("config-cache:store:key-1", _, _ as byte[])

        and: "stats reflect the store"
        listener.getStoreCount() == 1
    }

    def "shadowStore reports rust error when Rust throws exception"() {
        given:
        def client = Mock(RustConfigCacheClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new ConfigurationCacheShadowListener(client, reporter)
        def configBytes = "config-data".getBytes()

        when:
        listener.shadowStore("key-1", configBytes, 2, ["hash-a"])

        then:
        1 * client.storeConfigCache("key-1", configBytes, 2, ["hash-a"]) >> {
            throw new RuntimeException("connection refused")
        }
        1 * reporter.reportRustError("config-cache:store:key-1", { it.message.contains("Rust store failed") })

        and: "stats still reflect the store attempt"
        listener.getStoreCount() == 1
    }

    def "shadowLoad reports match when both Java and Rust find and match"() {
        given:
        def client = Mock(RustConfigCacheClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new ConfigurationCacheShadowListener(client, reporter)
        def configBytes = "config-data".getBytes()

        when:
        listener.shadowLoad("key-1", configBytes, true)

        then:
        1 * client.loadConfigCache("key-1") >> Mock(RustConfigCacheClient.CacheLoadResult) {
            isFound() >> true
            getSerializedConfig() >> configBytes
        }
        1 * reporter.reportMatch()

        and: "stats reflect hit"
        listener.getLoadCount() == 1
        listener.getHitCount() == 1
        listener.getMissCount() == 0
    }

    def "shadowLoad reports mismatch when both found but bytes differ"() {
        given:
        def client = Mock(RustConfigCacheClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new ConfigurationCacheShadowListener(client, reporter)
        def javaBytes = "java-config".getBytes()
        def rustBytes = "rust-config-different".getBytes()

        when:
        listener.shadowLoad("key-1", javaBytes, true)

        then:
        1 * client.loadConfigCache("key-1") >> Mock(RustConfigCacheClient.CacheLoadResult) {
            isFound() >> true
            getSerializedConfig() >> rustBytes
        }
        1 * reporter.reportMismatch("config-cache:load:key-1", _, rustBytes)

        and: "stats still count as hit (both found) but mismatch reported"
        listener.getLoadCount() == 1
        listener.getHitCount() == 1
        listener.getMissCount() == 0
    }

    def "shadowLoad reports mismatch when Java found but Rust misses"() {
        given:
        def client = Mock(RustConfigCacheClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new ConfigurationCacheShadowListener(client, reporter)
        def javaBytes = "java-config".getBytes()

        when:
        listener.shadowLoad("key-1", javaBytes, true)

        then:
        1 * client.loadConfigCache("key-1") >> Mock(RustConfigCacheClient.CacheLoadResult) {
            isFound() >> false
            getSerializedConfig() >> new byte[0]
        }
        1 * reporter.reportMismatch("config-cache:load:key-1", _, _ as byte[])

        and: "stats reflect a miss"
        listener.getLoadCount() == 1
        listener.getMissCount() == 1
        listener.getHitCount() == 0
    }

    def "shadowValidate reports match when both agree"() {
        given:
        def client = Mock(RustConfigCacheClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new ConfigurationCacheShadowListener(client, reporter)

        when:
        listener.shadowValidate("key-1", ["hash-a", "hash-b"], true)

        then:
        1 * client.validateConfig("key-1", ["hash-a", "hash-b"]) >> Mock(RustConfigCacheClient.ValidationResult) {
            isValid() >> true
            getReason() >> null
        }
        1 * reporter.reportMatch()

        and: "stats reflect the validate"
        listener.getValidateCount() == 1
    }

    def "shadowValidate reports mismatch when Java and Rust disagree"() {
        given:
        def client = Mock(RustConfigCacheClient)
        def reporter = Mock(HashMismatchReporter)
        def listener = new ConfigurationCacheShadowListener(client, reporter)

        when:
        listener.shadowValidate("key-1", ["hash-a"], true)

        then:
        1 * client.validateConfig("key-1", ["hash-a"]) >> Mock(RustConfigCacheClient.ValidationResult) {
            isValid() >> false
            getReason() >> "input hashes changed"
        }
        1 * reporter.reportMismatch("config-cache:validate:key-1", _, _)

        and: "stats reflect the validate"
        listener.getValidateCount() == 1
    }
}
