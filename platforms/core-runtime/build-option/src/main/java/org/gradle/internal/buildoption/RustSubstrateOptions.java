package org.gradle.internal.buildoption;

/**
 * Feature flags for the Rust execution substrate.
 *
 * <h3>Umbrella mode (recommended)</h3>
 * <pre>
 *   -Dorg.gradle.rust.substrate.mode=shadow        # all services enabled in shadow mode
 *   -Dorg.gradle.rust.substrate.mode=authoritative # all services enabled, Rust authoritative
 * </pre>
 *
 * <h3>Per-service flags (backward compatible)</h3>
 * <pre>
 *   -Dorg.gradle.rust.substrate.enabled=true
 *   -Dorg.gradle.rust.substrate.hashing.enabled=true
 *   -Dorg.gradle.rust.substrate.daemon.path=/path/to/daemon
 *   ...
 * </pre>
 *
 * When {@code org.gradle.rust.substrate.mode} is set, it overrides all per-service
 * flags. When not set, the system falls back to individual per-service flags.
 */
public class RustSubstrateOptions {

    /**
     * Umbrella mode for the Rust substrate.
     * Property: org.gradle.rust.substrate.mode
     * Values: "" (not set = use per-service flags), "off", "shadow", "authoritative"
     * Default: ""
     */
    public enum SubstrateMode {
        OFF,
        SHADOW,
        AUTHORITATIVE
    }

    public static final InternalOption<String> SUBSTRATE_MODE =
        InternalOptions.ofString("org.gradle.rust.substrate.mode", "");

    /**
     * Master switch: enable the Rust substrate daemon.
     * Property: org.gradle.rust.substrate.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_SUBSTRATE =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.enabled", false);

    /**
     * Enable Rust-backed hashing (requires ENABLE_SUBSTRATE).
     * Property: org.gradle.rust.substrate.hashing.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_HASHING =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.hashing.enabled", false);

    /**
     * Shadow mode for hashing: run both Java and Rust, compare results.
     * Property: org.gradle.rust.substrate.hashing.shadow
     * Default: true (when hashing is enabled, start in shadow mode)
     */
    public static final InternalOption<Boolean> SHADOW_HASHING =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.hashing.shadow", true);

    /**
     * Enable Rust-backed build cache.
     * Property: org.gradle.rust.substrate.cache.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_CACHE =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.cache.enabled", false);

    /**
     * Enable Rust-backed process execution.
     * Property: org.gradle.rust.substrate.exec.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_EXEC =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.exec.enabled", false);

    /**
     * Path to the Rust daemon binary (for development).
     * Property: org.gradle.rust.substrate.daemon.path
     * Default: "" (auto-detect)
     */
    public static final InternalOption<String> DAEMON_BINARY_PATH =
        InternalOptions.ofString("org.gradle.rust.substrate.daemon.path", "");

    /**
     * Report mismatches found during shadow mode.
     * Property: org.gradle.rust.substrate.shadow.report-mismatches
     * Default: true
     */
    public static final InternalOption<Boolean> REPORT_MISMATCHES =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.shadow.report-mismatches", true);

