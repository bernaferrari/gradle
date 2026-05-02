package org.gradle.internal.rustbridge.dependency;

import gradle.substrate.v1.AddArtifactToCacheRequest;
import gradle.substrate.v1.AddArtifactToCacheResponse;
import gradle.substrate.v1.CheckArtifactCacheRequest;
import gradle.substrate.v1.CheckArtifactCacheResponse;
import gradle.substrate.v1.CheckMetadataCacheRequest;
import gradle.substrate.v1.CheckMetadataCacheResponse;
import gradle.substrate.v1.DependencyDescriptor;
import gradle.substrate.v1.DependencyResolutionServiceGrpc;
import gradle.substrate.v1.DownloadArtifactRequest;
import gradle.substrate.v1.DownloadArtifactChunk;
import gradle.substrate.v1.GetResolutionStatsRequest;
import gradle.substrate.v1.GetResolutionStatsResponse;
import gradle.substrate.v1.RecordResolutionRequest;
import gradle.substrate.v1.RecordResolutionResponse;
import gradle.substrate.v1.RepositoryDescriptor;
import gradle.substrate.v1.ResolveDependenciesRequest;
import gradle.substrate.v1.ResolveDependenciesResponse;
import gradle.substrate.v1.ResolvedDependency;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;
import org.slf4j.Logger;

import java.io.BufferedOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.net.URI;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;
import java.util.Locale;

/**
 * Client for the Rust dependency resolution service.
 * Resolves dependency graphs, checks artifact caches, and downloads artifacts via gRPC.
 */
@ServiceScope(Scope.Build.class)
public class RustDependencyResolutionClient {

    private static final Logger LOGGER = Logging.getLogger(RustDependencyResolutionClient.class);

    private final SubstrateClient client;

    public RustDependencyResolutionClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of resolving a dependency graph.
     */
    public static class ResolutionResult {
        private final boolean success;
        private final List<ResolvedDependency> resolvedDependencies;
        private final String errorMessage;
        private final long resolutionTimeMs;
        private final int totalArtifacts;
        private final long totalDownloadSize;

        ResolutionResult(boolean success, List<ResolvedDependency> resolvedDependencies,
                                String errorMessage, long resolutionTimeMs,
                                int totalArtifacts, long totalDownloadSize) {
            this.success = success;
            this.resolvedDependencies = resolvedDependencies;
            this.errorMessage = errorMessage;
            this.resolutionTimeMs = resolutionTimeMs;
            this.totalArtifacts = totalArtifacts;
            this.totalDownloadSize = totalDownloadSize;
        }

        public boolean isSuccess() { return success; }
        public List<ResolvedDependency> getResolvedDependencies() { return resolvedDependencies; }
        public String getErrorMessage() { return errorMessage; }
        public long getResolutionTimeMs() { return resolutionTimeMs; }
        public int getTotalArtifacts() { return totalArtifacts; }
        public long getTotalDownloadSize() { return totalDownloadSize; }
    }

    /**
     * Cache check result.
     */
    public static class CacheCheckResult {
        private final boolean cached;
        private final String localPath;
        private final long cachedSize;

        private CacheCheckResult(boolean cached, String localPath, long cachedSize) {
            this.cached = cached;
            this.localPath = localPath;
            this.cachedSize = cachedSize;
        }

        public static CacheCheckResult cached(String localPath, long cachedSize) {
            return new CacheCheckResult(true, localPath, cachedSize);
        }

        public static CacheCheckResult notCached() {
            return new CacheCheckResult(false, null, 0);
        }

        public boolean isCached() { return cached; }
        public String getLocalPath() { return localPath; }
        public long getCachedSize() { return cachedSize; }
    }

    /**
     * Result of downloading an external resource through the Rust transport.
     */
    public static class DownloadResult {
        private final boolean success;
        private final String errorMessage;
        private final long bytesWritten;
        private final long totalSize;

        private DownloadResult(boolean success, String errorMessage, long bytesWritten, long totalSize) {
            this.success = success;
            this.errorMessage = errorMessage;
            this.bytesWritten = bytesWritten;
            this.totalSize = totalSize;
        }

