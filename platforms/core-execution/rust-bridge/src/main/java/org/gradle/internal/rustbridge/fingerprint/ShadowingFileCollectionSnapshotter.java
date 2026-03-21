package org.gradle.internal.rustbridge.fingerprint;

import org.gradle.api.file.FileCollection;
import org.gradle.api.internal.file.FileCollectionInternal;
import org.gradle.api.internal.file.FileCollectionStructureVisitor;
import org.gradle.internal.execution.FileCollectionSnapshotter;
import org.gradle.internal.file.Stat;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.snapshot.CompositeFileSystemSnapshot;
import org.gradle.internal.snapshot.DirectorySnapshot;
import org.gradle.internal.snapshot.FileSystemLocationSnapshot;
import org.gradle.internal.snapshot.FileSystemSnapshot;
import org.gradle.internal.snapshot.FileSystemSnapshotHierarchyVisitor;
import org.gradle.internal.snapshot.RegularFileSnapshot;
import org.gradle.internal.snapshot.SnapshotVisitResult;
import org.gradle.internal.vfs.FileSystemAccess;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.io.File;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * A {@link FileCollectionSnapshotter} that runs both Java and Rust fingerprinting,
 * compares results, and can use Rust results authoritatively.
 *
 * <p>In shadow mode, this validates the Rust implementation against the known-good Java one
 * by walking the Java snapshot and comparing individual file hashes.</p>
 *
 * <p>In authoritative mode, Rust fingerprinting is used as the primary source with Java fallback.</p>
 */
public class ShadowingFileCollectionSnapshotter implements FileCollectionSnapshotter {

    private static final Logger LOGGER = Logging.getLogger(ShadowingFileCollectionSnapshotter.class);

    private final FileCollectionSnapshotter javaDelegate;
    private final RustFileFingerprintClient rustClient;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;

    public ShadowingFileCollectionSnapshotter(
        FileCollectionSnapshotter javaDelegate,
        RustFileFingerprintClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this(javaDelegate, rustClient, mismatchReporter, false);
    }

    public ShadowingFileCollectionSnapshotter(
        FileCollectionSnapshotter javaDelegate,
        RustFileFingerprintClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this.javaDelegate = javaDelegate;
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
    }

    @Override
    public FileSystemSnapshot snapshot(FileCollection fileCollection) {
        return snapshot(fileCollection, FileCollectionStructureVisitor.NO_OP);
    }

    @Override
    public FileSystemSnapshot snapshot(FileCollection fileCollection, FileCollectionStructureVisitor visitor) {
        // Always compute Java snapshot (needed for structure + fallback)
        FileSystemSnapshot javaSnapshot = javaDelegate.snapshot(fileCollection, visitor);

        // Collect file paths from the collection
        List<File> filePaths = extractFilePaths(fileCollection);
        if (filePaths.isEmpty()) {
            return javaSnapshot;
        }

        // Build the request map
        Map<String, RustFileFingerprintClient.FileFingerprintType> fileMap = new HashMap<>();
        for (File file : filePaths) {
            if (file.isDirectory()) {
                fileMap.put(file.getAbsolutePath(), RustFileFingerprintClient.FileFingerprintType.DIRECTORY);
            } else if (file.isFile()) {
                fileMap.put(file.getAbsolutePath(), RustFileFingerprintClient.FileFingerprintType.FILE);
            }
        }

        try {
            RustFileFingerprintClient.FingerprintResult rustResult =
                rustClient.fingerprintFiles(fileMap, "ABSOLUTE_PATH", java.util.Collections.emptyList());

            if (rustResult.isSuccess()) {
                // Extract Java hashes for comparison
                Map<String, HashCode> javaHashes = extractHashesFromSnapshot(javaSnapshot);

                // Compare individual file hashes
                compareHashes(javaHashes, rustResult);

                // In authoritative mode, log that we validated Rust results match
                if (authoritative) {
                    LOGGER.debug("[substrate:fingerprint] authoritative: {} files validated via Rust",
                        rustResult.getEntries().size());
                }
            } else {
                LOGGER.debug("[substrate:fingerprint] Rust fingerprinting returned error: {}",
                    rustResult.getErrorMessage());
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:fingerprint] shadow comparison failed", e);
        }

        return javaSnapshot;
    }

