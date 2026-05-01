package org.gradle.internal.rustbridge.snapshot

import org.gradle.internal.rustbridge.SubstrateException
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter
import org.gradle.internal.snapshot.ValueSnapshottingException
import org.gradle.internal.snapshot.impl.ListValueSnapshot
import org.gradle.internal.snapshot.impl.StringValueSnapshot
import spock.lang.Specification

class AuthoritativeRustValueSnapshotterTest extends Specification {

    def rustClient = Mock(RustValueSnapshotClient)
    def reporter = Mock(HashMismatchReporter)
    def snapshotter = new AuthoritativeRustValueSnapshotter(rustClient, reporter)

    def "snapshots supported scalar values after Rust accepts canonical payload"() {
        given:
        rustAccepts()

        when:
        def snapshot = snapshotter.snapshot("value")

        then:
        snapshot == new StringValueSnapshot("value")
    }

    def "snapshots supported container values after Rust accepts canonical payload"() {
        given:
        rustAccepts()

        when:
        def snapshot = snapshotter.snapshot(["one", 2, true])

        then:
        snapshot instanceof ListValueSnapshot
    }

    def "reuses candidate when supported value did not change"() {
        given:
        rustAccepts()
        def candidate = new StringValueSnapshot("value")

        expect:
        snapshotter.snapshot("value", candidate).is(candidate)
    }

    def "fails closed before Rust for unsupported JVM-only values"() {
        when:
        snapshotter.snapshot(new Object())

        then:
        def failure = thrown(ValueSnapshottingException)
        failure.message.contains("does not support values of type java.lang.Object")
        0 * rustClient._
    }

    def "fails closed when Rust rejects canonical payload"() {
        given:
        rustClient.snapshotCanonicalValues(_, "") >>
            RustValueSnapshotClient.SnapshotResult.error("connection refused")

        when:
        snapshotter.snapshot("value")

        then:
        def failure = thrown(SubstrateException)
        failure.message == "Authoritative Rust value snapshot failed: connection refused"
        1 * reporter.reportRustError("value-snapshot", _ as SubstrateException)
    }

    def "fails closed on cyclic container values"() {
        given:
        def list = []
        list.add(list)

        when:
        snapshotter.snapshot(list)

        then:
        def failure = thrown(ValueSnapshottingException)
        failure.message.contains("does not support cyclic values")
        0 * rustClient._
    }

    private void rustAccepts() {
        rustClient.snapshotCanonicalValues({ Map<String, String> values ->
            values.size() == 1 && values.containsKey("value") && values["value"]
        }, "") >> RustValueSnapshotClient.SnapshotResult.success("rust-hash".bytes, [])
    }
}