    /**
     * Enable Phase 5: Advisory ExecutionEngine.
     * Property: org.gradle.rust.substrate.execution.advisory
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_ADVISORY_EXECUTION =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.execution.advisory", false);

    /**
     * Enable Phase 6: Authoritative ExecutionEngine.
     * Property: org.gradle.rust.substrate.execution.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_AUTHORITATIVE_EXECUTION =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.execution.authoritative", false);

    /**
     * Enable Phase 7: Rust-native execution history storage.
     * Property: org.gradle.rust.substrate.history.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_HISTORY =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.history.enabled", false);

    /**
     * Enable Phase 9: Rust-native file fingerprinting.
     * Property: org.gradle.rust.substrate.fingerprint.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_FINGERPRINTING =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.fingerprint.enabled", false);

    /**
     * Enable Phase 10: Rust-native value snapshotting.
     * Property: org.gradle.rust.substrate.snapshot.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_SNAPSHOTTING =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.snapshot.enabled", false);

    /**
     * Enable Phase 11: Rust-native task graph management.
     * Property: org.gradle.rust.substrate.taskgraph.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_TASK_GRAPH =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.taskgraph.enabled", false);

    /**
     * Enable Phase 12: Rust-native configuration model.
     * Property: org.gradle.rust.substrate.configuration.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_CONFIGURATION =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.configuration.enabled", false);

    /**
     * Enable Phase 13: Rust-native plugin system.
     * Property: org.gradle.rust.substrate.plugin.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_PLUGIN =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.plugin.enabled", false);

    /**
     * Enable Phase 14: Rust-native build operations.
     * Property: org.gradle.rust.substrate.buildops.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_OPS =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.buildops.enabled", false);

    /**
     * Enable Phase 15: Rust-native bootstrap (build session lifecycle).
     * Property: org.gradle.rust.substrate.bootstrap.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_BOOTSTRAP =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.bootstrap.enabled", false);

    /**
     * Enable Phase 18: Rust-native dependency resolution.
     * Property: org.gradle.rust.substrate.dependency.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_DEPENDENCY_RESOLUTION =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.dependency.enabled", false);

    /**
     * Enable Phase 19: Rust-native file system watching.
     * Property: org.gradle.rust.substrate.filewatch.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_FILE_WATCH =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.filewatch.enabled", false);

    /**
     * Enable Phase 20: Rust-native configuration cache.
     * Property: org.gradle.rust.substrate.configcache.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_CONFIG_CACHE =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.configcache.enabled", false);

    /**
     * Enable Phase 23: Rust-native toolchain management.
     * Property: org.gradle.rust.substrate.toolchain.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_TOOLCHAIN =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.toolchain.enabled", false);

    /**
     * Enable Phase 24: Rust-native build event streaming.
     * Property: org.gradle.rust.substrate.eventstream.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_EVENT_STREAM =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.eventstream.enabled", false);

    /**
     * Enable Phase 25: Rust-native worker process management.
     * Property: org.gradle.rust.substrate.worker.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_WORKER_PROCESS =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.worker.enabled", false);

    /**
     * Enable Phase 26: Rust-native build layout / project model.
     * Property: org.gradle.rust.substrate.buildlayout.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_LAYOUT =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.buildlayout.enabled", false);

    /**
     * Enable Phase 28: Rust-native build result reporting.
     * Property: org.gradle.rust.substrate.buildresult.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_RESULT =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.buildresult.enabled", false);

    /**
     * Enable Phase 29: Rust-native problem/diagnostic reporting.
     * Property: org.gradle.rust.substrate.problems.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_PROBLEMS =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.problems.enabled", false);

    /**
     * Enable Phase 30: Rust-native resource management.
     * Property: org.gradle.rust.substrate.resources.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_RESOURCES =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.resources.enabled", false);

    /**
     * Enable Phase 31: Rust-native build comparison.
     * Property: org.gradle.rust.substrate.comparison.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_COMPARISON =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.comparison.enabled", false);

    /**
     * Enable Phase 32: Rust-native console / rich output.
     * Property: org.gradle.rust.substrate.console.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_CONSOLE =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.console.enabled", false);

    /**
     * Enable Phase 33: Rust-native test execution.
     * Property: org.gradle.rust.substrate.testexec.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_TEST_EXECUTION =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.testexec.enabled", false);

    /**
     * Enable Phase 34: Rust-native artifact publishing.
     * Property: org.gradle.rust.substrate.publishing.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_PUBLISHING =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.publishing.enabled", false);

    /**
     * Enable Phase 35: Rust-native build initialization.
     * Property: org.gradle.rust.substrate.buildinit.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_INIT =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.buildinit.enabled", false);

    /**
     * Enable Phase 36: Rust-native incremental compilation.
     * Property: org.gradle.rust.substrate.incremental.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_INCREMENTAL =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.incremental.enabled", false);

    /**
     * Enable Phase 37: Rust-native build metrics tracking.
     * Property: org.gradle.rust.substrate.metrics.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_METRICS =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.metrics.enabled", false);

    /**
     * Enable Phase 38: Rust-native garbage collection.
     * Property: org.gradle.rust.substrate.gc.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_GC =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.gc.enabled", false);

    /**
     * Enable authoritative mode for Rust-backed hashing subsystem.
     * Property: org.gradle.rust.substrate.hashing.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_HASHING =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.hashing.authoritative", false);

    /**
     * Enable authoritative mode for Rust-backed build cache subsystem.
     * Property: org.gradle.rust.substrate.cache.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_CACHE =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.cache.authoritative", false);

    /**
     * Enable authoritative mode for Rust-backed configuration cache subsystem.
     * Property: org.gradle.rust.substrate.configcache.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_CONFIG_CACHE =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.configcache.authoritative", false);

    /**
     * Enable authoritative mode for Rust-backed file fingerprinting subsystem.
     * Property: org.gradle.rust.substrate.fingerprint.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_FINGERPRINTING =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.fingerprint.authoritative", false);

    /**
     * Enable authoritative mode for Rust-backed value snapshotting subsystem.
     * Property: org.gradle.rust.substrate.snapshot.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_SNAPSHOTTING =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.snapshot.authoritative", false);

    /**
     * Enable authoritative mode for Rust-backed task graph subsystem.
     * Property: org.gradle.rust.substrate.taskgraph.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_TASK_GRAPH =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.taskgraph.authoritative", false);

    /**
     * Enable authoritative mode for Rust-backed execution plan advisory subsystem.
     * Property: org.gradle.rust.substrate.executionplan.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_EXECUTION_PLAN =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.executionplan.authoritative", false);

    /**
     * Enable authoritative mode for Rust-backed process execution subsystem.
     * Property: org.gradle.rust.substrate.exec.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_EXEC =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.exec.authoritative", false);

    /**
     * Enable authoritative mode for Rust-backed file watching subsystem.
     * Property: org.gradle.rust.substrate.filewatch.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_FILE_WATCH =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.filewatch.authoritative", false);

    /**
     * Enable authoritative mode for Rust-backed dependency resolution subsystem.
     * Property: org.gradle.rust.substrate.dependency.authoritative
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_DEPENDENCY_RESOLUTION =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.dependency.authoritative", false);

    /**
     * Enable Phase 6: JVM Compatibility Host.
     * Property: org.gradle.rust.substrate.jvm.host.enabled
     * Default: false
     */
    public static final InternalOption<Boolean> ENABLE_JVM_HOST =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.jvm.host.enabled", false);

