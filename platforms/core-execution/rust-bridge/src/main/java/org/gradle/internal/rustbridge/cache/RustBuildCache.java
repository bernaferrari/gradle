package org.gradle.internal.rustbridge.cache;

import org.gradle.caching.BuildCache;
import org.gradle.caching.BuildCacheService;
import org.gradle.caching.configuration.BuildCacheServiceFactory;

/**
 * Configuration for the Rust-backed build cache.
 * Usage in settings.gradle:
 *   buildCache { remote(RustBuildCache) {} }
 */
public abstract class RustBuildCache implements BuildCache {
}
