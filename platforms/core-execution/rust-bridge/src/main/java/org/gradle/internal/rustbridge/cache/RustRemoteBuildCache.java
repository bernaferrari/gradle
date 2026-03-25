package org.gradle.internal.rustbridge.cache;

import org.gradle.caching.configuration.BuildCache;

/**
 * Configuration for the Rust remote build cache.
 * The actual remote URL and credentials are configured on the daemon side.
 */
public abstract class RustRemoteBuildCache implements BuildCache {
}
