package org.gradle.internal.rustbridge.fingerprint;

import gradle.substrate.v1.FileFingerprintEntry;
import gradle.substrate.v1.FileToFingerprint;
import gradle.substrate.v1.FingerprintFilesRequest;
import gradle.substrate.v1.FingerprintFilesResponse;
import gradle.substrate.v1.FingerprintType;
import org.gradle.api.logging.Logging;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.io.File;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;

/**
 * Client for the Rust file fingerprinting service.
 * Fingerprints files and directories via gRPC, returning content hashes
 * compatible with Gradle's MD5-based fingerprinting.
 */
public class RustFileFingerprintClient {

    private static final Logger LOGGER = Logging.getLogger(RustFileFingerprintClient.class);

    private final SubstrateClient client;

    public RustFileFingerprintClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of fingerprinting a file collection.
     */
    public static class FingerprintResult {
        private final List<IndividualFingerprint> entries;
        private final HashCode collectionHash;
        private final boolean success;
        private final String errorMessage;

        private FingerprintResult(List<IndividualFingerprint> entries, HashCode collectionHash,
                                  boolean success, String errorMessage) {
            this.entries = entries;
            this.collectionHash = collectionHash;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public List<IndividualFingerprint> getEntries() {
            return entries;
        }

        public HashCode getCollectionHash() {
            return collectionHash;
        }

        public boolean isSuccess() {
            return success;
        }

        public String getErrorMessage() {
            return errorMessage;
        }
    }

    /**
     * Individual file/directory fingerprint.
     */
    public static class IndividualFingerprint {
        private final String path;
        private final HashCode hash;
        private final long size;
        private final long lastModified;
        private final boolean isDirectory;

        private IndividualFingerprint(String path, HashCode hash, long size,
                                      long lastModified, boolean isDirectory) {
            this.path = path;
            this.hash = hash;
            this.size = size;
            this.lastModified = lastModified;
            this.isDirectory = isDirectory;
        }

        public String getPath() { return path; }
        public HashCode getHash() { return hash; }
        public long getSize() { return size; }
        public long getLastModified() { return lastModified; }
        public boolean isDirectory() { return isDirectory; }
    }

    /**
     * Fingerprint a collection of files/directories.
     *
     * @param files map of absolute path to type (FILE or DIRECTORY)
     * @param normalizationStrategy the normalization strategy identifier
     * @param ignorePatterns glob patterns to ignore (e.g. ".DS_Store")
     * @return fingerprint result with individual entries and collection hash
     */
    public FingerprintResult fingerprintFiles(
        Map<String, FileFingerprintType> files,
        String normalizationStrategy,
        List<String> ignorePatterns
    ) {
        if (client.isNoop()) {
            return new FingerprintResult(Collections.emptyList(), HashCode.fromInt(0), false, "Substrate not available");
        }

        try {
            FingerprintFilesRequest.Builder requestBuilder = FingerprintFilesRequest.newBuilder()
                .setNormalizationStrategy(normalizationStrategy);

            for (Map.Entry<String, FileFingerprintType> entry : files.entrySet()) {
                requestBuilder.addFiles(FileToFingerprint.newBuilder()
                    .setAbsolutePath(entry.getKey())
                    .setType(entry.getValue().toProto())
                    .build());
            }

            if (ignorePatterns != null) {
                requestBuilder.addAllIgnorePatterns(ignorePatterns);
            }

            FingerprintFilesResponse response = client.getFileFingerprintStub()
                .fingerprintFiles(requestBuilder.build());

            if (!response.getSuccess()) {
                LOGGER.warn("[substrate:fingerprint] fingerprinting failed: {}", response.getErrorMessage());
                return new FingerprintResult(Collections.emptyList(), HashCode.fromInt(0),
                    false, response.getErrorMessage());
            }

            List<IndividualFingerprint> entries = new ArrayList<>();
            for (FileFingerprintEntry protoEntry : response.getEntriesList()) {
                entries.add(new IndividualFingerprint(
                    protoEntry.getPath(),
                    HashCode.fromBytes(protoEntry.getHash().toByteArray()),
                    protoEntry.getSize(),
                    protoEntry.getLastModified(),
                    protoEntry.getIsDirectory()
                ));
            }

            HashCode collectionHash = HashCode.fromBytes(response.getCollectionHash().toByteArray());

            LOGGER.debug("[substrate:fingerprint] fingerprinted {} files, collection hash={}",
                entries.size(), collectionHash);

            return new FingerprintResult(entries, collectionHash, true, "");
        } catch (Exception e) {
            LOGGER.debug("[substrate:fingerprint] gRPC call failed", e);
            return new FingerprintResult(Collections.emptyList(), HashCode.fromInt(0),
                false, "gRPC error: " + e.getMessage());
        }
    }

    /**
     * Fingerprint a single file.
     */
    public IndividualFingerprint fingerprintFile(File file) {
        FingerprintResult result = fingerprintFiles(
            Collections.singletonMap(file.getAbsolutePath(), FileFingerprintType.FILE),
            "ABSOLUTE_PATH",
            Collections.emptyList()
        );
        return result.isSuccess() && !result.getEntries().isEmpty()
            ? result.getEntries().get(0)
            : null;
    }

    /**
     * Fingerprint a single directory (recursive merkle hash).
     */
    public IndividualFingerprint fingerprintDirectory(File dir) {
        FingerprintResult result = fingerprintFiles(
            Collections.singletonMap(dir.getAbsolutePath(), FileFingerprintType.DIRECTORY),
            "ABSOLUTE_PATH",
            Collections.emptyList()
        );
        return result.isSuccess() && !result.getEntries().isEmpty()
            ? result.getEntries().get(0)
            : null;
    }

    /**
     * Type of fingerprint to compute.
     */
    public enum FileFingerprintType {
        FILE(FingerprintType.FINGERPRINT_FILE),
        DIRECTORY(FingerprintType.FINGERPRINT_DIRECTORY),
        ROOT(FingerprintType.FINGERPRINT_ROOT);

        private final FingerprintType proto;

        FileFingerprintType(FingerprintType proto) {
            this.proto = proto;
        }

        public FingerprintType toProto() {
            return proto;
        }
    }
}
