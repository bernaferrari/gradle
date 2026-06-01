package org.gradle.internal.rustbridge.hash;

import org.gradle.internal.hash.ChecksumService;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.gradle.internal.rustbridge.filehashcache.RustFileHashCacheClient;
import org.jspecify.annotations.Nullable;

import java.io.File;
import java.util.Locale;

/**
 * Rust-backed checksum service for Gradle's raw artifact checksums.
 */
public class RustChecksumService implements ChecksumService {

    private final RustGrpcFileHasher md5;
    private final RustGrpcFileHasher sha1;
    private final RustGrpcFileHasher sha256;
    private final RustGrpcFileHasher sha512;

    public RustChecksumService(SubstrateClient client) {
        this(client, null, false);
    }

    public RustChecksumService(
        SubstrateClient client,
        @Nullable RustFileHashCacheClient fileHashCacheClient,
        boolean fileHashCacheAuthoritative
    ) {
        this.md5 = checksumHasher(client, fileHashCacheClient, fileHashCacheAuthoritative, "MD5", "md5-checksums");
        this.sha1 = checksumHasher(client, fileHashCacheClient, fileHashCacheAuthoritative, "SHA-1", "sha1-checksums");
        this.sha256 = checksumHasher(client, fileHashCacheClient, fileHashCacheAuthoritative, "SHA-256", "sha256-checksums");
        this.sha512 = checksumHasher(client, fileHashCacheClient, fileHashCacheAuthoritative, "SHA-512", "sha512-checksums");
    }

    private static RustGrpcFileHasher checksumHasher(
        SubstrateClient client,
        @Nullable RustFileHashCacheClient fileHashCacheClient,
        boolean fileHashCacheAuthoritative,
        String algorithm,
        String cacheKind
    ) {
        return new RustGrpcFileHasher(client, fileHashCacheClient, fileHashCacheAuthoritative, algorithm, cacheKind, false);
    }

    @Override
    public HashCode md5(File file) {
        return md5.hash(file);
    }

    @Override
    public HashCode sha1(File file) {
        return sha1.hash(file);
    }

    @Override
    public HashCode sha256(File file) {
        return sha256.hash(file);
    }

    @Override
    public HashCode sha512(File file) {
        return sha512.hash(file);
    }

    @Override
    public HashCode hash(File src, String algorithm) {
        switch (algorithm.toLowerCase(Locale.ROOT)) {
            case "md5":
                return md5(src);
            case "sha1":
            case "sha-1":
                return sha1(src);
            case "sha256":
            case "sha-256":
                return sha256(src);
            case "sha512":
            case "sha-512":
                return sha512(src);
        }
        throw new UnsupportedOperationException("Cannot hash with algorithm " + algorithm);
    }
}
