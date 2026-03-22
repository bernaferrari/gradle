package org.gradle.internal.rustbridge.snapshot

import com.google.common.collect.ImmutableSortedMap
import org.gradle.api.internal.file.FileCollectionStructureVisitor
import org.gradle.internal.execution.InputFingerprinter
import org.gradle.internal.fingerprint.CurrentFileCollectionFingerprint
import org.gradle.internal.fingerprint.FileCollectionFingerprint
import org.gradle.internal.snapshot.ValueSnapshot
import spock.lang.Specification

class ShadowingInputFingerprinterTest extends Specification {

    def "implements InputFingerprinter"() {
        expect:
        ShadowingInputFingerprinter instanceof InputFingerprinter
    }

    def "delegates to the real InputFingerprinter"() {
        given:
        def delegate = Mock(InputFingerprinter)
        def fingerprinter = new ShadowingInputFingerprinter(delegate, null)
        def result = Mock(InputFingerprinter.Result)

        when:
        def actual = fingerprinter.fingerprintInputProperties(
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            {},
            Mock(FileCollectionStructureVisitor)
        )

        then:
        1 * delegate.fingerprintInputProperties(_, _, _, _, _, _) >> result
        actual.is(result)
    }

    def "works with null shadowingSnapshotter (no shadow comparison)"() {
        given:
        def delegate = Mock(InputFingerprinter)
        def fingerprinter = new ShadowingInputFingerprinter(delegate, null)
        def result = Mock(InputFingerprinter.Result)

        when:
        def actual = fingerprinter.fingerprintInputProperties(
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            {},
            Mock(FileCollectionStructureVisitor)
        )

        then:
        1 * delegate.fingerprintInputProperties(_, _, _, _, _, _) >> result
        actual.is(result)
    }

    def "shadow comparison is fire-and-forget (does not affect result)"() {
        given:
        def delegate = Mock(InputFingerprinter)
        def shadowingSnapshotter = Mock(ShadowingValueSnapshotter)
        def fingerprinter = new ShadowingInputFingerprinter(delegate, shadowingSnapshotter)
        def result = Mock(InputFingerprinter.Result)

        when:
        def actual = fingerprinter.fingerprintInputProperties(
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            {},
            Mock(FileCollectionStructureVisitor)
        )

        then:
        1 * delegate.fingerprintInputProperties(_, _, _, _, _, _) >> result
        actual.is(result)
    }

    def "shadow comparison is fire-and-forget even when shadowingSnapshotter throws"() {
        given:
        def delegate = Mock(InputFingerprinter)
        def shadowingSnapshotter = Mock(ShadowingValueSnapshotter)
        def fingerprinter = new ShadowingInputFingerprinter(delegate, shadowingSnapshotter)
        def result = Mock(InputFingerprinter.Result)

        when:
        def actual = fingerprinter.fingerprintInputProperties(
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            { visitor ->
                visitor.visitInputProperty("myProp", { -> "value" } as InputFingerprinter.ValueSupplier)
            },
            Mock(FileCollectionStructureVisitor)
        )

        then:
        1 * delegate.fingerprintInputProperties(_, _, _, _, _, _) >> result
        1 * shadowingSnapshotter.snapshot({ it.containsKey("myProp") }, "") >> { throw new RuntimeException("shadow failed") }
        actual.is(result)
        noExceptionThrown()
    }

    def "shadow comparison calls snapshotter with collected property values"() {
        given:
        def delegate = Mock(InputFingerprinter)
        def shadowingSnapshotter = Mock(ShadowingValueSnapshotter)
        def fingerprinter = new ShadowingInputFingerprinter(delegate, shadowingSnapshotter)
        def result = Mock(InputFingerprinter.Result)

        when:
        fingerprinter.fingerprintInputProperties(
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            { visitor ->
                visitor.visitInputProperty("propA", { -> "valueA" } as InputFingerprinter.ValueSupplier)
                visitor.visitInputProperty("propB", { -> "valueB" } as InputFingerprinter.ValueSupplier)
            },
            Mock(FileCollectionStructureVisitor)
        )

        then:
        1 * delegate.fingerprintInputProperties(_, _, _, _, _, _) >> result
        1 * shadowingSnapshotter.snapshot({ it.size() == 2 && it["propA"] == "valueA" && it["propB"] == "valueB" }, "")
    }

    def "does not call snapshotter when no input properties are collected"() {
        given:
        def delegate = Mock(InputFingerprinter)
        def shadowingSnapshotter = Mock(ShadowingValueSnapshotter)
        def fingerprinter = new ShadowingInputFingerprinter(delegate, shadowingSnapshotter)
        def result = Mock(InputFingerprinter.Result)

        when:
        fingerprinter.fingerprintInputProperties(
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            ImmutableSortedMap.of(),
            {},
            Mock(FileCollectionStructureVisitor)
        )

        then:
        1 * delegate.fingerprintInputProperties(_, _, _, _, _, _) >> result
        0 * shadowingSnapshotter.snapshot(_, _)
    }
}
