package org.gradle.internal.rustbridge.history;

import com.google.common.collect.ImmutableSortedMap;
import com.google.common.collect.Interner;
import org.gradle.internal.execution.history.AfterExecutionState;
import org.gradle.internal.execution.history.CurrentFileCollectionFingerprint;
import org.gradle.internal.execution.history.PreviousExecutionState;
import org.gradle.internal.execution.history.impl.DefaultPreviousExecutionState;
import org.gradle.internal.execution.history.impl.DefaultPreviousExecutionStateSerializer;
import org.gradle.internal.execution.history.impl.FileCollectionFingerprintSerializer;
import org.gradle.internal.execution.history.impl.FileSystemSnapshotSerializer;
import org.gradle.internal.execution.history.impl.SerializableFileCollectionFingerprint;
import org.gradle.internal.fingerprint.FileCollectionFingerprint;
import org.gradle.internal.hash.ClassLoaderHierarchyHasher;
import org.gradle.internal.serialize.HashCodeSerializer;
import org.gradle.internal.serialize.kryo.KryoBackedDecoder;
import org.gradle.internal.serialize.kryo.KryoBackedEncoder;
import org.jspecify.annotations.Nullable;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;

import static com.google.common.collect.ImmutableSortedMap.copyOfSorted;
import static com.google.common.collect.Maps.transformValues;

/**
 * Serializes {@link AfterExecutionState} to bytes using Gradle's standard
 * {@link DefaultPreviousExecutionStateSerializer} backed by {@link KryoBackedEncoder}.
 *
 * <p>This produces the same byte representation that Java's {@code DefaultExecutionHistoryStore}
 * writes to its IndexedCache, ensuring Rust receives data it can eventually read back
 * using the same deserialization format.</p>
 */
public class BinaryEncoderExecutionHistorySerializer
    implements ShadowingExecutionHistoryStore.ExecutionHistorySerializer {

    private final DefaultPreviousExecutionStateSerializer serializer;

    public BinaryEncoderExecutionHistorySerializer(
        Interner<String> stringInterner,
        ClassLoaderHierarchyHasher classLoaderHasher
    ) {
        this.serializer = new DefaultPreviousExecutionStateSerializer(
            new FileCollectionFingerprintSerializer(stringInterner),
            new FileSystemSnapshotSerializer(stringInterner),
            classLoaderHasher,
            new HashCodeSerializer()
        );
    }

    @Override
    public byte[] serialize(AfterExecutionState state) {
        PreviousExecutionState previousState = new DefaultPreviousExecutionState(
            state.getOriginMetadata(),
            state.getCacheKey(),
            state.getImplementation(),
            state.getAdditionalImplementations(),
            state.getInputProperties(),
            prepareForSerialization(state.getInputFileProperties()),
            state.getOutputFilesProducedByWork(),
            state.isSuccessful()
        );

        ByteArrayOutputStream baos = new ByteArrayOutputStream();
        KryoBackedEncoder encoder = new KryoBackedEncoder(baos);
        try {
            serializer.write(encoder, previousState);
            encoder.flush();
        } catch (IOException e) {
            throw new RuntimeException("Failed to serialize execution history state", e);
        } catch (Exception e) {
            throw new RuntimeException("Failed to serialize execution history state", e);
        }
        return baos.toByteArray();
    }

    /**
     * Archives current fingerprints for serialization, same as
     * {@code DefaultExecutionHistoryStore.prepareForSerialization()}.
     */
    private static ImmutableSortedMap<String, FileCollectionFingerprint> prepareForSerialization(
        ImmutableSortedMap<String, CurrentFileCollectionFingerprint> fingerprints
    ) {
        return copyOfSorted(transformValues(
            fingerprints,
            value -> value.archive(SerializableFileCollectionFingerprint::new)
        ));
    }

    @Override
    @Nullable
    public PreviousExecutionState deserialize(byte[] data) {
        try {
            ByteArrayInputStream bais = new ByteArrayInputStream(data);
            KryoBackedDecoder decoder = new KryoBackedDecoder(bais);
            return serializer.read(decoder);
        } catch (Exception e) {
            return null;
        }
    }
}
