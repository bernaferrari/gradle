package org.gradle.internal.rustbridge.toolchain

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class ShadowingToolchainProviderTest extends Specification {

    def "compareToolchainLists reports match when counts agree"() {
        given:
        def rustClient = Mock(RustToolchainServiceClient) {
            listToolchains("macos", "aarch64") >> []
        }
        def reporter = Mock(HashMismatchReporter)
        def provider = new ShadowingToolchainProvider(rustClient, reporter)

        when:
        provider.compareToolchainLists("macos", "aarch64", [])

        then:
        1 * reporter.reportMatch()
        0 * reporter.reportMismatch(_, _, _)
    }

    def "compareToolchainLists reports mismatch when counts differ"() {
        given:
        def rustClient = Mock(RustToolchainServiceClient) {
            listToolchains("linux", "x86_64") >> []
        }
        def reporter = Mock(HashMismatchReporter)
        def provider = new ShadowingToolchainProvider(rustClient, reporter)

        when:
        provider.compareToolchainLists("linux", "x86_64", ["/usr/lib/jvm/java-17"])

        then:
        1 * reporter.reportMismatch("toolchain-list:linux/x86_64", "1", "0")
    }

    def "compareToolchainLists reports Rust error and continues"() {
        given:
        def rustClient = Mock(RustToolchainServiceClient) {
            listToolchains(_, _) >> { throw new RuntimeException("daemon down") }
        }
        def reporter = Mock(HashMismatchReporter)
        def provider = new ShadowingToolchainProvider(rustClient, reporter)

        when:
        provider.compareToolchainLists("windows", "x86_64", [])

        then:
        1 * reporter.reportRustError("toolchain-list:windows/x86_64", _ as RuntimeException)
    }

    def "compareToolchainVerification reports match when both valid"() {
        given:
        def rustResponse = gradle.substrate.v1.VerifyToolchainResponse.newBuilder()
            .setValid(true).build()
        def rustClient = Mock(RustToolchainServiceClient) {
            verifyToolchain("/path/to/jdk", "17") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def provider = new ShadowingToolchainProvider(rustClient, reporter)

        when:
        provider.compareToolchainVerification("/path/to/jdk", "17", true, null)

        then:
        1 * reporter.reportMatch()
    }

    def "compareToolchainVerification reports mismatch when results differ"() {
        given:
        def rustResponse = gradle.substrate.v1.VerifyToolchainResponse.newBuilder()
            .setValid(false).build()
        def rustClient = Mock(RustToolchainServiceClient) {
            verifyToolchain("/bad/jdk", "17") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def provider = new ShadowingToolchainProvider(rustClient, reporter)

        when:
        provider.compareToolchainVerification("/bad/jdk", "17", true, null)

        then:
        1 * reporter.reportMismatch("toolchain-verify:/bad/jdk", "true", "false")
    }

    def "compareJavaHomeLookup reports match when paths agree"() {
        given:
        def rustResponse = gradle.substrate.v1.GetJavaHomeResponse.newBuilder()
            .setJavaHome("/usr/lib/jvm/java-17").build()
        def rustClient = Mock(RustToolchainServiceClient) {
            getJavaHome("17", "hotspot") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def provider = new ShadowingToolchainProvider(rustClient, reporter)

        when:
        provider.compareJavaHomeLookup("17", "hotspot", "/usr/lib/jvm/java-17")

        then:
        1 * reporter.reportMatch()
    }

    def "compareJavaHomeLookup reports mismatch when paths differ"() {
        given:
        def rustResponse = gradle.substrate.v1.GetJavaHomeResponse.newBuilder()
            .setJavaHome("/other/jdk").build()
        def rustClient = Mock(RustToolchainServiceClient) {
            getJavaHome("17", "hotspot") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def provider = new ShadowingToolchainProvider(rustClient, reporter)

        when:
        provider.compareJavaHomeLookup("17", "hotspot", "/usr/lib/jvm/java-17")

        then:
        1 * reporter.reportMismatch("java-home:17:hotspot", "/usr/lib/jvm/java-17", "/other/jdk")
    }

    def "compareJavaHomeLookup reports match when both null/empty"() {
        given:
        def rustResponse = gradle.substrate.v1.GetJavaHomeResponse.newBuilder()
            .setJavaHome("").build()
        def rustClient = Mock(RustToolchainServiceClient) {
            getJavaHome("21", "hotspot") >> rustResponse
        }
        def reporter = Mock(HashMismatchReporter)
        def provider = new ShadowingToolchainProvider(rustClient, reporter)

        when:
        provider.compareJavaHomeLookup("21", "hotspot", null)

        then:
        1 * reporter.reportMatch()
    }

    def "four-arg constructor sets authoritative flag"() {
        given:
        def rustClient = Mock(RustToolchainServiceClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def provider = new ShadowingToolchainProvider(rustClient, reporter, true)

        then:
        provider.isAuthoritative()
    }

    def "three-arg constructor defaults to non-authoritative"() {
        given:
        def rustClient = Mock(RustToolchainServiceClient)
        def reporter = Mock(HashMismatchReporter)

        when:
        def provider = new ShadowingToolchainProvider(rustClient, reporter)

        then:
        !provider.isAuthoritative()
    }
}
