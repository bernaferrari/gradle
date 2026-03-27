package org.gradle.internal.rustbridge.filetree;

import gradle.substrate.v1.*;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * Client for the Rust file tree service.
 * Provides Ant-style pattern matching and filesystem traversal
 * compatible with Gradle's file tree operations.
 */
public class RustFileTreeClient {

    private static final Logger LOGGER = Logging.getLogger(RustFileTreeClient.class);

    private final SubstrateClient client;

    public RustFileTreeClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of a file tree traversal.
     */
    public static class TraverseResult {
        private final List<FileEntry> entries;
        private final int totalEntries;
        private final long totalSize;
        private final boolean success;
        private final String errorMessage;

        private TraverseResult(List<FileEntry> entries, int totalEntries, long totalSize,
                               boolean success, String errorMessage) {
            this.entries = entries;
            this.totalEntries = totalEntries;
            this.totalSize = totalSize;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public List<FileEntry> getEntries() { return entries; }
        public int getTotalEntries() { return totalEntries; }
        public long getTotalSize() { return totalSize; }
        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }

        public static TraverseResult empty() {
            return new TraverseResult(Collections.emptyList(), 0, 0, false, "Substrate not available");
        }

        public static TraverseResult failure(String message) {
            return new TraverseResult(Collections.emptyList(), 0, 0, false, message);
        }
    }

    /**
     * A single entry in the file tree.
     */
    public static class FileEntry {
        private final String relativePath;
        private final String absolutePath;
        private final boolean isDirectory;
        private final long size;
        private final long lastModifiedMs;

        private FileEntry(String relativePath, String absolutePath, boolean isDirectory,
                          long size, long lastModifiedMs) {
            this.relativePath = relativePath;
            this.absolutePath = absolutePath;
            this.isDirectory = isDirectory;
            this.size = size;
            this.lastModifiedMs = lastModifiedMs;
        }

        public String getRelativePath() { return relativePath; }
        public String getAbsolutePath() { return absolutePath; }
        public boolean isDirectory() { return isDirectory; }
        public long getSize() { return size; }
        public long getLastModifiedMs() { return lastModifiedMs; }
    }

    /**
     * Result of pattern matching against a set of paths.
     */
    public static class MatchResult {
        private final List<PatternMatch> results;
        private final boolean success;
        private final String errorMessage;

        private MatchResult(List<PatternMatch> results, boolean success, String errorMessage) {
            this.results = results;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public List<PatternMatch> getResults() { return results; }
        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }

        public static MatchResult failure(String message) {
            return new MatchResult(Collections.emptyList(), false, message);
        }
    }

    /**
     * Result of matching a single path against patterns.
     */
    public static class PatternMatch {
        private final String path;
        private final boolean included;

        private PatternMatch(String path, boolean included) {
            this.path = path;
            this.included = included;
        }

        public String getPath() { return path; }
        public boolean isIncluded() { return included; }
    }

