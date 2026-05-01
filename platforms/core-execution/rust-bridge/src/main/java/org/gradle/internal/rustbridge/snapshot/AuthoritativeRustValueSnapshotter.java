package org.gradle.internal.rustbridge.snapshot;

import com.google.common.collect.ImmutableList;
import com.google.common.collect.ImmutableSet;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateException;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.snapshot.ValueSnapshot;
import org.gradle.internal.snapshot.ValueSnapshotter;
import org.gradle.internal.snapshot.ValueSnapshottingException;
import org.gradle.internal.snapshot.impl.ArrayOfPrimitiveValueSnapshot;
import org.gradle.internal.snapshot.impl.ArrayValueSnapshot;
import org.gradle.internal.snapshot.impl.BooleanValueSnapshot;
import org.gradle.internal.snapshot.impl.EnumValueSnapshot;
import org.gradle.internal.snapshot.impl.FileValueSnapshot;
import org.gradle.internal.snapshot.impl.HashCodeSnapshot;
import org.gradle.internal.snapshot.impl.IntegerValueSnapshot;
import org.gradle.internal.snapshot.impl.ListValueSnapshot;
import org.gradle.internal.snapshot.impl.LongValueSnapshot;
import org.gradle.internal.snapshot.impl.MapEntrySnapshot;
import org.gradle.internal.snapshot.impl.MapValueSnapshot;
import org.gradle.internal.snapshot.impl.NullValueSnapshot;
import org.gradle.internal.snapshot.impl.SetValueSnapshot;
import org.gradle.internal.snapshot.impl.ShortValueSnapshot;
import org.gradle.internal.snapshot.impl.StringValueSnapshot;
import org.jspecify.annotations.Nullable;

import java.io.File;
import java.lang.reflect.Array;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

/**
 * Authoritative value snapshotter for values whose Gradle semantics can be
 * represented without Java serialization, classloader hashes, or live Gradle
 * model objects.
 */
public class AuthoritativeRustValueSnapshotter implements ValueSnapshotter {
    private static final String PROPERTY_NAME = "value";

