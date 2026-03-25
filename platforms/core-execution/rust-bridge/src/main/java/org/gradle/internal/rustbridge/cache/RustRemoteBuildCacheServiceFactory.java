package org.gradle.internal.rustbridge.cache;

import org.gradle.caching.BuildCacheService;
import org.gradle.caching.BuildCacheServiceFactory;
import org.gradle.internal.rustbridge.SubstrateClient;

/**
 * Factory for creating {@link RustRemoteBuildCacheService} instances.
 * The Rust daemon handles remote GET/PUT internally with retry logic.
 */
public class RustRemoteBuildCacheServiceFactory implements BuildCacheServiceFactory<RustRemoteBuildCache> {

    private final SubstrateClient client;

    public RustRemoteBuildCacheServiceFactory(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public BuildCacheService createBuildCacheService(
        RustRemoteBuildCache configuration,
        BuildCacheServiceFactory.Describer describer
    ) {
        describer.type("Rust Remote");
        describer.config("backend", "rust-remote-cache");
        return new RustRemoteBuildCacheService(client);
    }
}