        public static DownloadResult success(long bytesWritten, long totalSize) {
            return new DownloadResult(true, "", bytesWritten, totalSize);
        }

        public static DownloadResult failed(String errorMessage) {
            return new DownloadResult(false, errorMessage, 0, 0);
        }

        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }
        public long getBytesWritten() { return bytesWritten; }
        public long getTotalSize() { return totalSize; }
    }

    public static class MavenArtifactCoordinate {
        private final String group;
        private final String name;
        private final String version;
        private final String classifier;
        private final String extension;

        public MavenArtifactCoordinate(String group, String name, String version, String classifier, String extension) {
            this.group = group;
            this.name = name;
            this.version = version;
            this.classifier = classifier;
            this.extension = extension;
        }
    }

    /**
     * Resolve a dependency graph via the Rust substrate daemon.
     */
    public ResolutionResult resolveDependencies(
        String configurationName,
        List<DependencyDescriptor> dependencies,
        List<RepositoryDescriptor> repositories,
        boolean lenient
    ) {
        return resolveDependencies(configurationName, dependencies, repositories, lenient, false);
    }

    /**
     * Resolve a dependency graph via the Rust substrate daemon.
     */
    public ResolutionResult resolveDependencies(
        String configurationName,
        List<DependencyDescriptor> dependencies,
        List<RepositoryDescriptor> repositories,
        boolean lenient,
        boolean prefetchArtifacts
    ) {
        try {
            return resolveDependenciesStrict(configurationName, dependencies, repositories, lenient, prefetchArtifacts);
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] gRPC call failed", e);
            return new ResolutionResult(false, new ArrayList<>(), e.getMessage(), 0, 0, 0);
        }
    }

    /**
     * Resolve a dependency graph via the Rust substrate daemon.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public ResolutionResult resolveDependenciesStrict(
        String configurationName,
        List<DependencyDescriptor> dependencies,
        List<RepositoryDescriptor> repositories,
        boolean lenient
    ) {
        return resolveDependenciesStrict(configurationName, dependencies, repositories, lenient, false);
    }

    /**
     * Resolve a dependency graph via the Rust substrate daemon.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public ResolutionResult resolveDependenciesStrict(
        String configurationName,
        List<DependencyDescriptor> dependencies,
        List<RepositoryDescriptor> repositories,
        boolean lenient,
        boolean prefetchArtifacts
    ) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        ResolveDependenciesResponse response = client.getDependencyResolutionStub()
            .resolveDependencies(ResolveDependenciesRequest.newBuilder()
                .setConfigurationName(configurationName)
                .addAllDependencies(dependencies)
                .addAllRepositories(repositories)
                .setLenient(lenient)
                .setPrefetchArtifacts(prefetchArtifacts)
                .build());

        if (response.getSuccess()) {
            LOGGER.debug("[substrate:dep-resolve] resolved {} deps in {}ms",
                response.getTotalArtifacts(), response.getResolutionTimeMs());
        } else {
            LOGGER.debug("[substrate:dep-resolve] resolution failed: {}", response.getErrorMessage());
        }

        return new ResolutionResult(
            response.getSuccess(),
            response.getResolvedDependenciesList(),
            response.getErrorMessage(),
            response.getResolutionTimeMs(),
            response.getTotalArtifacts(),
            response.getTotalDownloadSize()
        );
    }

    /**
     * Check if an artifact is in the local cache.
     */
    public CacheCheckResult checkArtifactCache(String group, String name, String version,
                                                   String classifier, String sha256) {
        return checkArtifactCache(group, name, version, classifier, "jar", sha256);
    }

    /**
     * Check if an artifact is in the local cache.
     */
    public CacheCheckResult checkArtifactCache(String group, String name, String version,
                                                   String classifier, String extension, String sha256) {
        try {
            return checkArtifactCacheStrict(group, name, version, classifier, extension, sha256);
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] cache check failed", e);
            return CacheCheckResult.notCached();
        }
    }

    /**
     * Check if an artifact is in the local cache.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public CacheCheckResult checkArtifactCacheStrict(
        String group,
        String name,
        String version,
        String classifier,
        String sha256
    ) {
        return checkArtifactCacheStrict(group, name, version, classifier, "jar", sha256);
    }

    /**
     * Check if an artifact is in the local cache.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public CacheCheckResult checkArtifactCacheStrict(
        String group,
        String name,
        String version,
        String classifier,
        String extension,
        String sha256
    ) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        CheckArtifactCacheResponse response = client.getDependencyResolutionStub()
            .checkArtifactCache(CheckArtifactCacheRequest.newBuilder()
                .setGroup(group)
                .setName(name)
                .setVersion(version)
                .setClassifier(classifier)
                .setExtension(extension)
                .setSha256(sha256)
                .build());

        return new CacheCheckResult(
            response.getCached(),
            response.getLocalPath(),
            response.getCachedSize_()
        );
    }

    /**
     * Check if external metadata is in the Rust metadata cache.
     */
    public CacheCheckResult checkMetadataCache(String url, String extension, String sha256) {
        try {
            return checkMetadataCacheStrict(url, extension, sha256);
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] metadata cache check failed", e);
            return CacheCheckResult.notCached();
        }
    }

    /**
     * Check if external metadata is in the Rust metadata cache.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public CacheCheckResult checkMetadataCacheStrict(String url, String extension, String sha256) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        CheckMetadataCacheResponse response = client.getDependencyResolutionStub()
            .checkMetadataCache(CheckMetadataCacheRequest.newBuilder()
                .setUrl(url)
                .setExtension(extension)
                .setSha256(sha256)
                .build());

        return new CacheCheckResult(
            response.getCached(),
            response.getLocalPath(),
            response.getCachedSize_()
        );
    }

    /**
     * Download an external resource through the Rust transport into {@code destination}.
     */
    public DownloadResult downloadResource(URI location, File destination) {
        return downloadResource(location, destination, null);
    }

    /**
     * Download an external resource through the Rust transport into {@code destination}.
     */
    public DownloadResult downloadResource(URI location, File destination, MavenArtifactCoordinate coordinate) {
        try {
            return downloadResourceStrict(location, destination, coordinate);
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] resource download failed", e);
            return DownloadResult.failed(e.getMessage());
        }
    }

    /**
     * Download an external resource through the Rust transport into {@code destination}.
     *
     * @throws RuntimeException when substrate is unavailable, the RPC fails, or the Rust transport reports an error.
     */
    public DownloadResult downloadResourceStrict(URI location, File destination) {
        return downloadResourceStrict(location, destination, null);
    }

    /**
     * Download an external resource through the Rust transport into {@code destination}.
     *
     * @throws RuntimeException when substrate is unavailable, the RPC fails, or the Rust transport reports an error.
     */
    public DownloadResult downloadResourceStrict(URI location, File destination, MavenArtifactCoordinate coordinate) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        DownloadArtifactRequest.Builder request = DownloadArtifactRequest.newBuilder()
            .setUrl(location.toString());
        if (coordinate != null) {
            request
                .setGroup(coordinate.group)
                .setName(coordinate.name)
                .setVersion(coordinate.version)
                .setClassifier(coordinate.classifier)
                .setExtension(coordinate.extension);
        } else {
            String metadataExtension = metadataExtensionFor(location);
            if (!metadataExtension.isEmpty()) {
                request.setExtension(metadataExtension);
            }
        }

        Iterator<DownloadArtifactChunk> chunks = client.getDependencyResolutionStub()
            .downloadArtifact(request.build());

        long bytesWritten = 0;
        long totalSize = -1;
        try (BufferedOutputStream output = new BufferedOutputStream(new FileOutputStream(destination))) {
            while (chunks.hasNext()) {
                DownloadArtifactChunk chunk = chunks.next();
                if (!chunk.getErrorMessage().isEmpty()) {
                    destination.delete();
                    throw new IllegalStateException(chunk.getErrorMessage());
                }
                if (chunk.getTotalSize() >= 0) {
                    totalSize = chunk.getTotalSize();
                }
                if (!chunk.getData().isEmpty()) {
                    chunk.getData().writeTo(output);
                    bytesWritten += chunk.getData().size();
                }
            }
        } catch (Exception e) {
            destination.delete();
            throw new RuntimeException("Rust resource download failed for " + location, e);
        }

        return DownloadResult.success(bytesWritten, totalSize);
    }

    private static String metadataExtensionFor(URI location) {
        String path = location.getPath();
        if (path == null) {
            return "";
        }
        String lowerPath = path.toLowerCase(Locale.ROOT);
        if (lowerPath.endsWith(".pom")) {
            return "pom";
        }
        if (lowerPath.endsWith(".module")) {
            return "module";
        }
        if (lowerPath.endsWith(".ivy")) {
            return "ivy";
        }
        if (lowerPath.endsWith("/maven-metadata.xml")) {
            return "maven-metadata.xml";
        }
        return "";
    }

    /**
     * Record a resolution result for tracking.
     */
    public void recordResolution(
        String configurationName,
        long resolutionTimeMs,
        int dependencyCount,
        boolean success,
        long cacheHits
    ) {
        try {
            recordResolutionStrict(configurationName, resolutionTimeMs, dependencyCount, success, cacheHits);
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] record resolution failed", e);
        }
    }

    /**
     * Record a resolution result for tracking.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public RecordResolutionResponse recordResolutionStrict(
        String configurationName,
        long resolutionTimeMs,
        int dependencyCount,
        boolean success,
        long cacheHits
    ) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        return client.getDependencyResolutionStub()
            .recordResolution(RecordResolutionRequest.newBuilder()
                .setConfigurationName(configurationName)
                .setDependencyCount(dependencyCount)
                .setResolutionTimeMs(resolutionTimeMs)
                .setSuccess(success)
                .setCacheHits(cacheHits)
                .build());
    }

    /**
     * Add an artifact to the Rust-side cache after download.
     */
    public boolean addArtifactToCache(String group, String name, String version,
                                       String classifier, String localPath,
                                       long size, String sha256) {
        return addArtifactToCache(group, name, version, classifier, "jar", localPath, size, sha256);
    }

    /**
     * Add an artifact to the Rust-side cache after download.
     */
    public boolean addArtifactToCache(String group, String name, String version,
                                       String classifier, String extension, String localPath,
                                       long size, String sha256) {
        try {
            return addArtifactToCacheStrict(group, name, version, classifier, extension, localPath, size, sha256);
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] add artifact to cache failed", e);
            return false;
        }
    }

    /**
     * Add an artifact to the Rust-side cache after download.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public boolean addArtifactToCacheStrict(
        String group,
        String name,
        String version,
        String classifier,
        String localPath,
        long size,
        String sha256
    ) {
        return addArtifactToCacheStrict(group, name, version, classifier, "jar", localPath, size, sha256);
    }

    /**
     * Add an artifact to the Rust-side cache after download.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public boolean addArtifactToCacheStrict(
        String group,
        String name,
        String version,
        String classifier,
        String extension,
        String localPath,
        long size,
        String sha256
    ) {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        AddArtifactToCacheResponse response = client.getDependencyResolutionStub()
            .addArtifactToCache(AddArtifactToCacheRequest.newBuilder()
                .setGroup(group)
                .setName(name)
                .setVersion(version)
                .setClassifier(classifier)
                .setExtension(extension)
                .setLocalPath(localPath)
                .setSize(size)
                .setSha256(sha256)
                .build());
        return response.getAccepted();
    }

    /**
     * Get resolution statistics from the Rust service.
     */
    public GetResolutionStatsResponse getResolutionStats() {
        try {
            return getResolutionStatsStrict();
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] get resolution stats failed", e);
            return GetResolutionStatsResponse.getDefaultInstance();
        }
    }

    /**
     * Get resolution statistics from the Rust service.
     *
     * @throws RuntimeException when substrate is unavailable or the RPC fails.
     */
    public GetResolutionStatsResponse getResolutionStatsStrict() {
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }
        return client.getDependencyResolutionStub()
            .getResolutionStats(GetResolutionStatsRequest.newBuilder().build());
    }
}
