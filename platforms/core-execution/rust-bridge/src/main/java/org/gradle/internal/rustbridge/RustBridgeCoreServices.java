package org.gradle.internal.rustbridge;

import org.gradle.internal.buildoption.InternalOptions;
import org.gradle.internal.buildoption.RustSubstrateOptions;
import org.gradle.internal.event.ListenerManager;
import org.gradle.internal.rustbridge.cache.BuildCacheOrchestrationClient;
import org.gradle.internal.rustbridge.configcache.ConfigurationCacheShadowListener;
import org.gradle.internal.rustbridge.configcache.RustConfigCacheClient;
import org.gradle.internal.rustbridge.dependency.DependencyResolutionShadowListener;
import org.gradle.internal.rustbridge.dependency.RustDependencyResolutionClient;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.rustbridge.shadow.ShadowingBuildCacheKeyComputer;
import org.gradle.internal.service.Provides;
import org.gradle.internal.service.ServiceRegistration;
import org.gradle.internal.service.ServiceRegistrationProvider;
import org.gradle.internal.service.scopes.AbstractGradleModuleServices;
import org.jspecify.annotations.Nullable;

/**
 * Minimal compile-safe service wiring for the Rust bridge.
 *
 * <p>This intentionally avoids heavyweight launcher/compat classes that are still
 * excluded from this module's compile, while activating the Rust paths that are
 * already stable in shadow/authoritative fallback mode.</p>
 */
public class RustBridgeCoreServices extends AbstractGradleModuleServices {

    @Override
    public void registerGradleUserHomeServices(ServiceRegistration registration) {
        registration.addProvider(new UserHomeServices());
    }

    @Override
    public void registerBuildSessionServices(ServiceRegistration registration) {
        registration.addProvider(new BuildSessionServices());
    }

    @Override
    public void registerBuildServices(ServiceRegistration registration) {
        registration.addProvider(new BuildServices());
    }

    private static class UserHomeServices implements ServiceRegistrationProvider {
        @Provides
        SubstrateClient createSubstrateClient(InternalOptions options) {
            return RustDaemonSidecarLauncher.connectOrLaunch(options);
        }
    }

    private static class BuildSessionServices implements ServiceRegistrationProvider {
        @Provides
        HashMismatchReporter createHashMismatchReporter() {
            return new HashMismatchReporter(true);
        }
    }

    private static class BuildServices implements ServiceRegistrationProvider {
        @Provides
        BuildCacheOrchestrationClient createBuildCacheOrchestrationClient(SubstrateClient client) {
            return new BuildCacheOrchestrationClient(client);
        }

        @Provides
        RustDependencyResolutionClient createRustDependencyResolutionClient(SubstrateClient client) {
            return new RustDependencyResolutionClient(client);
        }

        @Provides
        RustConfigCacheClient createRustConfigCacheClient(SubstrateClient client) {
            return new RustConfigCacheClient(client);
        }

        @Provides
        ShadowingBuildCacheKeyComputer createShadowingBuildCacheKeyComputer(
            BuildCacheOrchestrationClient cacheOrchestrationClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_CACHE
            );
            return new ShadowingBuildCacheKeyComputer(cacheOrchestrationClient, mismatchReporter, authoritative);
        }

        @Provides
        @Nullable
        ConfigurationCacheShadowListener createConfigurationCacheShadowListener(
            RustConfigCacheClient rustConfigCacheClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_CONFIG_CACHE)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_CONFIG_CACHE
            );
            return new ConfigurationCacheShadowListener(rustConfigCacheClient, mismatchReporter, authoritative);
        }

        @Provides
        @Nullable
        DependencyResolutionShadowListener createDependencyResolutionShadowListener(
            RustDependencyResolutionClient rustDependencyResolutionClient,
            HashMismatchReporter mismatchReporter,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_DEPENDENCY_RESOLUTION)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_DEPENDENCY_RESOLUTION
            );
            DependencyResolutionShadowListener listener =
                new DependencyResolutionShadowListener(rustDependencyResolutionClient, mismatchReporter, authoritative);
            listenerManager.addListener(listener);
            return listener;
        }
    }
}
