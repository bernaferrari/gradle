package org.gradle.internal.buildoption;

/**
 * Feature flags for the Rust execution substrate.
 *
 * These are internal options controlled via system properties:
 *   -Dorg.gradle.rust.substrate.enabled=true
 *   -Dorg.gradle.rust.substrate.hashing.enabled=true
 *   -Dorg.gradle.rust.substrate.hashing.shadow=true
 *   -Dorg.gradle.rust.substrate.cache.enabled=true
 *   -Dorg.gradle.rust.substrate.exec.enabled=true
 *   -Dorg.gradle.rust.substrate.daemon.path=/path/to/daemon
 *   -Dorg.gradle.rust.substrate.shadow.report-mismatches=true
 */
public class RustSubstrateOptions {

    /**
     * Master switch: enable the Rust substrate daemon.
     * Property: org.gradle.rust.substrate.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_SUBSTRATE =
        new InternalFlag("org.gradle.rust.substrate.enabled", false);

    /**
     * Enable Rust-backed hashing (requires ENABLE_SUBSTRATE).
     * Property: org.gradle.rust.substrate.hashing.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_HASHING =
        new InternalFlag("org.gradle.rust.substrate.hashing.enabled", false);

    /**
     * Shadow mode for hashing: run both Java and Rust, compare results.
     * Property: org.gradle.rust.substrate.hashing.shadow
     * Default: true (when hashing is enabled, start in shadow mode)
     */
    public static final InternalFlag SHADOW_HASHING =
        new InternalFlag("org.gradle.rust.substrate.hashing.shadow", true);

    /**
     * Enable Rust-backed build cache.
     * Property: org.gradle.rust.substrate.cache.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_CACHE =
        new InternalFlag("org.gradle.rust.substrate.cache.enabled", false);

    /**
     * Enable Rust-backed process execution.
     * Property: org.gradle.rust.substrate.exec.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_EXEC =
        new InternalFlag("org.gradle.rust.substrate.exec.enabled", false);

    /**
     * Path to the Rust daemon binary (for development).
     * Property: org.gradle.rust.substrate.daemon.path
     * Default: "" (auto-detect)
     */
    public static final InternalOption<String> DAEMON_BINARY_PATH =
        StringInternalOption.of("org.gradle.rust.substrate.daemon.path", "");

    /**
     * Report mismatches found during shadow mode.
     * Property: org.gradle.rust.substrate.shadow.report-mismatches
     * Default: true
     */
    public static final InternalFlag REPORT_MISMATCHES =
        new InternalFlag("org.gradle.rust.substrate.shadow.report-mismatches", true);

    /**
     * Enable Phase 5: Advisory ExecutionEngine.
     * Rust predicts outcomes, Java remains authoritative.
     * Property: org.gradle.rust.substrate.execution.advisory
     * Default: false
     */
    public static final InternalFlag ENABLE_ADVISORY_EXECUTION =
        new InternalFlag("org.gradle.rust.substrate.execution.advisory", false);

    /**
     * Enable Phase 6: Authoritative ExecutionEngine.
     * Rust drives work identity, caching, and up-to-date decisions.
     * Property: org.gradle.rust.substrate.execution.authoritative
     * Default: false
     */
    public static final InternalFlag ENABLE_AUTHORITATIVE_EXECUTION =
        new InternalFlag("org.gradle.rust.substrate.execution.authoritative", false);

    private RustSubstrateOptions() {
        // utility class
    }
}
