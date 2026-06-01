package org.gradle.internal.buildoption;

import java.util.Locale;

/**
 * Feature flags for the Rust execution substrate.
 */
public final class RustSubstrateOptions {

    public enum SubstrateMode {
        OFF,
        SHADOW,
        AUTHORITATIVE
    }

    public enum ExecutionKernelAdmission {
        OFF,
        NATIVE_READY_DEFAULT,
        STRICT
    }

    public static final InternalOption<String> SUBSTRATE_MODE =
        InternalOptions.ofString("org.gradle.rust.substrate.mode", "");

    public static final InternalOption<Boolean> ENABLE_SUBSTRATE = flag("org.gradle.rust.substrate.enabled");
    public static final InternalOption<Boolean> ENABLE_AUTHORITATIVE_EXECUTION = flag("org.gradle.rust.substrate.execution.authoritative");
    public static final InternalOption<Boolean> ENABLE_ADVISORY_EXECUTION = flag("org.gradle.rust.substrate.execution.advisory");
    public static final InternalOption<Boolean> ENABLE_JVM_HOST = flag("org.gradle.rust.substrate.jvm.host.enabled");

    public static final InternalOption<String> DAEMON_BINARY_PATH =
        InternalOptions.ofString("org.gradle.rust.substrate.daemon.path", "");
    public static final InternalOption<String> STATE_DIRECTORY =
        InternalOptions.ofString("org.gradle.rust.substrate.state.dir", "");

    public static final InternalOption<Boolean> REPORT_MISMATCHES =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.shadow.report-mismatches", true);
    public static final InternalOption<Boolean> SHADOW_HASHING =
        InternalOptions.ofBoolean("org.gradle.rust.substrate.hashing.shadow", true);

