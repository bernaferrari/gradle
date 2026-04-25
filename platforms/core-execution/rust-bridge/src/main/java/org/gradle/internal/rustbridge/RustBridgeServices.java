package org.gradle.internal.rustbridge;

/**
 * Legacy compatibility entry point for the Rust bridge service layer.
 *
 * <p>The original implementation accumulated a large amount of stale provider wiring
 * that no longer matched the actively compiled bridge surface. The live mixed-mode
 * registration now flows through {@link RustBridgeCoreServices} and
 * {@code org.gradle.internal.rustbridge.cache.RustBridgeCacheServices}. This type is
 * kept compile-safe so older references still resolve while the active descriptor stays
 * focused on the smaller validated service set.</p>
 */
@Deprecated
public class RustBridgeServices extends RustBridgeCoreServices {
}
