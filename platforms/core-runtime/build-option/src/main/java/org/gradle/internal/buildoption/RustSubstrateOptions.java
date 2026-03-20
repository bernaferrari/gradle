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
 *   -Dorg.gradle.rust.substrate.execution.advisory=true
 *   -Dorg.gradle.rust.substrate.execution.authoritative=true
 *   -Dorg.gradle.rust.substrate.history.enabled=true
 *   -Dorg.gradle.rust.substrate.fingerprint.enabled=true
 *   -Dorg.gradle.rust.substrate.snapshot.enabled=true
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

    /**
     * Enable Phase 7: Rust-native execution history storage.
     * Property: org.gradle.rust.substrate.history.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_HISTORY =
        new InternalFlag("org.gradle.rust.substrate.history.enabled", false);

    /**
     * Enable Phase 9: Rust-native file fingerprinting.
     * Property: org.gradle.rust.substrate.fingerprint.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_FINGERPRINTING =
        new InternalFlag("org.gradle.rust.substrate.fingerprint.enabled", false);

    /**
     * Enable Phase 10: Rust-native value snapshotting.
     * Property: org.gradle.rust.substrate.snapshot.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_SNAPSHOTTING =
        new InternalFlag("org.gradle.rust.substrate.snapshot.enabled", false);

    /**
     * Enable Phase 11: Rust-native task graph management.
     * Property: org.gradle.rust.substrate.taskgraph.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_TASK_GRAPH =
        new InternalFlag("org.gradle.rust.substrate.taskgraph.enabled", false);

    /**
     * Enable Phase 12: Rust-native configuration model.
     * Property: org.gradle.rust.substrate.configuration.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_CONFIGURATION =
        new InternalFlag("org.gradle.rust.substrate.configuration.enabled", false);

    /**
     * Enable Phase 13: Rust-native plugin system.
     * Property: org.gradle.rust.substrate.plugin.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_PLUGIN =
        new InternalFlag("org.gradle.rust.substrate.plugin.enabled", false);

    /**
     * Enable Phase 14: Rust-native build operations.
     * Property: org.gradle.rust.substrate.buildops.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_BUILD_OPS =
        new InternalFlag("org.gradle.rust.substrate.buildops.enabled", false);

    /**
     * Enable Phase 15: Rust-native bootstrap (build session lifecycle).
     * Property: org.gradle.rust.substrate.bootstrap.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_BOOTSTRAP =
        new InternalFlag("org.gradle.rust.substrate.bootstrap.enabled", false);

    /**
     * Enable Phase 18: Rust-native dependency resolution.
     * Property: org.gradle.rust.substrate.dependency.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_DEPENDENCY_RESOLUTION =
        new InternalFlag("org.gradle.rust.substrate.dependency.enabled", false);

    /**
     * Enable Phase 19: Rust-native file system watching.
     * Property: org.gradle.rust.substrate.filewatch.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_FILE_WATCH =
        new InternalFlag("org.gradle.rust.substrate.filewatch.enabled", false);

    /**
     * Enable Phase 20: Rust-native configuration cache.
     * Property: org.gradle.rust.substrate.configcache.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_CONFIG_CACHE =
        new InternalFlag("org.gradle.rust.substrate.configcache.enabled", false);

    /**
     * Enable Phase 23: Rust-native toolchain management.
     * Property: org.gradle.rust.substrate.toolchain.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_TOOLCHAIN =
        new InternalFlag("org.gradle.rust.substrate.toolchain.enabled", false);

    /**
     * Enable Phase 24: Rust-native build event streaming.
     * Property: org.gradle.rust.substrate.eventstream.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_EVENT_STREAM =
        new InternalFlag("org.gradle.rust.substrate.eventstream.enabled", false);

    /**
     * Enable Phase 25: Rust-native worker process management.
     * Property: org.gradle.rust.substrate.worker.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_WORKER_PROCESS =
        new InternalFlag("org.gradle.rust.substrate.worker.enabled", false);

    /**
     * Enable Phase 26: Rust-native build layout / project model.
     * Property: org.gradle.rust.substrate.buildlayout.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_BUILD_LAYOUT =
        new InternalFlag("org.gradle.rust.substrate.buildlayout.enabled", false);

    /**
     * Enable Phase 28: Rust-native build result reporting.
     * Property: org.gradle.rust.substrate.buildresult.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_BUILD_RESULT =
        new InternalFlag("org.gradle.rust.substrate.buildresult.enabled", false);

    /**
     * Enable Phase 29: Rust-native problem/diagnostic reporting.
     * Property: org.gradle.rust.substrate.problems.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_PROBLEMS =
        new InternalFlag("org.gradle.rust.substrate.problems.enabled", false);

    /**
     * Enable Phase 30: Rust-native resource management.
     * Property: org.gradle.rust.substrate.resources.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_RESOURCES =
        new InternalFlag("org.gradle.rust.substrate.resources.enabled", false);

    /**
     * Enable Phase 31: Rust-native build comparison.
     * Property: org.gradle.rust.substrate.comparison.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_COMPARISON =
        new InternalFlag("org.gradle.rust.substrate.comparison.enabled", false);

    /**
     * Enable Phase 32: Rust-native console / rich output.
     * Property: org.gradle.rust.substrate.console.enabled
     * Default: false
     */
    public static final InternalFlag ENABLE_RUST_CONSOLE =
        new InternalFlag("org.gradle.rust.substrate.console.enabled", false);

    private RustSubstrateOptions() {
        // utility class
    }
}
