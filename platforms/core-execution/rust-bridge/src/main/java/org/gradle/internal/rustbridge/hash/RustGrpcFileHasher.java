package org.gradle.internal.rustbridge.hash;

import gradle.substrate.v1.FileToHash;
import gradle.substrate.v1.HashBatchRequest;
import gradle.substrate.v1.HashBatchResponse;
import gradle.substrate.v1.HashResult;
import org.gradle.internal.hash.FileHasher;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateClient;

import java.io.File;
import java.util.Collections;

/**
 * A {@link FileHasher} implementation that delegates to the Rust substrate daemon via gRPC.
 */
public class RustGrpcFileHasher implements FileHasher {

    private final SubstrateClient client;

    public RustGrpcFileHasher(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public HashCode hash(File file) {
        return hash(file, file.length(), file.lastModified());
    }

    @Override
    public HashCode hash(File file, long length, long lastModified) {
        HashBatchRequest request = HashBatchRequest.newBuilder()
            .addFiles(FileToHash.newBuilder()
                .setAbsolutePath(file.getAbsolutePath())
                .setLength(length)
                .setLastModified(lastModified)
                .build())
            .setAlgorithm("MD5")
            .build();

        HashBatchResponse response = client.getHashStub().hashBatch(request);
        if (response.getResultsCount() != 1) {
            throw new RuntimeException("Expected 1 hash result, got " + response.getResultsCount());
        }

        HashResult result = response.getResults(0);
        if (result.getError()) {
            throw new RuntimeException("Rust hash error for " + file + ": " + result.getErrorMessage());
        }

        return HashCode.fromBytes(result.getHashBytes().toByteArray());
    }
}