    private final RustValueSnapshotClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public AuthoritativeRustValueSnapshotter(
        RustValueSnapshotClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    @Override
    public ValueSnapshot snapshot(@Nullable Object value) throws ValueSnapshottingException {
        SnapshotAndCanonical snapshot = snapshotSupportedValue(value, new IdentityHashMap<>());
        confirmRustSnapshots(Collections.singletonMap(PROPERTY_NAME, snapshot.canonical));
        return snapshot.snapshot;
    }

    @Override
    public ValueSnapshot snapshot(@Nullable Object value, ValueSnapshot candidate) throws ValueSnapshottingException {
        SnapshotAndCanonical snapshot = snapshotSupportedValue(value, new IdentityHashMap<>());
        confirmRustSnapshots(Collections.singletonMap(PROPERTY_NAME, snapshot.canonical));
        if (snapshot.snapshot.equals(candidate)) {
            return candidate;
        }
        return snapshot.snapshot;
    }

    public Map<String, ValueSnapshot> snapshotAll(
        Map<String, Object> values,
        Map<String, ValueSnapshot> candidateSnapshots
    ) throws ValueSnapshottingException {
        if (values.isEmpty()) {
            return new TreeMap<>();
        }

        Map<String, SnapshotAndCanonical> snapshots = new LinkedHashMap<>();
        Map<String, String> canonicalValues = new LinkedHashMap<>();
        for (Map.Entry<String, Object> entry : values.entrySet()) {
            try {
                SnapshotAndCanonical snapshot = snapshotSupportedValue(entry.getValue(), new IdentityHashMap<>());
                snapshots.put(entry.getKey(), snapshot);
                canonicalValues.put(entry.getKey(), snapshot.canonical);
            } catch (ValueSnapshottingException e) {
                throw new PropertySnapshottingException(entry.getKey(), entry.getValue(), e);
            }
        }

        confirmRustSnapshots(canonicalValues);

        Map<String, ValueSnapshot> result = new TreeMap<>();
        for (Map.Entry<String, SnapshotAndCanonical> entry : snapshots.entrySet()) {
            ValueSnapshot snapshot = entry.getValue().snapshot;
            ValueSnapshot candidate = candidateSnapshots.get(entry.getKey());
            result.put(entry.getKey(), candidate != null && snapshot.equals(candidate) ? candidate : snapshot);
        }
        return result;
    }

    private void confirmRustSnapshots(Map<String, String> canonicalValues) {
        try {
            RustValueSnapshotClient.SnapshotResult result = rustClient.snapshotCanonicalValues(
                canonicalValues,
                ""
            );
            if (result.isSuccess() && result.getCompositeHash() != null) {
                return;
            }
            String message = result.getErrorMessage() == null
                ? "Rust value snapshot returned no composite hash"
                : result.getErrorMessage();
            SubstrateException failure = new SubstrateException("Authoritative Rust value snapshot failed: " + message);
            mismatchReporter.reportRustError("value-snapshot", failure);
            throw failure;
        } catch (SubstrateException e) {
            throw e;
        } catch (Exception e) {
            SubstrateException failure = new SubstrateException("Authoritative Rust value snapshot failed", e);
            mismatchReporter.reportRustError("value-snapshot", failure);
            throw failure;
        }
    }

    private SnapshotAndCanonical snapshotSupportedValue(@Nullable Object value, IdentityHashMap<Object, Boolean> visiting) {
        if (value == null) {
            return new SnapshotAndCanonical(NullValueSnapshot.INSTANCE, "N;");
        }
        if (value instanceof String) {
            return new SnapshotAndCanonical(new StringValueSnapshot((String) value), canonical("S", (String) value));
        }
        if (value instanceof Boolean) {
            boolean bool = (Boolean) value;
            return new SnapshotAndCanonical(bool ? BooleanValueSnapshot.TRUE : BooleanValueSnapshot.FALSE, bool ? "B:1;" : "B:0;");
        }
        if (value instanceof Integer) {
            Integer integer = (Integer) value;
            return new SnapshotAndCanonical(new IntegerValueSnapshot(integer), "I:" + integer + ";");
        }
        if (value instanceof Long) {
            Long longValue = (Long) value;
            return new SnapshotAndCanonical(new LongValueSnapshot(longValue), "J:" + longValue + ";");
        }
        if (value instanceof Short) {
            Short shortValue = (Short) value;
            return new SnapshotAndCanonical(new ShortValueSnapshot(shortValue), "H:" + shortValue + ";");
        }
        if (value instanceof HashCode) {
            HashCode hashCode = (HashCode) value;
            return new SnapshotAndCanonical(new HashCodeSnapshot(hashCode), canonical("G", hashCode.toString()));
        }
        if (value instanceof Enum<?>) {
            Enum<?> enumValue = (Enum<?>) value;
            String canonical = canonical("E", enumValue.getDeclaringClass().getName()) + canonical("e", enumValue.name());
            return new SnapshotAndCanonical(new EnumValueSnapshot(enumValue), canonical);
        }
        if (value.getClass().equals(File.class)) {
            File file = (File) value;
            return new SnapshotAndCanonical(new FileValueSnapshot(file), canonical("F", file.getPath()));
        }
        Class<?> valueClass = value.getClass();
        if (value instanceof List) {
            return snapshotList((List<?>) value, visiting);
        }
        if (value instanceof Set) {
            return snapshotSet((Set<?>) value, visiting);
        }
        if (value instanceof Map) {
            return snapshotMap((Map<?, ?>) value, visiting);
        }
        if (valueClass.isArray()) {
            return snapshotArray(value, visiting);
        }
        throw unsupported(value);
    }

    private SnapshotAndCanonical snapshotList(List<?> value, IdentityHashMap<Object, Boolean> visiting) {
        enter(value, visiting);
        try {
            if (value.isEmpty()) {
                return new SnapshotAndCanonical(ListValueSnapshot.EMPTY, "L:0;");
            }
            ImmutableList.Builder<ValueSnapshot> snapshots = ImmutableList.builderWithExpectedSize(value.size());
            StringBuilder canonical = new StringBuilder("L:").append(value.size()).append('[');
            for (Object element : value) {
                SnapshotAndCanonical elementSnapshot = snapshotSupportedValue(element, visiting);
                snapshots.add(elementSnapshot.snapshot);
                canonical.append(elementSnapshot.canonical);
            }
            canonical.append("];");
            return new SnapshotAndCanonical(new ListValueSnapshot(snapshots.build()), canonical.toString());
        } finally {
            visiting.remove(value);
        }
    }

    private SnapshotAndCanonical snapshotSet(Set<?> value, IdentityHashMap<Object, Boolean> visiting) {
        enter(value, visiting);
        try {
            ImmutableSet.Builder<ValueSnapshot> snapshots = ImmutableSet.builderWithExpectedSize(value.size());
            StringBuilder canonical = new StringBuilder("T:").append(value.size()).append('[');
            for (Object element : value) {
                SnapshotAndCanonical elementSnapshot = snapshotSupportedValue(element, visiting);
                snapshots.add(elementSnapshot.snapshot);
                canonical.append(elementSnapshot.canonical);
            }
            canonical.append("];");
            return new SnapshotAndCanonical(new SetValueSnapshot(snapshots.build()), canonical.toString());
        } finally {
            visiting.remove(value);
        }
    }

    private SnapshotAndCanonical snapshotMap(Map<?, ?> value, IdentityHashMap<Object, Boolean> visiting) {
        enter(value, visiting);
        try {
            ImmutableList.Builder<MapEntrySnapshot<ValueSnapshot>> snapshots = ImmutableList.builderWithExpectedSize(value.size());
            StringBuilder canonical = new StringBuilder("M:").append(value.size()).append('{');
            for (Map.Entry<?, ?> entry : value.entrySet()) {
                SnapshotAndCanonical key = snapshotSupportedValue(entry.getKey(), visiting);
                SnapshotAndCanonical mapValue = snapshotSupportedValue(entry.getValue(), visiting);
                snapshots.add(new MapEntrySnapshot<>(key.snapshot, mapValue.snapshot));
                canonical.append(key.canonical).append("=>").append(mapValue.canonical);
            }
            canonical.append("};");
            return new SnapshotAndCanonical(new MapValueSnapshot(snapshots.build()), canonical.toString());
        } finally {
            visiting.remove(value);
        }
    }

    private SnapshotAndCanonical snapshotArray(Object value, IdentityHashMap<Object, Boolean> visiting) {
        enter(value, visiting);
        try {
            int length = Array.getLength(value);
            Class<?> componentType = value.getClass().getComponentType();
            if (length == 0) {
                return new SnapshotAndCanonical(ArrayValueSnapshot.EMPTY, canonical("A0", componentType.getName()));
            }
            if (componentType.isPrimitive()) {
                return new SnapshotAndCanonical(new ArrayOfPrimitiveValueSnapshot(value), primitiveArrayCanonical(value, componentType, length));
            }
            ImmutableList.Builder<ValueSnapshot> snapshots = ImmutableList.builderWithExpectedSize(length);
            StringBuilder canonical = new StringBuilder("A:").append(componentType.getName()).append(':').append(length).append('[');
            for (int i = 0; i < length; i++) {
                SnapshotAndCanonical element = snapshotSupportedValue(Array.get(value, i), visiting);
                snapshots.add(element.snapshot);
                canonical.append(element.canonical);
            }
            canonical.append("];");
            return new SnapshotAndCanonical(new ArrayValueSnapshot(snapshots.build()), canonical.toString());
        } finally {
            visiting.remove(value);
        }
    }

    private static String primitiveArrayCanonical(Object value, Class<?> componentType, int length) {
        StringBuilder canonical = new StringBuilder("P:").append(componentType.getName()).append(':').append(length).append('[');
        for (int i = 0; i < length; i++) {
            Object element = Array.get(value, i);
            canonical.append(element).append(';');
        }
        return canonical.append("];").toString();
    }

    private static String canonical(String tag, String value) {
        return tag + ':' + value.length() + ':' + value + ';';
    }

    private static void enter(Object value, IdentityHashMap<Object, Boolean> visiting) {
        if (visiting.put(value, Boolean.TRUE) != null) {
            throw new ValueSnapshottingException("Authoritative Rust value snapshotting does not support cyclic values of type " + value.getClass().getName());
        }
    }

    private static ValueSnapshottingException unsupported(Object value) {
        return new ValueSnapshottingException("Authoritative Rust value snapshotting does not support values of type " + value.getClass().getName());
    }

    public static class PropertySnapshottingException extends ValueSnapshottingException {
        private final String propertyName;
        @Nullable
        private final Object value;

        public PropertySnapshottingException(String propertyName, @Nullable Object value, Throwable cause) {
            super("Authoritative Rust value snapshotting failed for input property '" + propertyName + "'", cause);
            this.propertyName = propertyName;
            this.value = value;
        }

        public String getPropertyName() {
            return propertyName;
        }

        @Nullable
        public Object getValue() {
            return value;
        }
    }

    private static class SnapshotAndCanonical {
        private final ValueSnapshot snapshot;
        private final String canonical;

        private SnapshotAndCanonical(ValueSnapshot snapshot, String canonical) {
            this.snapshot = snapshot;
            this.canonical = canonical;
        }
    }
}
