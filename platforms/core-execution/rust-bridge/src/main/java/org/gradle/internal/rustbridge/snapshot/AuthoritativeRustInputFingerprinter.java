package org.gradle.internal.rustbridge.snapshot;

import com.google.common.collect.ImmutableSet;
import com.google.common.collect.ImmutableSortedMap;
import org.gradle.api.internal.file.DelegatingFileCollectionStructureVisitor;
import org.gradle.api.internal.file.FileCollectionStructureVisitor;
import org.gradle.api.internal.file.FileTreeInternal;
import org.gradle.api.internal.file.collections.FileSystemMirroringFileTree;
import org.gradle.internal.MutableBoolean;
import org.gradle.internal.execution.FileCollectionFingerprinter;
import org.gradle.internal.execution.FileCollectionFingerprinterRegistry;
import org.gradle.internal.execution.FileCollectionSnapshotter;
import org.gradle.internal.execution.FileNormalizationSpec;
import org.gradle.internal.execution.InputFingerprinter;
import org.gradle.internal.execution.InputVisitor;
import org.gradle.internal.execution.impl.DefaultFileNormalizationSpec;
import org.gradle.internal.execution.impl.DefaultInputFingerprinter;
import org.gradle.internal.fingerprint.CurrentFileCollectionFingerprint;
import org.gradle.internal.fingerprint.FileCollectionFingerprint;
import org.gradle.internal.properties.InputBehavior;
import org.gradle.internal.rustbridge.SubstrateException;
import org.gradle.internal.snapshot.FileSystemSnapshot;
import org.gradle.internal.snapshot.ValueSnapshot;
import org.gradle.internal.snapshot.ValueSnapshottingException;

import java.io.File;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.TreeMap;
import java.util.function.Consumer;

/**
 * Input fingerprinter that batches authoritative Rust value snapshot validation
 * while preserving Gradle's file fingerprinting behavior.
 */
public class AuthoritativeRustInputFingerprinter implements InputFingerprinter {
    private static final String BATCH_PROPERTY_NAME = "value-snapshot-batch";

    private final FileCollectionSnapshotter snapshotter;
    private final FileCollectionFingerprinterRegistry fingerprinterRegistry;
    private final AuthoritativeRustValueSnapshotter valueSnapshotter;

    public AuthoritativeRustInputFingerprinter(
        FileCollectionSnapshotter snapshotter,
        FileCollectionFingerprinterRegistry fingerprinterRegistry,
        AuthoritativeRustValueSnapshotter valueSnapshotter
    ) {
        this.snapshotter = snapshotter;
        this.fingerprinterRegistry = fingerprinterRegistry;
        this.valueSnapshotter = valueSnapshotter;
    }

    @Override
    public Result fingerprintInputProperties(
        ImmutableSortedMap<String, ValueSnapshot> previousValueSnapshots,
        ImmutableSortedMap<String, ? extends FileCollectionFingerprint> previousFingerprints,
        ImmutableSortedMap<String, ValueSnapshot> knownCurrentValueSnapshots,
        ImmutableSortedMap<String, CurrentFileCollectionFingerprint> knownCurrentFingerprints,
        Consumer<InputVisitor> inputs,
        FileCollectionStructureVisitor validatingVisitor
    ) throws InputFingerprintingException, InputFileFingerprintingException {
        InputCollectingVisitor visitor = new InputCollectingVisitor(
            previousValueSnapshots,
            previousFingerprints,
            snapshotter,
            fingerprinterRegistry,
            valueSnapshotter,
            knownCurrentValueSnapshots,
            knownCurrentFingerprints,
            validatingVisitor
        );
        inputs.accept(visitor);
        return visitor.complete();
    }

    private static class InputCollectingVisitor implements InputVisitor {
        private final ImmutableSortedMap<String, ValueSnapshot> previousValueSnapshots;
        private final ImmutableSortedMap<String, ? extends FileCollectionFingerprint> previousFingerprints;
        private final FileCollectionSnapshotter snapshotter;
        private final FileCollectionFingerprinterRegistry fingerprinterRegistry;
        private final AuthoritativeRustValueSnapshotter valueSnapshotter;
        private final ImmutableSortedMap<String, ValueSnapshot> knownCurrentValueSnapshots;
        private final ImmutableSortedMap<String, CurrentFileCollectionFingerprint> knownCurrentFingerprints;
        private final FileCollectionStructureVisitor validatingVisitor;

