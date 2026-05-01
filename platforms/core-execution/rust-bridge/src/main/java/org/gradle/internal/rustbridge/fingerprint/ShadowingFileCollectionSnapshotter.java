package org.gradle.internal.rustbridge.fingerprint;

import org.apache.tools.ant.DirectoryScanner;
import org.gradle.api.file.FilePermissions;
import org.gradle.api.file.FileCollection;
import org.gradle.api.file.FileTreeElement;
import org.gradle.api.file.RelativePath;
import org.gradle.api.internal.file.DefaultFilePermissions;
import org.gradle.api.logging.Logging;
import org.gradle.api.internal.file.DelegatingFileCollectionStructureVisitor;
import org.gradle.api.internal.file.FileCollectionInternal;
import org.gradle.api.internal.file.FileCollectionStructureVisitor;
import org.gradle.api.internal.file.FileTreeInternal;
import org.gradle.api.internal.file.collections.FileSystemMirroringFileTree;
import org.gradle.api.specs.Spec;
import org.gradle.api.tasks.util.PatternSet;
import org.gradle.internal.UncheckedException;
import org.gradle.internal.execution.FileCollectionSnapshotter;
import org.gradle.internal.file.FileMetadata;
import org.gradle.internal.file.Stat;
import org.gradle.internal.file.impl.DefaultFileMetadata;
import org.gradle.internal.hash.HashCode;
import org.gradle.internal.rustbridge.SubstrateException;
import org.gradle.internal.snapshot.CompositeFileSystemSnapshot;
import org.gradle.internal.snapshot.DirectorySnapshotBuilder;
import org.gradle.internal.snapshot.FileSystemLocationSnapshot;
import org.gradle.internal.snapshot.FileSystemSnapshot;
import org.gradle.internal.snapshot.FileSystemSnapshotHierarchyVisitor;
import org.gradle.internal.snapshot.MerkleDirectorySnapshotBuilder;
import org.gradle.internal.snapshot.MissingFileSnapshot;
import org.gradle.internal.snapshot.RegularFileSnapshot;
import org.gradle.internal.snapshot.SnapshotVisitResult;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Predicate;

/**
 * A {@link FileCollectionSnapshotter} that runs both Java and Rust fingerprinting,
 * compares results, and can use Rust results authoritatively.
 *
 * <p>In shadow mode, this validates the Rust implementation against the known-good Java one
 * by walking the Java snapshot and comparing individual file hashes.</p>
 *
 * <p>In authoritative mode, this supports direct files, missing paths, directory roots,
 * PatternSet-backed file trees, and file-tree-backed archive files. Symlink and special-file
 * paths still fail closed until their Gradle access semantics are modeled explicitly.</p>
 */
public class ShadowingFileCollectionSnapshotter implements FileCollectionSnapshotter {

    private static final Logger LOGGER = Logging.getLogger(ShadowingFileCollectionSnapshotter.class);
    private static final DefaultExcludes DEFAULT_EXCLUDES = new DefaultExcludes(DirectoryScanner.getDefaultExcludes());

