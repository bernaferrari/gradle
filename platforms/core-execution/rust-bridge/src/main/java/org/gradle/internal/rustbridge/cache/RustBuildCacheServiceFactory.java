package org.gradle.internal.rustbridge.cache;

import org.gradle.caching.BuildCacheService;
import org.gradle.caching.BuildCacheServiceFactory;
import org.gradle.internal.rustbridge.SubstrateClient;

/**
 * Factory for creating {@link RustBuildCacheService} instances.
 * Registered via the BuildCacheServiceRegistration SPI.
 */
public class RustBuildCacheServiceFactory implements BuildCacheServiceFactory<RustBuildCache> {

    private final SubstrateClient client;

    public RustBuildCacheServiceFactory(SubstrateClient client) {
        this.client = client;
    }

    @Override
    public BuildCacheService createBuildCacheService(
        RustBuildCache configuration,
        BuildCacheServiceFactory.Describer describer
    ) {
        describer.type("Rust Local");
        describer.config("backend", "rust-local-filesystem");
        return new RustBuildCacheService(client);
    }
}
