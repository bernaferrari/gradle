package org.gradle.internal.rustbridge.classpath;

import gradle.substrate.v1.*;
import org.gradle.api.logging.Logging;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * Client for the Rust classpath hashing service.
 * Computes composite hashes of classpath entries (JARs and directories)
 * and detects classpath changes between builds.
 */
public class RustClasspathClient {

    private static final Logger LOGGER = Logging.getLogger(RustClasspathClient.class);

    private final SubstrateClient client;

    public RustClasspathClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of hashing a classpath.
     */
    public static class ClasspathHashResult {
        private final HashCode classpathHash;
        private final List<EntryHash> entries;
        private final String algorithm;
        private final boolean success;
        private final String errorMessage;

        private ClasspathHashResult(HashCode classpathHash, List<EntryHash> entries,
                                    String algorithm, boolean success, String errorMessage) {
            this.classpathHash = classpathHash;
            this.entries = entries;
            this.algorithm = algorithm;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public HashCode getClasspathHash() { return classpathHash; }
        public List<EntryHash> getEntries() { return entries; }
        public String getAlgorithm() { return algorithm; }
        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }

        public static ClasspathHashResult empty() {
            return new ClasspathHashResult(
                HashCode.fromBytes(new byte[0]),
                Collections.emptyList(), "", false, "Substrate not available"
            );
        }

        public static ClasspathHashResult failure(String message) {
            return new ClasspathHashResult(
                HashCode.fromBytes(new byte[0]),
                Collections.emptyList(), "", false, message
            );
        }
    }

    /**
     * Hash of an individual classpath entry.
     */
    public static class EntryHash {
        private final String absolutePath;
        private final HashCode hash;
        private final long size;

        private EntryHash(String absolutePath, HashCode hash, long size) {
            this.absolutePath = absolutePath;
            this.hash = hash;
            this.size = size;
        }

        public String getAbsolutePath() { return absolutePath; }
        public HashCode getHash() { return hash; }
        public long getSize() { return size; }
    }

    /**
     * Result of comparing two classpaths.
     */
    public static class ComparisonResult {
        private final boolean changed;
        private final HashCode newHash;
        private final List<ClasspathDifference> differences;
        private final boolean success;
        private final String errorMessage;

        private ComparisonResult(boolean changed, HashCode newHash,
                                 List<ClasspathDifference> differences,
                                 boolean success, String errorMessage) {
            this.changed = changed;
            this.newHash = newHash;
            this.differences = differences;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public boolean isChanged() { return changed; }
        public HashCode getNewHash() { return newHash; }
        public List<ClasspathDifference> getDifferences() { return differences; }
        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }

        public static ComparisonResult failure(String message) {
            return new ComparisonResult(false, HashCode.fromBytes(new byte[0]),
                Collections.emptyList(), false, message);
        }
    }

    /**
     * Describes a single difference between two classpaths.
     */
    public static class ClasspathDifference {
        private final String changeType;
        private final String absolutePath;

        private ClasspathDifference(String changeType, String absolutePath) {
            this.changeType = changeType;
            this.absolutePath = absolutePath;
        }

        public String getChangeType() { return changeType; }
        public String getAbsolutePath() { return absolutePath; }
    }

    /**
     * Type of classpath entry.
     */
    public enum EntryType {
        JAR(ClasspathEntryType.JAR),
        DIRECTORY(ClasspathEntryType.DIRECTORY);

        private final ClasspathEntryType proto;

        EntryType(ClasspathEntryType proto) {
            this.proto = proto;
        }

        public ClasspathEntryType toProto() { return proto; }
    }

    /**
     * Hashes a classpath with the default algorithm (MD5).
     *
     * @param entries list of classpath entries with metadata
     * @return composite hash and per-entry hashes
     */
    public ClasspathHashResult hashClasspath(List<ClasspathEntryDescriptor> entries) {
        return hashClasspath(entries, "MD5", false, false);
    }

