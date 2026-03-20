package org.gradle.internal.rustbridge.cache;

import org.gradle.caching.configuration.internal.BuildCacheServiceRegistration;
import org.gradle.caching.configuration.internal.DefaultBuildCacheServiceRegistration;
import org.gradle.internal.service.ServiceRegistration;
import org.gradle.internal.service.scopes.AbstractGradleModuleServices;

/**
 * Registers the Rust build cache service via the BuildCacheServiceRegistration SPI.
 */
public class RustBridgeCacheServices extends AbstractGradleModuleServices {

    @Override
    public void registerBuildServices(ServiceRegistration registration) {
        registration.add(
            BuildCacheServiceRegistration.class,
            new DefaultBuildCacheServiceRegistration(
                RustBuildCache.class,
                RustBuildCacheServiceFactory.class
            )
        );
    }
}
