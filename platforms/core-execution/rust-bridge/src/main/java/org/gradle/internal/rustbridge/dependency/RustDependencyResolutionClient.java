package org.gradle.internal.rustbridge.dependency;

import gradle.substrate.v1.AddArtifactToCacheRequest;
import gradle.substrate.v1.AddArtifactToCacheResponse;
import gradle.substrate.v1.CheckArtifactCacheRequest;
import gradle.substrate.v1.CheckArtifactCacheResponse;
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
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.List;

/**
 * Client for the Rust dependency resolution service.
 * Resolves dependency graphs, checks artifact caches, and downloads artifacts via gRPC.
 */
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

        private ResolutionResult(boolean success, List<ResolvedDependency> resolvedDependencies,
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

        public boolean isCached() { return cached; }
        public String getLocalPath() { return localPath; }
        public long getCachedSize() { return cachedSize; }
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
        try {
            return resolveDependenciesStrict(configurationName, dependencies, repositories, lenient);
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
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        ResolveDependenciesResponse response = client.getDependencyResolutionStub()
            .resolveDependencies(ResolveDependenciesRequest.newBuilder()
                .setConfigurationName(configurationName)
                .addAllDependencies(dependencies)
                .addAllRepositories(repositories)
                .setLenient(lenient)
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
        try {
            return checkArtifactCacheStrict(group, name, version, classifier, sha256);
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] cache check failed", e);
            return new CacheCheckResult(false, null, 0);
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
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        CheckArtifactCacheResponse response = client.getDependencyResolutionStub()
            .checkArtifactCache(CheckArtifactCacheRequest.newBuilder()
                .setGroup(group)
                .setName(name)
                .setVersion(version)
                .setClassifier(classifier)
                .setSha256(sha256)
                .build());

        return new CacheCheckResult(
            response.getCached(),
            response.getLocalPath(),
            response.getCachedSize_()
        );
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
        try {
            return addArtifactToCacheStrict(group, name, version, classifier, localPath, size, sha256);
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
        if (client.isNoop()) {
            throw new IllegalStateException("Substrate not available");
        }

        AddArtifactToCacheResponse response = client.getDependencyResolutionStub()
            .addArtifactToCache(AddArtifactToCacheRequest.newBuilder()
                .setGroup(group)
                .setName(name)
                .setVersion(version)
                .setClassifier(classifier)
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