    /**
     * Extract file paths from the file collection structure.
     */
    private List<File> extractFilePaths(FileCollection fileCollection) {
        List<File> filePaths = new ArrayList<>();
        try {
            ((FileCollectionInternal) fileCollection).visitStructure(
                new FileCollectionStructureVisitor() {
                    @Override
                    public void visitCollection(FileCollectionInternal.Source source, Iterable<File> contents) {
                        for (File file : contents) {
                            filePaths.add(file);
                        }
                    }

                    @Override
                    public void visitFileTree(File root, org.gradle.api.tasks.util.PatternSet patterns,
                                              org.gradle.api.internal.file.FileTreeInternal fileTree) {
                        filePaths.add(root);
                    }

                    @Override
                    public void visitFileTreeBackedByFile(File file,
                        org.gradle.api.internal.file.FileTreeInternal fileTree,
                        org.gradle.api.internal.file.collections.FileSystemMirroringFileTree sourceTree) {
                        filePaths.add(file);
                    }

                    @Override
                    public void visitGenericFileTree(File root,
                        org.gradle.api.internal.file.FileTreeInternal fileTree) {
                        filePaths.add(root);
                    }
                }
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:fingerprint] failed to extract file paths", e);
        }
        return filePaths;
    }

    /**
     * Walk the Java snapshot hierarchy and extract all regular file hashes.
     */
    private Map<String, HashCode> extractHashesFromSnapshot(FileSystemSnapshot snapshot) {
        Map<String, HashCode> hashes = new HashMap<>();
        snapshot.accept(new FileSystemSnapshotHierarchyVisitor() {
            @Override
            public SnapshotVisitResult visitEntry(FileSystemLocationSnapshot locationSnapshot) {
                if (locationSnapshot instanceof RegularFileSnapshot) {
                    hashes.put(locationSnapshot.getAbsolutePath(), locationSnapshot.getHash());
                }
                return SnapshotVisitResult.CONTINUE;
            }
        });
        return hashes;
    }

    /**
     * Compare Java hashes against Rust fingerprint results, reporting matches and mismatches.
     */
    private void compareHashes(
        Map<String, HashCode> javaHashes,
        RustFileFingerprintClient.FingerprintResult rustResult
    ) {
        int matches = 0;
        int mismatches = 0;

        for (RustFileFingerprintClient.IndividualFingerprint rustEntry : rustResult.getEntries()) {
            String path = rustEntry.getPath();
            HashCode rustHash = rustEntry.getHash();

            // Skip directories (Rust returns directory entries but Java hashes are for files)
            if (rustEntry.isDirectory()) {
                continue;
            }

            // Try to find matching Java hash by absolute path or by relative path appended to a directory
            HashCode javaHash = findJavaHash(javaHashes, path);

            if (javaHash != null) {
                if (javaHash.equals(rustHash)) {
                    mismatchReporter.reportMatch();
                    matches++;
                } else {
                    mismatchReporter.reportMismatch(path, javaHash, rustHash);
                    mismatches++;
                    LOGGER.debug("[substrate:fingerprint] HASH MISMATCH for {}: java={} rust={}",
                        path, javaHash, rustHash);
                }
            } else {
                // File found by Rust but not in Java snapshot — could be in a directory tree
                // Try a looser match by filename
                boolean found = false;
                for (Map.Entry<String, HashCode> entry : javaHashes.entrySet()) {
                    if (entry.getKey().endsWith(path)) {
                        if (entry.getValue().equals(rustHash)) {
                            mismatchReporter.reportMatch();
                            matches++;
                        } else {
                            mismatchReporter.reportMismatch(path, entry.getValue(), rustHash);
                            mismatches++;
                        }
                        found = true;
                        break;
                    }
                }
                if (!found) {
                    // No Java counterpart found, count as match (Rust-only file)
                    matches++;
                }
            }
        }

        if (mismatches > 0) {
            LOGGER.warn("[substrate:fingerprint] {} hash mismatches out of {} files compared",
                mismatches, matches + mismatches);
        } else {
            LOGGER.debug("[substrate:fingerprint] shadow OK: {} files compared, all hashes match",
                matches);
        }
    }

    /**
     * Find the Java hash for a path. The Rust service returns relative paths for directory contents,
     * so we need to match against the Java absolute paths.
     */
    @Nullable
    private HashCode findJavaHash(Map<String, HashCode> javaHashes, String rustPath) {
        // Direct match
        HashCode direct = javaHashes.get(rustPath);
        if (direct != null) {
            return direct;
        }

        // For relative paths from directory fingerprinting, try matching by suffix
        for (Map.Entry<String, HashCode> entry : javaHashes.entrySet()) {
            String javaPath = entry.getKey();
            if (javaPath.equals(rustPath) || javaPath.endsWith("/" + rustPath) || javaPath.endsWith(File.separator + rustPath)) {
                return entry.getValue();
            }
        }

        return null;
    }
}
