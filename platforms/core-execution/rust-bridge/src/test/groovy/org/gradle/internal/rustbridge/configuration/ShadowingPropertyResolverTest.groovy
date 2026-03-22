package org.gradle.internal.rustbridge.configuration

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class ShadowingPropertyResolverTest extends Specification {

    def rustClient = Mock(RustConfigurationClient)
    def reporter = Mock(HashMismatchReporter)

    def "constructor stores dependencies"() {
        when:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)

        then:
        resolver != null
    }

    def "registerProject delegates to rustClient"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)

        when:
        resolver.registerProject(":app", "/tmp/app", ["version": "1.0"], ["java"])

        then:
        1 * rustClient.registerProject(":app", "/tmp/app", ["version": "1.0"], ["java"])
    }

    def "registerProject handles exceptions gracefully"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)

        when:
        resolver.registerProject(":app", "/tmp/app", [:], [])

        then:
        1 * rustClient.registerProject(_, _, _, _) >> { throw new RuntimeException("gRPC connection refused") }
        noExceptionThrown()
    }

    def "shadowResolveProperty with null rustClient returns immediately"() {
        given:
        def resolver = new ShadowingPropertyResolver(null, reporter)

        when:
        resolver.shadowResolveProperty(":app", "version", "1.0")

        then:
        0 * reporter._
    }

    def "shadowResolveProperty reports match when values are equal"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)
        def propertyResult = Mock(RustConfigurationClient.PropertyResult)
        propertyResult.isSuccess() >> true
        propertyResult.isFound() >> true
        propertyResult.getValue() >> "1.0"

        when:
        resolver.shadowResolveProperty(":app", "version", "1.0")

        then:
        1 * rustClient.resolveProperty(":app", "version", "shadow") >> propertyResult
        1 * reporter.reportMatch()
        0 * reporter.reportRustError(_, _)
    }

    def "shadowResolveProperty reports mismatch when values differ"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)
        def propertyResult = Mock(RustConfigurationClient.PropertyResult)
        propertyResult.isSuccess() >> true
        propertyResult.isFound() >> true
        propertyResult.getValue() >> "2.0"

        when:
        resolver.shadowResolveProperty(":app", "version", "1.0")

        then:
        1 * rustClient.resolveProperty(":app", "version", "shadow") >> propertyResult
        1 * reporter.reportRustError(":app:version", _ as RuntimeException)
        0 * reporter.reportMatch()
    }

    def "shadowResolveProperty reports Rust error when result is not successful"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)
        def propertyResult = Mock(RustConfigurationClient.PropertyResult)
        propertyResult.isSuccess() >> false
        propertyResult.getErrorMessage() >> "Substrate not available"

        when:
        resolver.shadowResolveProperty(":app", "version", "1.0")

        then:
        1 * rustClient.resolveProperty(":app", "version", "shadow") >> propertyResult
        1 * reporter.reportRustError(":app:version", _ as RuntimeException)
        0 * reporter.reportMatch()
    }

    def "shadowResolveProperty handles Rust failure gracefully"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)

        when:
        resolver.shadowResolveProperty(":app", "version", "1.0")

        then:
        1 * rustClient.resolveProperty(":app", "version", "shadow") >> { throw new RuntimeException("connection lost") }
        1 * reporter.reportRustError(":app:version", _ as RuntimeException)
        noExceptionThrown()
    }

    def "shadowResolveProperty reports mismatch when Java value is null"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)
        def propertyResult = Mock(RustConfigurationClient.PropertyResult)
        propertyResult.isSuccess() >> true
        propertyResult.isFound() >> true
        propertyResult.getValue() >> "1.0"

        when:
        resolver.shadowResolveProperty(":app", "version", null)

        then:
        1 * rustClient.resolveProperty(":app", "version", "shadow") >> propertyResult
        1 * reporter.reportRustError(":app:version", _ as RuntimeException)
        0 * reporter.reportMatch()
    }

    def "shadowValidateConfigCache reports match when both agree valid"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)
        def validationResult = Mock(RustConfigurationClient.ValidationResult)
        validationResult.isSuccess() >> true
        validationResult.isValid() >> true

        when:
        resolver.shadowValidateConfigCache(":app", "hash".getBytes(), [], [], true)

        then:
        1 * rustClient.validateConfigCache(":app", "hash".getBytes(), [], []) >> validationResult
        1 * reporter.reportMatch()
        0 * reporter.reportRustError(_, _)
    }

    def "shadowValidateConfigCache reports match when both agree invalid"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)
        def validationResult = Mock(RustConfigurationClient.ValidationResult)
        validationResult.isSuccess() >> true
        validationResult.isValid() >> false
        validationResult.getReason() >> "inputs changed"

        when:
        resolver.shadowValidateConfigCache(":app", "hash".getBytes(), [], [], false)

        then:
        1 * rustClient.validateConfigCache(":app", "hash".getBytes(), [], []) >> validationResult
        1 * reporter.reportMatch()
        0 * reporter.reportRustError(_, _)
    }

    def "shadowValidateConfigCache reports mismatch when Java and Rust disagree"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)
        def validationResult = Mock(RustConfigurationClient.ValidationResult)
        validationResult.isSuccess() >> true
        validationResult.isValid() >> false
        validationResult.getReason() >> "hash mismatch"

        when:
        resolver.shadowValidateConfigCache(":app", "hash".getBytes(), [], [], true)

        then:
        1 * rustClient.validateConfigCache(":app", "hash".getBytes(), [], []) >> validationResult
        1 * reporter.reportRustError("config-cache::app", _ as RuntimeException)
        0 * reporter.reportMatch()
    }

    def "shadowValidateConfigCache with null rustClient returns immediately"() {
        given:
        def resolver = new ShadowingPropertyResolver(null, reporter)

        when:
        resolver.shadowValidateConfigCache(":app", "hash".getBytes(), [], [], true)

        then:
        0 * reporter._
    }

    def "shadowValidateConfigCache handles Rust failure gracefully"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)

        when:
        resolver.shadowValidateConfigCache(":app", "hash".getBytes(), [], [], true)

        then:
        1 * rustClient.validateConfigCache(_, _, _, _) >> { throw new RuntimeException("gRPC timeout") }
        1 * reporter.reportRustError("config-cache::app", _ as RuntimeException)
        noExceptionThrown()
    }

    def "shadowValidateConfigCache does not report when Rust result is not successful"() {
        given:
        def resolver = new ShadowingPropertyResolver(rustClient, reporter)
        def validationResult = Mock(RustConfigurationClient.ValidationResult)
        validationResult.isSuccess() >> false

        when:
        resolver.shadowValidateConfigCache(":app", "hash".getBytes(), [], [], true)

        then:
        1 * rustClient.validateConfigCache(":app", "hash".getBytes(), [], []) >> validationResult
        0 * reporter.reportMatch()
        0 * reporter.reportRustError(_, _)
    }
}