    // --- Umbrella mode helpers ---

    /**
     * Resolve the effective substrate mode.
     * Returns the mode from the umbrella flag, or null if not set (use per-service flags).
     */
    public static SubstrateMode getMode(InternalOptions options) {
        String modeString = options.getOptionValue(SUBSTRATE_MODE).get();
        if (modeString == null || modeString.isEmpty()) {
            return null; // not set — fall back to per-service flags
        }
        try {
            return SubstrateMode.valueOf(modeString.toUpperCase(java.util.Locale.ROOT));
        } catch (IllegalArgumentException e) {
            return null; // invalid value — fall back to per-service flags
        }
    }

    /**
     * Check whether a subsystem is enabled, considering both umbrella mode and per-service flags.
     *
     * <ul>
     *   <li>If SUBSTRATE_MODE is "off" → false</li>
     *   <li>If SUBSTRATE_MODE is "shadow" or "authoritative" → true</li>
     *   <li>If SUBSTRATE_MODE is not set → check the per-service flag</li>
     * </ul>
     */
    public static boolean isSubsystemEnabled(InternalOptions options, InternalOption<Boolean> perServiceFlag) {
        SubstrateMode mode = getMode(options);
        if (mode != null) {
            return mode != SubstrateMode.OFF;
        }
        // Fall back to per-service flag
        return options.getBoolean(perServiceFlag);
    }

    /**
     * Check whether the substrate is in authoritative mode.
     *
     * <ul>
     *   <li>If SUBSTRATE_MODE is "authoritative" → true</li>
     *   <li>Otherwise → check ENABLE_AUTHORITATIVE_EXECUTION flag</li>
     * </ul>
     */
    public static boolean isAuthoritative(InternalOptions options) {
        SubstrateMode mode = getMode(options);
        if (mode == SubstrateMode.AUTHORITATIVE) {
            return true;
        }
        if (mode != null) {
            return false; // shadow or off
        }
        // Fall back to per-service flag
        return options.getBoolean(ENABLE_AUTHORITATIVE_EXECUTION);
    }

    /**
     * Check whether a specific subsystem is in authoritative mode, considering both
     * umbrella mode and the per-subsystem authoritative flag.
     *
     * <ul>
     *   <li>If SUBSTRATE_MODE is "authoritative" → true</li>
     *   <li>If SUBSTRATE_MODE is "shadow" or not set → check the per-service authoritative flag</li>
     *   <li>If SUBSTRATE_MODE is "off" → false</li>
     * </ul>
     */
    public static boolean isSubsystemAuthoritative(InternalOptions options, InternalOption<Boolean> perServiceAuthFlag) {
        SubstrateMode mode = getMode(options);
        if (mode == SubstrateMode.AUTHORITATIVE) {
            return true;
        }
        if (mode == SubstrateMode.OFF) {
            return false;
        }
        // shadow mode or not set — fall back to per-service authoritative flag
        return options.getBoolean(perServiceAuthFlag);
    }

    /**
     * Check whether the substrate is in shadow mode (both Java and Rust run).
     *
     * <ul>
     *   <li>If SUBSTRATE_MODE is "shadow" → true</li>
     *   <li>Otherwise → check SHADOW_HASHING flag</li>
     * </ul>
     */
    public static boolean isShadowMode(InternalOptions options) {
        SubstrateMode mode = getMode(options);
        if (mode == SubstrateMode.SHADOW) {
            return true;
        }
        if (mode != null) {
            return false; // authoritative or off
        }
        // Fall back to per-service flag
        return options.getBoolean(SHADOW_HASHING);
    }

    /**
     * Check whether the master substrate switch is on.
     * Returns true if SUBSTRATE_MODE is shadow/authoritative, or if ENABLE_SUBSTRATE flag is true.
     */
    public static boolean isSubstrateEnabled(InternalOptions options) {
        SubstrateMode mode = getMode(options);
        if (mode != null) {
            return mode != SubstrateMode.OFF;
        }
        return options.getBoolean(ENABLE_SUBSTRATE);
    }

    private RustSubstrateOptions() {
        // utility class
    }
}
