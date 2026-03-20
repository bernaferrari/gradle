package org.gradle.internal.rustbridge.snapshot;

import gradle.substrate.v1.PropertyValue;
import gradle.substrate.v1.SnapshotValuesRequest;
import gradle.substrate.v1.SnapshotValuesResponse;
import gradle.substrate.v1.ValueSnapshotServiceGrpc;
import gradle.substrate.v1.ValueSnapshotResult;
import org.gradle.internal.rustbridge.SubstrateClient;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.ObjectOutputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * Client for the Rust value snapshot service.
 * Serializes Java objects to proto PropertyValue and calls the Rust snapshotValues RPC.
 */
public class RustValueSnapshotClient {

    private final SubstrateClient client;

    public RustValueSnapshotClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Snapshot a set of input properties via the Rust substrate daemon.
     *
     * @param properties map of property name to Java value
     * @param implementationFingerprint fingerprint of the task implementation class
     * @return snapshot result containing individual fingerprints and composite hash
     */
    public SnapshotResult snapshotValues(
        Map<String, Object> properties,
        String implementationFingerprint
    ) {
        if (client.isNoop()) {
            return SnapshotResult.error("Substrate client is in no-op mode");
        }

        try {
            List<PropertyValue> protoValues = new ArrayList<>();
            for (Map.Entry<String, Object> entry : properties.entrySet()) {
                protoValues.add(toPropertyValue(entry.getKey(), entry.getValue()));
            }

            SnapshotValuesResponse response = client.getValueSnapshotStub()
                .snapshotValues(SnapshotValuesRequest.newBuilder()
                    .addAllValues(protoValues)
                    .setImplementationFingerprint(implementationFingerprint)
                    .build());

            if (!response.getSuccess()) {
                return SnapshotResult.error(response.getErrorMessage());
            }

            return SnapshotResult.success(
                response.getCompositeHash().toByteArray(),
                response.getResultsList()
            );
        } catch (Exception e) {
            return SnapshotResult.error("Rust snapshot failed: " + e.getMessage());
        }
    }

    private PropertyValue toPropertyValue(String name, Object value) {
        PropertyValue.Builder builder = PropertyValue.newBuilder()
            .setName(name)
            .setTypeName(value != null ? value.getClass().getName() : "null");

        if (value == null) {
            // Leave value unset (None in proto)
        } else if (value instanceof String) {
            builder.setStringValue((String) value);
        } else if (value instanceof Boolean) {
            builder.setBoolValue((Boolean) value);
        } else if (value instanceof Integer) {
            builder.setLongValue(((Integer) value).longValue());
        } else if (value instanceof Long) {
            builder.setLongValue((Long) value);
        } else if (value instanceof List) {
            builder.setListValue(serializeToString(value));
        } else if (value instanceof Map) {
            builder.setMapValue(serializeToString(value));
        } else {
            builder.setBinaryValue(serializeToBytes(value));
        }

        return builder.build();
    }

    private String serializeToString(Object value) {
        return value.toString();
    }

    private com.google.protobuf.ByteString serializeToBytes(Object value) {
        try {
            ByteArrayOutputStream baos = new ByteArrayOutputStream();
            ObjectOutputStream oos = new ObjectOutputStream(baos);
            oos.writeObject(value);
            oos.close();
            return com.google.protobuf.ByteString.copyFrom(baos.toByteArray());
        } catch (IOException e) {
            return com.google.protobuf.ByteString.copyFromUtf8(value.toString());
        }
    }

    /**
     * Result of a Rust value snapshot call.
     */
    public static class SnapshotResult {
        private final boolean success;
        private final byte[] compositeHash;
        private final List<ValueSnapshotResult> results;
        private final String errorMessage;

        private SnapshotResult(boolean success, byte[] compositeHash,
                              List<ValueSnapshotResult> results, String errorMessage) {
            this.success = success;
            this.compositeHash = compositeHash;
            this.results = results;
            this.errorMessage = errorMessage;
        }

        public static SnapshotResult success(byte[] compositeHash, List<ValueSnapshotResult> results) {
            return new SnapshotResult(true, compositeHash, results, null);
        }

        public static SnapshotResult error(String errorMessage) {
            return new SnapshotResult(false, null, null, errorMessage);
        }

        public boolean isSuccess() { return success; }
        public byte[] getCompositeHash() { return compositeHash; }
        public List<ValueSnapshotResult> getResults() { return results; }
        public String getErrorMessage() { return errorMessage; }
    }
}
