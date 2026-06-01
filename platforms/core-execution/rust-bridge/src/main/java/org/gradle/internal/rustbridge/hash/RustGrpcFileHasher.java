package org.gradle.internal.rustbridge.hash;

import gradle.substrate.v1.FileToHash;
import gradle.substrate.v1.HashBatchRequest;
import gradle.substrate.v1.HashBatchResponse;
import gradle.substrate.v1.HashResult;
import org.gradle.internal.hash.FileHasher;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.gradle.internal.rustbridge.SubstrateException;
import org.gradle.internal.rustbridge.filehashcache.RustFileHashCacheClient;
import org.jspecify.annotations.Nullable;

import java.io.File;

/**
 * A {@link FileHasher} implementation that delegates to the Rust substrate daemon via gRPC.
 */
public class RustGrpcFileHasher implements FileHasher {

    private static final String FILE_HASH_KIND = "FILE_HASHES";

    private final SubstrateClient client;
    private final @Nullable RustFileHashCacheClient fileHashCacheClient;
    private final boolean fileHashCacheAuthoritative;
    private final String algorithm;
    private final String fileHashCacheKind;
    private final boolean gradleSignature;

    public RustGrpcFileHasher(SubstrateClient client) {
        this(client, null, false);
    }

    public RustGrpcFileHasher(
        SubstrateClient client,
        @Nullable RustFileHashCacheClient fileHashCacheClient,
        boolean fileHashCacheAuthoritative
    ) {
        this(client, fileHashCacheClient, fileHashCacheAuthoritative, "MD5", FILE_HASH_KIND, true);
    }

    public RustGrpcFileHasher(
        SubstrateClient client,
        @Nullable RustFileHashCacheClient fileHashCacheClient,
        boolean fileHashCacheAuthoritative,
        String algorithm,
        String fileHashCacheKind,
        boolean gradleSignature
    ) {
        this.client = client;
        this.fileHashCacheClient = fileHashCacheClient;
        this.fileHashCacheAuthoritative = fileHashCacheAuthoritative;
        this.algorithm = algorithm;
        this.fileHashCacheKind = fileHashCacheKind;
        this.gradleSignature = gradleSignature;
    }

    @Override
    public HashCode hash(File file) {
        return hash(file, file.length(), file.lastModified());
    }

    @Override
    public HashCode hash(File file, long length, long lastModified) {
        String absolutePath = file.getAbsolutePath();
        HashCode cached = loadFromFileHashCache(absolutePath, length, lastModified);
        if (cached != null) {
            return cached;
        }

        HashBatchRequest request = HashBatchRequest.newBuilder()
            .addFiles(FileToHash.newBuilder()
                .setAbsolutePath(absolutePath)
                .setLength(length)
                .setLastModified(lastModified)
                .build())
            .setAlgorithm(algorithm)
            .setGradleSignature(gradleSignature)
            .build();

        HashBatchResponse response = client.getHashStub().hashBatch(request);
        if (response.getResultsCount() != 1) {
            throw new RuntimeException("Expected 1 hash result, got " + response.getResultsCount());
        }

        HashResult result = response.getResults(0);
        if (result.getError()) {
            throw new RuntimeException("Rust hash error for " + file + ": " + result.getErrorMessage());
        }

        HashCode hash = HashCode.fromBytes(result.getHashBytes().toByteArray());
        storeInFileHashCache(absolutePath, hash, length, lastModified);
        return hash;
    }

    @Nullable
    private HashCode loadFromFileHashCache(String absolutePath, long length, long lastModified) {
        if (fileHashCacheClient == null) {
            return null;
        }
        RustFileHashCacheClient.FileInfoResult result =
            fileHashCacheClient.getFileInfo(absolutePath, length, lastModified, fileHashCacheKind);
        if (!result.isSuccess()) {
            if (fileHashCacheAuthoritative) {
                throw new SubstrateException(
                    "Authoritative Rust file hash cache get failed for " + absolutePath + ": " + result.getErrorMessage()
                );
            }
            return null;
        }
        return result.getInfo()
            .filter(info -> info.getLength() == length && info.getLastModified() == lastModified)
            .map(RustFileHashCacheClient.FileInfoData::getHash)
            .orElse(null);
    }

    private void storeInFileHashCache(String absolutePath, HashCode hash, long length, long lastModified) {
        if (fileHashCacheClient == null) {
            return;
        }
        boolean stored = fileHashCacheClient.putFileInfo(absolutePath, hash, length, lastModified, fileHashCacheKind);
        if (!stored && fileHashCacheAuthoritative) {
            throw new SubstrateException("Authoritative Rust file hash cache put failed for " + absolutePath);
        }
    }
}