    public static final InternalOption<Boolean> ENABLE_RUST_HASHING = flag("org.gradle.rust.substrate.hashing.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_CACHE = flag("org.gradle.rust.substrate.cache.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_EXEC = flag("org.gradle.rust.substrate.exec.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_HISTORY = flag("org.gradle.rust.substrate.history.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_FINGERPRINTING = flag("org.gradle.rust.substrate.fingerprint.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_SNAPSHOTTING = flag("org.gradle.rust.substrate.snapshot.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_TASK_GRAPH = flag("org.gradle.rust.substrate.taskgraph.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_CONFIGURATION = flag("org.gradle.rust.substrate.configuration.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_PLUGIN = flag("org.gradle.rust.substrate.plugin.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_OPS = flag("org.gradle.rust.substrate.buildops.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_BOOTSTRAP = flag("org.gradle.rust.substrate.bootstrap.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_DEPENDENCY_RESOLUTION = flag("org.gradle.rust.substrate.dependency.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_DEPENDENCY_ARTIFACT_MIRROR = flag("org.gradle.rust.substrate.dependency.mirror.artifacts");
    public static final InternalOption<Boolean> ENABLE_RUST_DEPENDENCY_ARTIFACT_PREFETCH = flag("org.gradle.rust.substrate.dependency.prefetch.artifacts");
    public static final InternalOption<Boolean> ENABLE_RUST_DEPENDENCY_ARTIFACT_READ_THROUGH = flag("org.gradle.rust.substrate.dependency.readthrough.artifacts");
    public static final InternalOption<Boolean> ENABLE_RUST_DEPENDENCY_METADATA_READ_THROUGH = flag("org.gradle.rust.substrate.dependency.readthrough.metadata");
    public static final InternalOption<Boolean> ENABLE_RUST_DEPENDENCY_RESOURCE_DOWNLOAD = flag("org.gradle.rust.substrate.dependency.download.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_FILE_WATCH = flag("org.gradle.rust.substrate.filewatch.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_CONFIG_CACHE = flag("org.gradle.rust.substrate.configcache.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_TOOLCHAIN = flag("org.gradle.rust.substrate.toolchain.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_EVENT_STREAM = flag("org.gradle.rust.substrate.eventstream.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_WORKER_PROCESS = flag("org.gradle.rust.substrate.worker.process.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_LAYOUT = flag("org.gradle.rust.substrate.layout.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_RESULT = flag("org.gradle.rust.substrate.result.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_PROBLEMS = flag("org.gradle.rust.substrate.problems.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_RESOURCES = flag("org.gradle.rust.substrate.resources.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_COMPARISON = flag("org.gradle.rust.substrate.comparison.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_CONSOLE = flag("org.gradle.rust.substrate.console.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_TEST_EXECUTION = flag("org.gradle.rust.substrate.testexec.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_PUBLISHING = flag("org.gradle.rust.substrate.publishing.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_INIT = flag("org.gradle.rust.substrate.init.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_INCREMENTAL = flag("org.gradle.rust.substrate.incremental.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_METRICS = flag("org.gradle.rust.substrate.metrics.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_GC = flag("org.gradle.rust.substrate.gc.enabled");

    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_HASHING = flag("org.gradle.rust.substrate.hashing.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_CACHE = flag("org.gradle.rust.substrate.cache.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_CONFIG_CACHE = flag("org.gradle.rust.substrate.configcache.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_FINGERPRINTING = flag("org.gradle.rust.substrate.fingerprint.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_SNAPSHOTTING = flag("org.gradle.rust.substrate.snapshot.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_TASK_GRAPH = flag("org.gradle.rust.substrate.taskgraph.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_EXECUTION_PLAN = flag("org.gradle.rust.substrate.executionplan.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_EXEC = flag("org.gradle.rust.substrate.exec.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_FILE_WATCH = flag("org.gradle.rust.substrate.filewatch.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_DEPENDENCY_RESOLUTION = flag("org.gradle.rust.substrate.dependency.authoritative");

    public static final InternalOption<Boolean> ENABLE_RUST_RUN_BUILD = flag("org.gradle.rust.substrate.runbuild.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_RUN_BUILD = flag("org.gradle.rust.substrate.runbuild.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_NATIVE_READY_DEFAULT_RUN_BUILD = flag("org.gradle.rust.substrate.runbuild.native-ready-default");
    public static final InternalOption<Boolean> ENABLE_RUST_EXECUTION_KERNEL = flag("org.gradle.rust.substrate.execution.kernel");

    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_TASK_EXECUTION_LOWERING = flag("org.gradle.rust.substrate.task.execution.lowering.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_WORKER_PROCESS = flag("org.gradle.rust.substrate.worker.process.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_PLAN_SHADOW = flag("org.gradle.rust.substrate.build.plan.shadow.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_SCRIPT_LOWERING = flag("org.gradle.rust.substrate.build.script.lowering.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_CACHE_PACKAGING = flag("org.gradle.rust.substrate.cache.packaging.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_CC_DURABLE_V2 = flag("org.gradle.rust.substrate.cc.durable.v2.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_DAG_EXECUTOR = flag("org.gradle.rust.substrate.dag.executor.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_DEPENDENCY_METADATA = flag("org.gradle.rust.substrate.dependency.metadata.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_DEPENDENCY_METADATA_CACHE = flag("org.gradle.rust.substrate.dependency.metadata.cache.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_EXECUTION_HISTORY = flag("org.gradle.rust.substrate.execution.history.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_FULL_DEP_GRAPH = flag("org.gradle.rust.substrate.full.dep.graph.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_GC_INTEGRITY = flag("org.gradle.rust.substrate.gc.integrity.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_INCREMENTAL_COMPILATION = flag("org.gradle.rust.substrate.incremental.compilation.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_PARALLEL_SCHEDULER_WORKSTEAL = flag("org.gradle.rust.substrate.parallel.scheduler.worksteal.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_PERSISTENT_CACHE = flag("org.gradle.rust.substrate.persistent.cache.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_REMOTE_CACHE = flag("org.gradle.rust.substrate.remote_cache.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_TASK_EXECUTION_LOWERING = flag("org.gradle.rust.substrate.task.execution.lowering.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_TEST_EXEC = flag("org.gradle.rust.substrate.test.exec.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_VFS_HIERARCHY = flag("org.gradle.rust.substrate.vfs.hierarchy.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_WORKER_PROCESS_LEASE_HEARTBEAT = flag("org.gradle.rust.substrate.worker.process.lease.heartbeat.enabled");

    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_BUILD_PLAN_SHADOW = flag("org.gradle.rust.substrate.build.plan.shadow.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_DEPENDENCY_METADATA = flag("org.gradle.rust.substrate.dependency.metadata.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_EXECUTION_HISTORY = flag("org.gradle.rust.substrate.execution.history.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_FILE_HASH_CACHE = flag("org.gradle.rust.substrate.file.hash.cache.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_HISTORY = flag("org.gradle.rust.substrate.history.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_INCREMENTAL_COMPILATION = flag("org.gradle.rust.substrate.incremental.compilation.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_PUBLISHING = flag("org.gradle.rust.substrate.publishing.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_PUBLISHING_UPLOAD = flag("org.gradle.rust.substrate.publishing.upload.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_REMOTE_CACHE = flag("org.gradle.rust.substrate.remote_cache.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_RESOLVED_GRAPH = flag("org.gradle.rust.substrate.dependency.resolved-graph.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_VFS_HIERARCHY = flag("org.gradle.rust.substrate.vfs.hierarchy.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_AUTHORITATIVE_WORKERS = flag("org.gradle.rust.substrate.workers.authoritative");

    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_SCRIPT = flag("org.gradle.rust.substrate.build.script.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_SCRIPT_AUTHORITATIVE = flag("org.gradle.rust.substrate.build.script.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_SCRIPT_JAVA_COMPILE_LOWERING = flag("org.gradle.rust.substrate.build.script.java.compile.lowering.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_BUILD_SCRIPT_JAVA_COMPILE_PERPETUAL_019E6B6152A1 = flag("org.gradle.rust.substrate.build.script.java.compile.perpetual.019e6b6152a1.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_FILE_FINGERPRINT = flag("org.gradle.rust.substrate.file.fingerprint.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_FILE_HASH_CACHE = flag("org.gradle.rust.substrate.file.hash.cache.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_FULL_DEP_GRAPH_AUTHORITATIVE = flag("org.gradle.rust.substrate.full.dep.graph.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_FULL_INCREMENTAL = flag("org.gradle.rust.substrate.full.incremental.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_HYGIENE_COMPANION_5 = flag("org.gradle.rust.substrate.hygiene.companion.5.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_INCREMENTAL_FULL = flag("org.gradle.rust.substrate.incremental.full.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_INTEGRITY = flag("org.gradle.rust.substrate.integrity.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_KERNEL = flag("org.gradle.rust.substrate.kernel.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_KERNEL_AUTHORITATIVE = flag("org.gradle.rust.substrate.kernel.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_NATIVE_COMPILE = flag("org.gradle.rust.substrate.native.compile.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_POSTGREEN_DUALHYGIENE = flag("org.gradle.rust.substrate.postgreen.dualhygiene.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_PROBLEM_REPORTING = flag("org.gradle.rust.substrate.problem.reporting.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_PUBLISHING_UPLOAD = flag("org.gradle.rust.substrate.publishing.upload.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_QUINTUPLE_MEGA = flag("org.gradle.rust.substrate.quintuple.mega.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_RESOLVED_GRAPH = flag("org.gradle.rust.substrate.resolved.graph.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_TEST_EXEC_LOWERING = flag("org.gradle.rust.substrate.test.exec.lowering.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_VFS_DELTA = flag("org.gradle.rust.substrate.vfs.delta.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_VFS_INCREMENTAL_CROSS = flag("org.gradle.rust.substrate.vfs.incremental.cross.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_VFS_NATIVE_CROSS = flag("org.gradle.rust.substrate.vfs.native.cross.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_VFS_SNAPSHOT = flag("org.gradle.rust.substrate.vfs.snapshot.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_VFS_SNAPSHOT_TRANSFER_AUTHORITATIVE = flag("org.gradle.rust.substrate.vfs.snapshot.transfer.authoritative");
    public static final InternalOption<Boolean> ENABLE_RUST_WORKERS = flag("org.gradle.rust.substrate.workers.enabled");

    public static final InternalOption<Boolean> ENABLE_RUST_TAR_SYNC_LOWERING = flag("org.gradle.rust.substrate.tar.sync.lowering.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_SYMLINK_MKDIR_LOWERING = flag("org.gradle.rust.substrate.symlink.mkdir.lowering.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_DELETE_STARTSCRIPTS_LOWERING = flag("org.gradle.rust.substrate.delete.startscripts.lowering.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_JAR_LOWERING = flag("org.gradle.rust.substrate.jar.lowering.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_JAVA_COMPILE_LOWERING = flag("org.gradle.rust.substrate.java_compile_lowering.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_KERNEL_ADMISSION_GATE = flag("org.gradle.rust.substrate.kernel.admission.enabled");
    public static final InternalOption<Boolean> ENABLE_RUST_DEEPER_KERNEL_VFS_CROSS_92AT = flag("org.gradle.rust.substrate.deeper.kernel.vfs.cross.enabled");

    public static SubstrateMode getMode(InternalOptions options) {
        String modeString = options == null
            ? System.getProperty(SUBSTRATE_MODE.getPropertyName(), "")
            : options.getOptionValue(SUBSTRATE_MODE).get();
        if (modeString == null || modeString.trim().isEmpty()) {
            return null;
        }
        try {
            return SubstrateMode.valueOf(modeString.trim().toUpperCase(Locale.ROOT));
        } catch (IllegalArgumentException e) {
            return null;
        }
    }

    public static boolean isSubsystemEnabled(InternalOptions options, InternalOption<Boolean> perServiceFlag) {
        SubstrateMode mode = getMode(options);
        if (mode != null) {
            return mode != SubstrateMode.OFF;
        }
        return options == null
            ? Boolean.getBoolean(perServiceFlag.getPropertyName())
            : options.getBoolean(perServiceFlag);
    }

    public static boolean isSubsystemEnabled(InternalOptions options, String propertyName) {
        SubstrateMode mode = getMode(options);
        if (mode != null) {
            return mode != SubstrateMode.OFF;
        }
        return options == null ? Boolean.getBoolean(propertyName) : options.getBoolean(propertyName, false);
    }

    public static boolean isAuthoritative(InternalOptions options) {
        SubstrateMode mode = getMode(options);
        if (mode == SubstrateMode.AUTHORITATIVE) {
            return true;
        }
        if (mode != null) {
            return false;
        }
        return options == null
            ? Boolean.getBoolean(ENABLE_AUTHORITATIVE_EXECUTION.getPropertyName())
            : options.getBoolean(ENABLE_AUTHORITATIVE_EXECUTION);
    }

    public static boolean isSubsystemAuthoritative(InternalOptions options, InternalOption<Boolean> perServiceAuthFlag) {
        SubstrateMode mode = getMode(options);
        if (mode == SubstrateMode.AUTHORITATIVE) {
            return true;
        }
        if (mode == SubstrateMode.OFF) {
            return false;
        }
        return options == null
            ? Boolean.getBoolean(perServiceAuthFlag.getPropertyName())
            : options.getBoolean(perServiceAuthFlag);
    }

    public static boolean isShadowMode(InternalOptions options) {
        SubstrateMode mode = getMode(options);
        if (mode == SubstrateMode.SHADOW) {
            return true;
        }
        if (mode != null) {
            return false;
        }
        return options == null ? Boolean.getBoolean(SHADOW_HASHING.getPropertyName()) : options.getBoolean(SHADOW_HASHING);
    }

    public static boolean isSubstrateEnabled(InternalOptions options) {
        SubstrateMode mode = getMode(options);
        if (mode != null) {
            return mode != SubstrateMode.OFF;
        }
        return options == null
            ? Boolean.getBoolean(ENABLE_SUBSTRATE.getPropertyName())
            : options.getBoolean(ENABLE_SUBSTRATE);
    }

    public static boolean isExecutionKernelEnabled(InternalOptions options) {
        return getExecutionKernelAdmission(options) == ExecutionKernelAdmission.STRICT;
    }

    public static ExecutionKernelAdmission getExecutionKernelAdmission(InternalOptions options) {
        SubstrateMode mode = getMode(options);
        if (mode == SubstrateMode.AUTHORITATIVE) {
            return ExecutionKernelAdmission.STRICT;
        }
        if (mode == SubstrateMode.OFF) {
            return ExecutionKernelAdmission.OFF;
        }
        if (!isSubstrateEnabled(options)) {
            return ExecutionKernelAdmission.OFF;
        }
        if (isSubsystemEnabled(options, ENABLE_RUST_EXECUTION_KERNEL)
            || isSubsystemEnabled(options, ENABLE_RUST_AUTHORITATIVE_RUN_BUILD)) {
            return ExecutionKernelAdmission.STRICT;
        }
        if (isSubsystemEnabled(options, ENABLE_RUST_NATIVE_READY_DEFAULT_RUN_BUILD)) {
            return ExecutionKernelAdmission.NATIVE_READY_DEFAULT;
        }
        return ExecutionKernelAdmission.OFF;
    }

    public static boolean isExecutionKernelRequested(InternalOptions options) {
        return getExecutionKernelAdmission(options) != ExecutionKernelAdmission.OFF;
    }

    private static InternalOption<Boolean> flag(String propertyName) {
        return InternalOptions.ofBoolean(propertyName, false);
    }

    private RustSubstrateOptions() {
    }
}