        private final Map<String, Object> values = new LinkedHashMap<>();
        private final ImmutableSortedMap.Builder<String, CurrentFileCollectionFingerprint> fingerprintsBuilder = ImmutableSortedMap.naturalOrder();
        private final ImmutableSet.Builder<String> propertiesRequiringIsEmptyCheck = ImmutableSet.builder();

        private InputCollectingVisitor(
            ImmutableSortedMap<String, ValueSnapshot> previousValueSnapshots,
            ImmutableSortedMap<String, ? extends FileCollectionFingerprint> previousFingerprints,
            FileCollectionSnapshotter snapshotter,
            FileCollectionFingerprinterRegistry fingerprinterRegistry,
            AuthoritativeRustValueSnapshotter valueSnapshotter,
            ImmutableSortedMap<String, ValueSnapshot> knownCurrentValueSnapshots,
            ImmutableSortedMap<String, CurrentFileCollectionFingerprint> knownCurrentFingerprints,
            FileCollectionStructureVisitor validatingVisitor
        ) {
            this.previousValueSnapshots = previousValueSnapshots;
            this.previousFingerprints = previousFingerprints;
            this.snapshotter = snapshotter;
            this.fingerprinterRegistry = fingerprinterRegistry;
            this.valueSnapshotter = valueSnapshotter;
            this.knownCurrentValueSnapshots = knownCurrentValueSnapshots;
            this.knownCurrentFingerprints = knownCurrentFingerprints;
            this.validatingVisitor = validatingVisitor;
        }

        @Override
        public void visitInputProperty(String propertyName, ValueSupplier value) {
            if (knownCurrentValueSnapshots.containsKey(propertyName)) {
                return;
            }
            values.put(propertyName, value.getValue());
        }

        @Override
        public void visitInputFileProperty(String propertyName, InputBehavior behavior, InputFileValueSupplier value) {
            if (knownCurrentFingerprints.containsKey(propertyName)) {
                return;
            }

            FileCollectionFingerprint previousFingerprint = previousFingerprints.get(propertyName);
            FileNormalizationSpec normalizationSpec = DefaultFileNormalizationSpec.from(
                value.getNormalizer(),
                value.getDirectorySensitivity(),
                value.getLineEndingNormalization()
            );
            FileCollectionFingerprinter fingerprinter = fingerprinterRegistry.getFingerprinter(normalizationSpec);
            try {
                MutableBoolean containsArchiveTrees = new MutableBoolean(false);
                FileSystemSnapshot snapshot = snapshotter.snapshot(value.getFiles(), new DelegatingFileCollectionStructureVisitor(validatingVisitor) {
                    @Override
                    public void visitFileTreeBackedByFile(File file, FileTreeInternal fileTree, FileSystemMirroringFileTree sourceTree) {
                        super.visitFileTreeBackedByFile(file, fileTree, sourceTree);
                        containsArchiveTrees.set(true);
                    }
                });
                CurrentFileCollectionFingerprint fingerprint = fingerprinter.fingerprint(snapshot, previousFingerprint);
                fingerprintsBuilder.put(propertyName, fingerprint);
                if (containsArchiveTrees.get()) {
                    propertiesRequiringIsEmptyCheck.add(propertyName);
                }
            } catch (Exception e) {
                throw new InputFileFingerprintingException(propertyName, e);
            }
        }

        private Result complete() {
            ImmutableSortedMap<String, ValueSnapshot> valueSnapshots = snapshotValues();
            return new DefaultInputFingerprinter.InputFingerprints(
                knownCurrentValueSnapshots,
                valueSnapshots,
                knownCurrentFingerprints,
                fingerprintsBuilder.build(),
                propertiesRequiringIsEmptyCheck.build()
            );
        }

        private ImmutableSortedMap<String, ValueSnapshot> snapshotValues() {
            if (values.isEmpty()) {
                return ImmutableSortedMap.of();
            }
            try {
                Map<String, ValueSnapshot> currentSnapshots = valueSnapshotter.snapshotAll(
                    values,
                    previousValueSnapshots
                );
                return ImmutableSortedMap.copyOfSorted(new TreeMap<>(currentSnapshots));
            } catch (AuthoritativeRustValueSnapshotter.PropertySnapshottingException e) {
                throw new InputFingerprintingException(
                    e.getPropertyName(),
                    String.format("value '%s' cannot be serialized", e.getValue()),
                    e
                );
            } catch (SubstrateException e) {
                throw new InputFingerprintingException(BATCH_PROPERTY_NAME, "Rust value snapshot batch failed", e);
            } catch (ValueSnapshottingException e) {
                throw new InputFingerprintingException(BATCH_PROPERTY_NAME, "Rust value snapshot batch failed", e);
            }
        }
    }
}