    /**
     * Hashes a classpath with configurable options.
     *
     * @param entries list of classpath entries with metadata
     * @param algorithm hash algorithm ("MD5", "SHA-256", "BLAKE3")
     * @param ignoreTimestamps if true, hash content only (ignore mtime)
     * @param includeEntryHashes if true, return per-entry hashes
     * @return composite hash and optional per-entry hashes
     */
    public ClasspathHashResult hashClasspath(List<ClasspathEntryDescriptor> entries,
                                              String algorithm,
                                              boolean ignoreTimestamps,
                                              boolean includeEntryHashes) {
        if (client.isNoop()) {
            return ClasspathHashResult.empty();
        }

        try {
            HashClasspathRequest.Builder requestBuilder = HashClasspathRequest.newBuilder()
                .setAlgorithm(algorithm)
                .setIgnoreTimestamps(ignoreTimestamps)
                .setIncludeEntryHashes(includeEntryHashes);

            for (ClasspathEntryDescriptor entry : entries) {
                requestBuilder.addEntries(ClasspathEntry.newBuilder()
                    .setAbsolutePath(entry.absolutePath)
                    .setEntryType(entry.entryType.toProto())
                    .setLength(entry.length)
                    .setLastModified(entry.lastModified)
                    .build());
            }

            HashClasspathResponse response = client.getClasspathStub()
                .hashClasspath(requestBuilder.build());

            HashCode classpathHash = HashCode.fromBytes(response.getClasspathHash().toByteArray());

            List<EntryHash> entryHashes = new ArrayList<>();
            for (ClasspathEntryHash protoEntry : response.getEntriesList()) {
                entryHashes.add(new EntryHash(
                    protoEntry.getAbsolutePath(),
                    HashCode.fromBytes(protoEntry.getHash().toByteArray()),
                    protoEntry.getSize()
                ));
            }

            LOGGER.debug("[substrate:classpath] hashed {} entries, algorithm={}",
                entries.size(), response.getAlgorithmUsed());

            return new ClasspathHashResult(classpathHash, entryHashes,
                response.getAlgorithmUsed(), true, "");
        } catch (Exception e) {
            LOGGER.debug("[substrate:classpath] gRPC call failed", e);
            return ClasspathHashResult.failure("gRPC error: " + e.getMessage());
        }
    }

    /**
     * Compares a previous classpath hash against current entries.
     *
     * @param previousHash the hash from a previous classpath computation
     * @param currentEntries the current classpath entries
     * @param algorithm hash algorithm (must match what was used for previousHash)
     * @return comparison result indicating changes
     */
    public ComparisonResult compareClasspaths(HashCode previousHash,
                                               List<ClasspathEntryDescriptor> currentEntries,
                                               String algorithm) {
        if (client.isNoop()) {
            return ComparisonResult.failure("Substrate not available");
        }

        try {
            CompareClasspathsRequest.Builder requestBuilder = CompareClasspathsRequest.newBuilder()
                .setPreviousHash(com.google.protobuf.ByteString.copyFrom(previousHash.toByteArray()))
                .setAlgorithm(algorithm);

            for (ClasspathEntryDescriptor entry : currentEntries) {
                requestBuilder.addCurrentEntries(ClasspathEntry.newBuilder()
                    .setAbsolutePath(entry.absolutePath)
                    .setEntryType(entry.entryType.toProto())
                    .setLength(entry.length)
                    .setLastModified(entry.lastModified)
                    .build());
            }

            CompareClasspathsResponse response = client.getClasspathStub()
                .compareClasspaths(requestBuilder.build());

            List<ClasspathDifference> differences = new ArrayList<>();
            for (gradle.substrate.v1.ClasspathDifference diff : response.getDifferencesList()) {
                differences.add(new ClasspathDifference(diff.getChangeType(), diff.getAbsolutePath()));
            }

            HashCode newHash = HashCode.fromBytes(response.getNewHash().toByteArray());

            return new ComparisonResult(response.getChanged(), newHash, differences, true, "");
        } catch (Exception e) {
            LOGGER.debug("[substrate:classpath] gRPC call failed", e);
            return ComparisonResult.failure("gRPC error: " + e.getMessage());
        }
    }

    /**
     * Descriptor for a classpath entry (JAR or directory).
     */
    public static class ClasspathEntryDescriptor {
        final String absolutePath;
        final EntryType entryType;
        final long length;
        final long lastModified;

        public ClasspathEntryDescriptor(String absolutePath, EntryType entryType,
                                        long length, long lastModified) {
            this.absolutePath = absolutePath;
            this.entryType = entryType;
            this.length = length;
            this.lastModified = lastModified;
        }
    }
}