    /**
     * Traverses a directory tree with include/exclude patterns.
     *
     * @param rootDir root directory to traverse
     * @param includePatterns Ant-style include patterns (e.g. "**/*.java")
     * @param excludePatterns Ant-style exclude patterns (e.g. "**/test/**")
     * @param includeFiles whether to include files in the result
     * @param includeDirs whether to include directories in the result
     * @param followSymlinks whether to follow symbolic links
     * @param maxDepth maximum traversal depth (0 = unlimited)
     * @param includeMetadata whether to include size/modified metadata
     * @param applyDefaultExcludes whether to apply Gradle default excludes (.git, .DS_Store, etc.)
     * @return traversal result with matched entries
     */
    public TraverseResult traverseFileTree(String rootDir,
                                            List<String> includePatterns,
                                            List<String> excludePatterns,
                                            boolean includeFiles,
                                            boolean includeDirs,
                                            boolean followSymlinks,
                                            int maxDepth,
                                            boolean includeMetadata,
                                            boolean applyDefaultExcludes) {
        if (client.isNoop()) {
            return TraverseResult.empty();
        }

        try {
            TraverseFileTreeRequest.Builder requestBuilder = TraverseFileTreeRequest.newBuilder()
                .setRootDir(rootDir)
                .setIncludeFiles(includeFiles)
                .setIncludeDirs(includeDirs)
                .setFollowSymlinks(followSymlinks)
                .setMaxDepth(maxDepth)
                .setIncludeMetadata(includeMetadata)
                .setApplyDefaultExcludes(applyDefaultExcludes);

            if (includePatterns != null) {
                requestBuilder.addAllIncludePatterns(includePatterns);
            }
            if (excludePatterns != null) {
                requestBuilder.addAllExcludePatterns(excludePatterns);
            }

            TraverseFileTreeResponse response = client.getFileTreeStub()
                .traverseFileTree(requestBuilder.build());

            if (!response.getErrorMessage().isEmpty()) {
                LOGGER.warn("[substrate:filetree] traversal failed: {}", response.getErrorMessage());
                return TraverseResult.failure(response.getErrorMessage());
            }

            List<FileEntry> entries = new ArrayList<>();
            for (FileTreeEntry protoEntry : response.getEntriesList()) {
                entries.add(new FileEntry(
                    protoEntry.getRelativePath(),
                    protoEntry.getAbsolutePath(),
                    protoEntry.getIsDirectory(),
                    protoEntry.getSize(),
                    protoEntry.getLastModifiedMs()
                ));
            }

            LOGGER.debug("[substrate:filetree] traversed {}, {} entries, {} bytes",
                rootDir, response.getTotalEntries(), response.getTotalSize());

            return new TraverseResult(entries, response.getTotalEntries(),
                response.getTotalSize(), true, "");
        } catch (Exception e) {
            LOGGER.debug("[substrate:filetree] gRPC call failed", e);
            return TraverseResult.failure("gRPC error: " + e.getMessage());
        }
    }

    /**
     * Traverses a directory tree with default settings (files only, no metadata, no symlinks).
     *
     * @param rootDir root directory to traverse
     * @param includePatterns Ant-style include patterns
     * @return traversal result
     */
    public TraverseResult traverseFileTree(String rootDir, List<String> includePatterns) {
        return traverseFileTree(rootDir, includePatterns, Collections.emptyList(),
            true, false, false, 0, false, true);
    }

    /**
     * Matches paths against include/exclude patterns without filesystem access.
     *
     * @param paths paths to test
     * @param includePatterns Ant-style include patterns
     * @param excludePatterns Ant-style exclude patterns
     * @return match results for each path
     */
    public MatchResult matchPatterns(List<String> paths,
                                      List<String> includePatterns,
                                      List<String> excludePatterns) {
        if (client.isNoop()) {
            return MatchResult.failure("Substrate not available");
        }

        try {
            MatchPatternsRequest.Builder requestBuilder = MatchPatternsRequest.newBuilder();

            if (paths != null) {
                requestBuilder.addAllPaths(paths);
            }
            if (includePatterns != null) {
                requestBuilder.addAllIncludePatterns(includePatterns);
            }
            if (excludePatterns != null) {
                requestBuilder.addAllExcludePatterns(excludePatterns);
            }

            MatchPatternsResponse response = client.getFileTreeStub()
                .matchPatterns(requestBuilder.build());

            List<PatternMatch> results = new ArrayList<>();
            for (PatternMatchResult protoResult : response.getResultsList()) {
                results.add(new PatternMatch(protoResult.getPath(), protoResult.getIncluded()));
            }

            return new MatchResult(results, true, "");
        } catch (Exception e) {
            LOGGER.debug("[substrate:filetree] gRPC call failed", e);
            return MatchResult.failure("gRPC error: " + e.getMessage());
        }
    }
}
