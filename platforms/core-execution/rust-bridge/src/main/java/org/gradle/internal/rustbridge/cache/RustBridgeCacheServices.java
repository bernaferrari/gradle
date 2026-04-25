package org.gradle.internal.rustbridge.cache;

import org.gradle.api.logging.Logging;
import org.gradle.internal.service.ServiceRegistration;
import org.gradle.internal.service.scopes.AbstractGradleModuleServices;
import org.slf4j.Logger;

import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;

/**
 * Registers the Rust build cache service via the BuildCacheServiceRegistration SPI.
 */
public class RustBridgeCacheServices extends AbstractGradleModuleServices {

    private static final Logger LOGGER = Logging.getLogger(RustBridgeCacheServices.class);
    private static final String REGISTRATION_TYPE = "org.gradle.caching.configuration.internal.BuildCacheServiceRegistration";
    private static final String DEFAULT_REGISTRATION_TYPE = "org.gradle.caching.configuration.internal.DefaultBuildCacheServiceRegistration";

    @Override
    public void registerBuildServices(ServiceRegistration registration) {
        registerCache(registration, RustBuildCache.class, RustBuildCacheServiceFactory.class);
        registerCache(registration, RustRemoteBuildCache.class, RustRemoteBuildCacheServiceFactory.class);
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static void registerCache(
        ServiceRegistration registration,
        Class<?> configurationType,
        Class<?> factoryType
    ) {
        try {
            Class<?> serviceType = Class.forName(REGISTRATION_TYPE);
            Class<?> implementationType = Class.forName(DEFAULT_REGISTRATION_TYPE);
            Constructor<?> constructor = implementationType.getConstructor(Class.class, Class.class);
            Object service = constructor.newInstance(configurationType, factoryType);
            registration.add((Class) serviceType, service);
        } catch (ClassNotFoundException e) {
            LOGGER.debug("[substrate:cache] build cache registration SPI unavailable", e);
        } catch (NoSuchMethodException e) {
            throw new IllegalStateException("Build cache registration constructor changed", e);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException("Cannot access build cache registration constructor", e);
        } catch (InstantiationException e) {
            throw new IllegalStateException("Cannot instantiate build cache registration", e);
        } catch (InvocationTargetException e) {
            Throwable cause = e.getCause();
            throw new IllegalStateException("Build cache registration failed", cause != null ? cause : e);
        }
    }
}
