package org.gradle.internal.rustbridge.snapshot

import org.gradle.internal.rustbridge.SubstrateException
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import spock.lang.Specification

class ShadowingValueSnapshotterTest extends Specification {

    def reporter = Mock(HashMismatchReporter)
    def javaDelegate = Mock(ShadowingValueSnapshotter.ValueSnapshotterDelegate)
    def rustClient = Mock(RustValueSnapshotClient)

    def "uses Java hash in shadow mode"() {
        given:
        def snapshotter = new ShadowingValueSnapshotter(javaDelegate, rustClient, reporter, false)
        def properties = [key: "value"]
        javaDelegate.snapshot(properties) >> "java-hash".bytes
        rustClient.snapshotValues(properties, "fp") >>
            RustValueSnapshotClient.SnapshotResult.success("rust-hash".bytes, [])

        when:
        def result = snapshotter.snapshot(properties, "fp")

        then:
        result == "java-hash".bytes
    }

    def "exposes authoritative flag"() {
        expect:
        !new ShadowingValueSnapshotter(javaDelegate, rustClient, reporter, false).isAuthoritative()
        new ShadowingValueSnapshotter(javaDelegate, rustClient, reporter, true).isAuthoritative()
    }

    def "reports match when Java and Rust hashes are equal"() {
        given:
        def snapshotter = new ShadowingValueSnapshotter(javaDelegate, rustClient, reporter, false)
        def hash = "match-hash".bytes
        def properties = [key: "value"]
        javaDelegate.snapshot(properties) >> hash
        rustClient.snapshotValues(properties, "fp") >>
            RustValueSnapshotClient.SnapshotResult.success(hash, [])

        when:
        snapshotter.snapshot(properties, "fp")

        then:
        1 * reporter.reportMatch()
    }

    def "reports mismatch when Java and Rust hashes differ"() {
        given:
        def snapshotter = new ShadowingValueSnapshotter(javaDelegate, rustClient, reporter, false)
        def properties = [key: "value"]
        javaDelegate.snapshot(properties) >> "java-hash".bytes
        rustClient.snapshotValues(properties, "fp") >>
            RustValueSnapshotClient.SnapshotResult.success("rust-hash".bytes, [])

        when:
        snapshotter.snapshot(properties, "fp")

        then:
        1 * reporter.reportMismatch("value-snapshot", _, _)
    }

    def "uses Rust hash in authoritative mode when Rust succeeds"() {
        given:
        def snapshotter = new ShadowingValueSnapshotter(javaDelegate, rustClient, reporter, true)
        def properties = [key: "value"]
        rustClient.snapshotValues(properties, "fp") >>
            RustValueSnapshotClient.SnapshotResult.success("rust-hash".bytes, [])

        when:
        def result = snapshotter.snapshot(properties, "fp")

        then:
        result == "rust-hash".bytes
        // Java delegate should NOT be called in authoritative success path
        0 * javaDelegate._
    }

    def "fails closed in authoritative mode when Rust returns an error"() {
        given:
        def snapshotter = new ShadowingValueSnapshotter(javaDelegate, rustClient, reporter, true)
        def properties = [key: "value"]
        rustClient.snapshotValues(properties, "fp") >>
            RustValueSnapshotClient.SnapshotResult.error("connection refused")

        when:
        snapshotter.snapshot(properties, "fp")

        then:
        def failure = thrown(SubstrateException)
        failure.message == "Authoritative Rust value snapshot failed: connection refused"
        1 * reporter.reportRustError("value-snapshot", _ as SubstrateException)
        0 * javaDelegate._
    }

    def "uses Rust in authoritative mode when properties are empty"() {
        given:
        def snapshotter = new ShadowingValueSnapshotter(javaDelegate, rustClient, reporter, true)
        rustClient.snapshotValues([:], "fp") >>
            RustValueSnapshotClient.SnapshotResult.success("rust-empty-hash".bytes, [])

        when:
        def result = snapshotter.snapshot([:], "fp")

        then:
        result == "rust-empty-hash".bytes
        0 * javaDelegate._
    }

    def "fails closed in authoritative mode when Rust client is unavailable"() {
        given:
        def snapshotter = new ShadowingValueSnapshotter(javaDelegate, null, reporter, true)

        when:
        snapshotter.snapshot([key: "value"], "fp")

        then:
        def failure = thrown(SubstrateException)
        failure.message == "Authoritative Rust value snapshotting is unavailable"
        1 * reporter.reportRustError("value-snapshot", _ as SubstrateException)
        0 * javaDelegate._
    }

    def "works with null Rust client in non-authoritative mode"() {
        given:
        def snapshotter = new ShadowingValueSnapshotter(javaDelegate, null, reporter, false)
        def properties = [key: "value"]
        javaDelegate.snapshot(properties) >> "java-hash".bytes

        when:
        def result = snapshotter.snapshot(properties, "fp")

        then:
        result == "java-hash".bytes
        0 * reporter._
    }
}