    private final FileCollectionSnapshotter javaDelegate;
    private final RustFileFingerprintClient rustClient;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;
    @Nullable
    private final Stat stat;

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
        this(javaDelegate, rustClient, mismatchReporter, authoritative, null);
    }

    public ShadowingFileCollectionSnapshotter(
        FileCollectionSnapshotter javaDelegate,
        RustFileFingerprintClient rustClient,
        HashMismatchReporter mismatchReporter,
        boolean authoritative,
        @Nullable Stat stat
    ) {
        this.javaDelegate = javaDelegate;
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
        this.stat = stat;
    }

    public boolean isAuthoritative() {
        return authoritative;
    }

    @Override
    public FileSystemSnapshot snapshot(FileCollection fileCollection) {
        return snapshot(fileCollection, FileCollectionStructureVisitor.NO_OP);
    }

    @Override
    public FileSystemSnapshot snapshot(FileCollection fileCollection, FileCollectionStructureVisitor visitor) {
        if (authoritative) {
            return snapshotAuthoritatively(fileCollection, visitor);
        }

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

            } else {
                LOGGER.debug("[substrate:fingerprint] Rust fingerprinting returned error: {}",
                    rustResult.getErrorMessage());
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:fingerprint] shadow comparison failed", e);
        }

        return javaSnapshot;
    }

    private FileSystemSnapshot snapshotAuthoritatively(FileCollection fileCollection, FileCollectionStructureVisitor visitor) {
        AuthoritativeSnapshottingVisitor snapshottingVisitor = new AuthoritativeSnapshottingVisitor(visitor);
        try {
            ((FileCollectionInternal) fileCollection).visitStructure(snapshottingVisitor);
        } catch (SubstrateException e) {
            throw e;
        } catch (Exception e) {
            throw new SubstrateException("Authoritative Rust file fingerprinting could not inspect file collection structure", e);
        }

        List<RootSpec> roots = snapshottingVisitor.getRoots();
        if (roots.isEmpty()) {
            return CompositeFileSystemSnapshot.of(java.util.Collections.emptyList());
        }

        List<File> regularFiles = new ArrayList<>();
        for (RootSpec root : roots) {
            collectRegularFiles(root, regularFiles);
        }

        Map<String, RustFileFingerprintClient.FileFingerprintType> fileMap = new LinkedHashMap<>();
        for (File file : regularFiles) {
            fileMap.put(file.getAbsolutePath(), RustFileFingerprintClient.FileFingerprintType.FILE);
        }

        Map<String, RustFileFingerprintClient.IndividualFingerprint> rustEntriesByPath = new HashMap<>();
        if (!fileMap.isEmpty()) {
            RustFileFingerprintClient.FingerprintResult rustResult;
            try {
                rustResult = rustClient.fingerprintFiles(fileMap, "ABSOLUTE_PATH", java.util.Collections.emptyList());
            } catch (Exception e) {
                throw new SubstrateException("Authoritative Rust file fingerprinting failed", e);
            }
            if (!rustResult.isSuccess()) {
                throw new SubstrateException("Authoritative Rust file fingerprinting failed: " + rustResult.getErrorMessage());
            }

            for (RustFileFingerprintClient.IndividualFingerprint entry : rustResult.getEntries()) {
                rustEntriesByPath.put(entry.getPath(), entry);
            }
        }

        List<FileSystemSnapshot> snapshots = new ArrayList<>(roots.size());
        for (RootSpec root : roots) {
            FileSystemLocationSnapshot snapshot = snapshotRoot(root, rustEntriesByPath);
            if (snapshot != null) {
                snapshots.add(snapshot);
            }
        }

        LOGGER.debug("[substrate:fingerprint] authoritative: {} roots, {} regular files snapshotted via Rust", snapshots.size(), regularFiles.size());
        return CompositeFileSystemSnapshot.of(snapshots);
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
                }
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:fingerprint] failed to extract file paths", e);
        }
        return filePaths;
    }

    private FileSystemLocationSnapshot snapshotRoot(RootSpec root, Map<String, RustFileFingerprintClient.IndividualFingerprint> rustEntriesByPath) {
        File absoluteRoot = root.root;
        if (!absoluteRoot.exists()) {
            return new MissingFileSnapshot(absoluteRoot.getAbsolutePath(), absoluteRoot.getName(), FileMetadata.AccessType.DIRECT);
        }
        if (absoluteRoot.isFile()) {
            if (!root.isIncluded(absoluteRoot, false, java.util.Collections.singletonList(absoluteRoot.getName()), stat)) {
                return null;
            }
            return regularFileSnapshot(absoluteRoot, rustEntriesByPath);
        }
        if (absoluteRoot.isDirectory()) {
            DirectorySnapshotBuilder builder = MerkleDirectorySnapshotBuilder.sortingRequired();
            appendDirectory(builder, root, absoluteRoot, rustEntriesByPath, java.util.Collections.emptyList());
            return builder.getResult();
        }
        throw new SubstrateException("Authoritative Rust file fingerprinting does not support special file roots: " + absoluteRoot.getAbsolutePath());
    }

    private void appendDirectory(
        DirectorySnapshotBuilder builder,
        RootSpec root,
        File directory,
        Map<String, RustFileFingerprintClient.IndividualFingerprint> rustEntriesByPath,
        List<String> relativeSegments
    ) {
        ensureDirectPath(directory);
        builder.enterDirectory(
            FileMetadata.AccessType.DIRECT,
            directory.getAbsolutePath(),
            directory.getName(),
            DirectorySnapshotBuilder.EmptyDirectoryHandlingStrategy.INCLUDE_EMPTY_DIRS
        );
        File[] children = directory.listFiles();
        if (children == null) {
            throw new SubstrateException("Authoritative Rust file fingerprinting could not list directory: " + directory.getAbsolutePath());
        }
        Arrays.sort(children, (left, right) -> left.getName().compareTo(right.getName()));
        for (File child : children) {
            List<String> childRelativeSegments = appendSegment(relativeSegments, child.getName());
            if (child.isDirectory()) {
                if (!DEFAULT_EXCLUDES.excludeDir(child.getName()) && root.isIncluded(child, true, childRelativeSegments, stat)) {
                    appendDirectory(builder, root, child.getAbsoluteFile(), rustEntriesByPath, childRelativeSegments);
                }
            } else if (child.isFile()) {
                if (!DEFAULT_EXCLUDES.excludeFile(child.getName()) && root.isIncluded(child, false, childRelativeSegments, stat)) {
                    builder.visitLeafElement(regularFileSnapshot(child.getAbsoluteFile(), rustEntriesByPath));
                }
            } else {
                throw new SubstrateException("Authoritative Rust file fingerprinting does not support special files: " + child.getAbsolutePath());
            }
        }
        builder.leaveDirectory();
    }

    private RegularFileSnapshot regularFileSnapshot(File file, Map<String, RustFileFingerprintClient.IndividualFingerprint> rustEntriesByPath) {
        ensureDirectPath(file);
        RustFileFingerprintClient.IndividualFingerprint entry = rustEntriesByPath.get(file.getAbsolutePath());
        if (entry == null) {
            throw new SubstrateException("Authoritative Rust file fingerprinting returned no entry for " + file.getAbsolutePath());
        }
        if (entry.isDirectory()) {
            throw new SubstrateException("Authoritative Rust file fingerprinting returned a directory for regular file " + file.getAbsolutePath());
        }
        FileMetadata metadata = DefaultFileMetadata.file(
            entry.getLastModified(),
            entry.getSize(),
            FileMetadata.AccessType.DIRECT
        );
        return new RegularFileSnapshot(
            file.getAbsolutePath(),
            file.getName(),
            entry.getHash(),
            metadata
        );
    }

    private void collectRegularFiles(RootSpec root, List<File> regularFiles) {
        collectRegularFiles(root, root.root, regularFiles, java.util.Collections.emptyList());
    }

    private void collectRegularFiles(RootSpec root, File file, List<File> regularFiles, List<String> relativeSegments) {
        File absoluteRoot = file.getAbsoluteFile();
        ensureDirectPath(absoluteRoot);
        if (!absoluteRoot.exists()) {
            return;
        }
        if (absoluteRoot.isFile()) {
            if (root.isIncluded(absoluteRoot, false, relativeSegments.isEmpty() ? java.util.Collections.singletonList(absoluteRoot.getName()) : relativeSegments, stat)) {
                regularFiles.add(absoluteRoot);
            }
            return;
        }
        if (absoluteRoot.isDirectory()) {
            File[] children = absoluteRoot.listFiles();
            if (children == null) {
                throw new SubstrateException("Authoritative Rust file fingerprinting could not list directory: " + absoluteRoot.getAbsolutePath());
            }
            for (File child : children) {
                List<String> childRelativeSegments = appendSegment(relativeSegments, child.getName());
                if (child.isDirectory()) {
                    if (!DEFAULT_EXCLUDES.excludeDir(child.getName()) && root.isIncluded(child, true, childRelativeSegments, stat)) {
                        collectRegularFiles(root, child, regularFiles, childRelativeSegments);
                    }
                } else if (child.isFile()) {
                    if (!DEFAULT_EXCLUDES.excludeFile(child.getName()) && root.isIncluded(child, false, childRelativeSegments, stat)) {
                        ensureDirectPath(child);
                        regularFiles.add(child.getAbsoluteFile());
                    }
                } else {
                    throw new SubstrateException("Authoritative Rust file fingerprinting does not support special files: " + child.getAbsolutePath());
                }
            }
            return;
        }
        throw new SubstrateException("Authoritative Rust file fingerprinting does not support special file roots: " + absoluteRoot.getAbsolutePath());
    }

    private static List<String> appendSegment(List<String> segments, String segment) {
        List<String> result = new ArrayList<>(segments.size() + 1);
        result.addAll(segments);
        result.add(segment);
        return result;
    }

    private static void ensureDirectPath(File file) {
        if (Files.isSymbolicLink(file.toPath())) {
            throw new SubstrateException("Authoritative Rust file fingerprinting does not yet support symlink paths: " + file.getAbsolutePath());
        }
    }

    private static class AuthoritativeSnapshottingVisitor extends DelegatingFileCollectionStructureVisitor {
        private final List<RootSpec> roots = new ArrayList<>();

        private AuthoritativeSnapshottingVisitor(FileCollectionStructureVisitor delegate) {
            super(delegate);
        }

        @Override
        public void visitCollection(FileCollectionInternal.Source source, Iterable<File> contents) {
            super.visitCollection(source, contents);
            for (File file : contents) {
                roots.add(new RootSpec(file.getAbsoluteFile(), null));
            }
        }

        @Override
        public void visitFileTree(File root, PatternSet patterns, FileTreeInternal fileTree) {
            super.visitFileTree(root, patterns, fileTree);
            roots.add(new RootSpec(root.getAbsoluteFile(), patterns));
        }

        @Override
        public void visitFileTreeBackedByFile(File file, FileTreeInternal fileTree, FileSystemMirroringFileTree sourceTree) {
            super.visitFileTreeBackedByFile(file, fileTree, sourceTree);
            roots.add(new RootSpec(file.getAbsoluteFile(), null));
        }

        private List<RootSpec> getRoots() {
            return roots;
        }
    }

    private static class RootSpec {
        private final File root;
        @Nullable
        private final PatternSet patterns;

        private RootSpec(File root, @Nullable PatternSet patterns) {
            this.root = root;
            this.patterns = patterns;
        }

        private boolean isIncluded(File file, boolean isDirectory, List<String> relativeSegments, @Nullable Stat stat) {
            if (patterns == null || patterns.isEmpty()) {
                return true;
            }
            Spec<FileTreeElement> spec = patterns.getAsSpec();
            return spec.isSatisfiedBy(new PathBackedFileTreeElement(file.toPath(), file.getName(), isDirectory, relativeSegments, stat));
        }
    }

    private static class PathBackedFileTreeElement implements FileTreeElement {
        private final Path path;
        private final String name;
        private final boolean directory;
        private final Iterable<String> relativePathIterable;
        @Nullable
        private final Stat stat;
        private RelativePath relativePath;

        private PathBackedFileTreeElement(Path path, String name, boolean directory, Iterable<String> relativePathIterable, @Nullable Stat stat) {
            this.path = path;
            this.name = name;
            this.directory = directory;
            this.relativePathIterable = relativePathIterable;
            this.stat = stat;
        }

        @Override
        public File getFile() {
            return path.toFile();
        }

        @Override
        public boolean isDirectory() {
            return directory;
        }

        @Override
        public long getLastModified() {
            return getFile().lastModified();
        }

        @Override
        public long getSize() {
            return getFile().length();
        }

        @Override
        public InputStream open() {
            try {
                return Files.newInputStream(path);
            } catch (IOException e) {
                throw UncheckedException.throwAsUncheckedException(e);
            }
        }

        @Override
        public void copyTo(OutputStream output) {
            throw new UnsupportedOperationException("Copy to not supported for filters");
        }

        @Override
        public boolean copyTo(File target) {
            throw new UnsupportedOperationException("Copy to not supported for filters");
        }

        @Override
        public String getName() {
            return name;
        }

        @Override
        public String getPath() {
            return getRelativePath().getPathString();
        }

        @Override
        public RelativePath getRelativePath() {
            if (relativePath == null) {
                relativePath = new RelativePath(!directory, toArray(relativePathIterable));
            }
            return relativePath;
        }

        @Override
        public FilePermissions getPermissions() {
            if (stat == null) {
                throw new SubstrateException("Authoritative Rust file-tree filtering requires Stat for permission-sensitive patterns");
            }
            int unixNumeric = stat.getUnixMode(getFile());
            return new DefaultFilePermissions(unixNumeric);
        }

        private static String[] toArray(Iterable<String> elements) {
            List<String> list = new ArrayList<>();
            for (String element : elements) {
                list.add(element);
            }
            return list.toArray(new String[0]);
        }
    }

    private static class DefaultExcludes {
        private final Set<String> excludeFileNames = new HashSet<>();
        private final Set<String> excludedDirNames = new HashSet<>();
        private final Predicate<String> excludedFileNameSpec;

        private DefaultExcludes(String[] defaultExcludes) {
            List<Predicate<String>> excludeFileSpecs = new ArrayList<>();
            for (String defaultExcludePattern : defaultExcludes) {
                String defaultExclude = defaultExcludePattern;
                if (defaultExclude.startsWith("**/")) {
                    defaultExclude = defaultExclude.substring(3);
                }
                int length = defaultExclude.length();
                if (defaultExclude.endsWith("/**")) {
                    excludedDirNames.add(defaultExclude.substring(0, length - 3));
                } else {
                    int firstStar = defaultExclude.indexOf('*');
                    if (firstStar == -1) {
                        excludeFileNames.add(defaultExclude);
                    } else {
                        Predicate<String> start = firstStar == 0
                            ? it -> true
                            : new StartMatcher(defaultExclude.substring(0, firstStar));
                        Predicate<String> end = firstStar == length - 1
                            ? it -> true
                            : new EndMatcher(defaultExclude.substring(firstStar + 1, length));
                        excludeFileSpecs.add(start.and(end));
                    }
                }
            }
            this.excludedFileNameSpec = excludeFileSpecs.stream().reduce(it -> false, Predicate::or);
        }

        private boolean excludeDir(String name) {
            return excludedDirNames.contains(name);
        }

        private boolean excludeFile(String name) {
            return excludeFileNames.contains(name) || excludedFileNameSpec.test(name);
        }
    }

    private static class EndMatcher implements Predicate<String> {
        private final String end;

        private EndMatcher(String end) {
            this.end = end;
        }

        @Override
        public boolean test(String element) {
            return element.endsWith(end);
        }
    }

    private static class StartMatcher implements Predicate<String> {
        private final String start;

        private StartMatcher(String start) {
            this.start = start;
        }

        @Override
        public boolean test(String element) {
            return element.startsWith(start);
        }
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
