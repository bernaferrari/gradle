package org.gradle.internal.rustbridge.snapshot;

import com.google.common.collect.ImmutableSortedMap;
import org.gradle.api.internal.file.FileCollectionStructureVisitor;
import org.gradle.api.logging.Logging;
import org.gradle.internal.execution.InputFingerprinter;
import org.gradle.internal.execution.InputVisitor;
import org.gradle.internal.fingerprint.CurrentFileCollectionFingerprint;
import org.gradle.internal.fingerprint.FileCollectionFingerprint;
import org.gradle.internal.properties.InputBehavior;
import org.gradle.internal.snapshot.ValueSnapshot;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.Consumer;

/**
 * An {@link InputFingerprinter} that wraps a delegate and intercepts input property values
 * to perform shadow comparison via {@link ShadowingValueSnapshotter}.
 *
 * <p>The shadow comparison is fire-and-forget: it never affects the result returned to the caller.
 * Failures are logged at debug level and swallowed.</p>
 */
public class ShadowingInputFingerprinter implements InputFingerprinter {

    private static final Logger LOGGER = Logging.getLogger(ShadowingInputFingerprinter.class);

    private final InputFingerprinter delegate;
    @Nullable
    private final ShadowingValueSnapshotter shadowingSnapshotter;

    public ShadowingInputFingerprinter(
        InputFingerprinter delegate,
        @Nullable ShadowingValueSnapshotter shadowingSnapshotter
    ) {
        this.delegate = delegate;
        this.shadowingSnapshotter = shadowingSnapshotter;
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
        Map<String, Object> collected = new LinkedHashMap<>();

        // Wrap the inputs consumer to intercept property values
        Consumer<InputVisitor> recordingConsumer = visitor -> inputs.accept(new InputVisitor() {
            @Override
            public void visitInputProperty(String propertyName, ValueSupplier value) {
                Object actualValue = value.getValue();
                if (actualValue != null) {
                    collected.put(propertyName, actualValue);
                }
                visitor.visitInputProperty(propertyName, value);
            }

            @Override
            public void visitInputFileProperty(String propertyName, InputBehavior behavior, InputFileValueSupplier value) {
                visitor.visitInputFileProperty(propertyName, behavior, value);
            }
        });

        Result result = delegate.fingerprintInputProperties(
            previousValueSnapshots,
            previousFingerprints,
            knownCurrentValueSnapshots,
            knownCurrentFingerprints,
            recordingConsumer,
            validatingVisitor
        );

        // Shadow compare (fire-and-forget, doesn't affect result)
        if (shadowingSnapshotter != null && !collected.isEmpty()) {
            try {
                shadowingSnapshotter.snapshot(collected, "");
            } catch (Exception e) {
                LOGGER.debug("[substrate:snapshot] shadow comparison failed for input fingerprinting", e);
            }
        }

        return result;
    }
}
