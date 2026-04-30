package org.gradle.internal.rustbridge;

import org.gradle.api.logging.Logging;
import org.gradle.internal.buildoption.InternalOptions;
import org.gradle.internal.buildoption.RustArtifactCacheReadThroughRegistry;
import org.gradle.internal.buildoption.RustMetadataCacheReadThroughRegistry;
import org.gradle.internal.buildoption.RustSubstrateOptions;
import org.gradle.internal.event.ListenerManager;
import org.gradle.internal.rustbridge.bootstrap.BootstrapLifecycleListener;
import org.gradle.internal.rustbridge.bootstrap.RustBootstrapClient;
import org.gradle.internal.rustbridge.cache.BuildCacheOrchestrationClient;
import org.gradle.internal.rustbridge.buildresult.BuildResultShadowListener;
import org.gradle.internal.rustbridge.buildresult.RustBuildResultClient;
import org.gradle.internal.rustbridge.configcache.ConfigurationCacheShadowListener;
import org.gradle.internal.rustbridge.configcache.RustConfigCacheClient;
import org.gradle.internal.rustbridge.dependency.DependencyResolutionShadowListener;
import org.gradle.internal.rustbridge.dependency.RustArtifactCacheReadThrough;
import org.gradle.internal.rustbridge.dependency.RustDependencyResolutionClient;
import org.gradle.internal.rustbridge.dependency.RustMetadataCacheReadThrough;
import org.gradle.internal.rustbridge.history.RustExecutionHistoryClient;
import org.gradle.internal.rustbridge.jvmhost.BuildPlanTaskSelectionSnapshot;
import org.gradle.internal.rustbridge.jvmhost.JvmHostServiceImpl;
import org.gradle.internal.rustbridge.jvmhost.JvmTaskExecutionProviderAdapter;
import org.gradle.internal.rustbridge.jvmhost.ProjectModelProviderAdapter;
import org.gradle.internal.rustbridge.metrics.RustBuildMetricsClient;
import org.gradle.internal.rustbridge.shadow.BuildFinishMismatchLogger;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.rustbridge.shadow.ShadowingBuildCacheKeyComputer;
import org.gradle.internal.rustbridge.taskgraph.RustBuildExecutionClient;
import org.gradle.internal.rustbridge.taskgraph.RustTaskGraphClient;
import org.gradle.internal.rustbridge.taskgraph.TaskGraphShadowListener;
import org.gradle.internal.rustbridge.taskgraph.TaskGraphShadowReporter;
import org.gradle.internal.rustbridge.testexec.TestExecutionShadowListener;
import org.gradle.internal.service.PrivateService;
import org.gradle.internal.service.Provides;
import org.gradle.internal.service.ServiceRegistration;
import org.gradle.internal.service.ServiceRegistrationProvider;
import org.gradle.internal.service.ServiceRegistry;
import org.gradle.internal.service.scopes.AbstractGradleModuleServices;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.io.File;

/**
 * Minimal compile-safe service wiring for the Rust bridge.
 *
 * <p>This intentionally avoids heavyweight launcher/compat classes that are still
 * excluded from this module's compile, while activating the Rust paths that are
 * already stable in shadow mode or fail-closed authoritative mode.</p>
 */
public class RustBridgeCoreServices extends AbstractGradleModuleServices {

    private static final Logger LOGGER = Logging.getLogger(RustBridgeCoreServices.class);

    @Override
    public void registerGlobalServices(ServiceRegistration registration) {
    }

    @Override
    public void registerGradleUserHomeServices(ServiceRegistration registration) {
    }

    @Override
    public void registerBuildSessionServices(ServiceRegistration registration) {
        registration.addProvider(new BuildSessionServices());
    }

    @Override
    public void registerBuildServices(ServiceRegistration registration) {
        registration.addProvider(new BuildServices());
    }

    private static class BuildSessionServices implements ServiceRegistrationProvider {
        @Provides
        DaemonLauncher createDaemonLauncher(InternalOptions options) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return DaemonLauncher.noop();
            }
            String binaryPath = options.getValue(RustSubstrateOptions.DAEMON_BINARY_PATH);
            File daemonBinary;
            if (binaryPath.isEmpty()) {
                String javaHome = System.getProperty("java.home");
                File installDir = new File(javaHome).getParentFile();
                if (installDir == null) {
                    return DaemonLauncher.noop("daemon-install-dir-unavailable:" + javaHome);
                }
                daemonBinary = DaemonLauncher.resolveBinary(installDir);
            } else {
                daemonBinary = new File(binaryPath);
            }

            File socketDirectory = new File(System.getProperty("user.home"), ".gradle-substrate");
            boolean enableJvmHost = RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_JVM_HOST);
            return enableJvmHost
                ? DaemonLauncher.withJvmHost(daemonBinary, socketDirectory)
                : DaemonLauncher.of(daemonBinary, socketDirectory);
        }

        @Provides
        SubstrateClient createSubstrateClient(DaemonLauncher launcher, InternalOptions options) {
            SubstrateClient client;
            try {
                client = launcher.launchOrConnect();
            } catch (Exception e) {
                client = unavailableClient(options, "daemon-launch-or-connect-failed:" + e.getMessage(), e);
            }
            if (RustSubstrateOptions.isSubstrateEnabled(options) && client.isNoop()) {
                if (RustSubstrateOptions.isAuthoritative(options)) {
                    throw new SubstrateException("Rust substrate is authoritative but unavailable: " + client.getNoopReason());
                }
                LOGGER.warn(
                    "[substrate] Rust substrate is enabled but bridge is running in no-op fallback mode: {}",
                    client.getNoopReason()
                );
            }
            return client;
        }

        static SubstrateClient unavailableClient(InternalOptions options, String reason, @Nullable Throwable cause) {
            if (RustSubstrateOptions.isAuthoritative(options)) {
                throw cause == null
                    ? new SubstrateException("Rust substrate is authoritative but unavailable: " + reason)
                    : new SubstrateException("Rust substrate is authoritative but unavailable: " + reason, cause);
            }
            LOGGER.warn("[substrate] bridge is running in no-op fallback mode: {}", reason, cause);
            return SubstrateClient.noop(reason);
        }

        @Provides
        HashMismatchReporter createHashMismatchReporter() {
            return new HashMismatchReporter(true);
        }
    }

    private static class BuildServices implements ServiceRegistrationProvider {
        @Provides
        RustBootstrapClient createRustBootstrapClient(
            SubstrateClient client,
            JvmHostBridgeWiring jvmHostBridgeWiring
        ) {
            if (jvmHostBridgeWiring == null) {
                throw new IllegalStateException("JVM host bridge wiring service was not initialized");
            }
            return new RustBootstrapClient(client);
        }

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
        RustBuildResultClient createRustBuildResultClient(SubstrateClient client) {
            return new RustBuildResultClient(client);
        }

        @Provides
        RustBuildMetricsClient createRustBuildMetricsClient(SubstrateClient client) {
            return new RustBuildMetricsClient(client);
        }

        @Provides
        RustExecutionHistoryClient createRustExecutionHistoryClient(SubstrateClient client) {
            return new RustExecutionHistoryClient(client);
        }

        @Provides
        RustTaskGraphClient createRustTaskGraphClient(SubstrateClient client) {
            return new RustTaskGraphClient(client);
        }

        @Provides
        RustBuildExecutionClient createRustBuildExecutionClient(SubstrateClient client) {
            return new RustBuildExecutionClient(client);
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
            configureDependencyReadThrough(rustDependencyResolutionClient, options);
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_DEPENDENCY_RESOLUTION)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_DEPENDENCY_RESOLUTION
            );
            boolean mirrorArtifacts = options.getBoolean(RustSubstrateOptions.ENABLE_RUST_DEPENDENCY_ARTIFACT_MIRROR);
            DependencyResolutionShadowListener listener =
                new DependencyResolutionShadowListener(rustDependencyResolutionClient, mismatchReporter, authoritative, mirrorArtifacts);
            listenerManager.addListener(listener);
            return listener;
        }

        private static void configureDependencyReadThrough(
            RustDependencyResolutionClient rustDependencyResolutionClient,
            InternalOptions options
        ) {
            boolean dependencyEnabled = RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_DEPENDENCY_RESOLUTION);
            if (dependencyEnabled && options.getBoolean(RustSubstrateOptions.ENABLE_RUST_DEPENDENCY_ARTIFACT_READ_THROUGH)) {
                RustArtifactCacheReadThroughRegistry.set(new RustArtifactCacheReadThrough(rustDependencyResolutionClient));
            } else {
                RustArtifactCacheReadThroughRegistry.reset();
            }
            if (dependencyEnabled && options.getBoolean(RustSubstrateOptions.ENABLE_RUST_DEPENDENCY_METADATA_READ_THROUGH)) {
                RustMetadataCacheReadThroughRegistry.set(new RustMetadataCacheReadThrough(rustDependencyResolutionClient));
            } else {
                RustMetadataCacheReadThroughRegistry.reset();
            }
        }

        @Provides
        @PrivateService
        JvmHostBridgeWiring createJvmHostBridgeWiring(
            DaemonLauncher daemonLauncher,
            BuildPlanTaskSelectionSnapshot taskSelectionSnapshot,
            ServiceRegistry services,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return JvmHostBridgeWiring.INSTANCE;
            }
            JvmHostServiceImpl serviceImpl = daemonLauncher.getJvmHostServiceImpl();
            if (serviceImpl != null) {
                serviceImpl.setProjectModelProvider(ProjectModelProviderAdapter.fromServiceRegistry(services));
                serviceImpl.setTaskSelectionSnapshot(taskSelectionSnapshot);
                serviceImpl.setTaskExecutionProvider(JvmTaskExecutionProviderAdapter.fromServiceRegistry(services));
                return JvmHostBridgeWiring.INSTANCE;
            }
            return JvmHostBridgeWiring.INSTANCE;
        }

        @Provides
        @Nullable
        TaskGraphShadowListener createTaskGraphShadowListener(
            RustTaskGraphClient rustTaskGraphClient,
            RustBuildExecutionClient rustBuildExecutionClient,
            RustBootstrapClient bootstrapClient,
            JvmHostBridgeWiring jvmHostBridgeWiring,
            BuildPlanTaskSelectionSnapshot taskSelectionSnapshot,
            HashMismatchReporter mismatchReporter,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (jvmHostBridgeWiring == null) {
                throw new IllegalStateException("JVM host bridge wiring service was not initialized");
            }
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            RustTaskGraphClient activeClient = RustSubstrateOptions.isSubsystemEnabled(
                options,
                RustSubstrateOptions.ENABLE_RUST_TASK_GRAPH
            ) ? rustTaskGraphClient : null;
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_TASK_GRAPH
            );
            boolean runBuildEnabled = RustSubstrateOptions.isSubsystemEnabled(
                options,
                RustSubstrateOptions.ENABLE_RUST_RUN_BUILD
            );
            boolean runBuildAuthoritative = options.getBoolean(RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_RUN_BUILD);
            TaskGraphShadowReporter reporter = new TaskGraphShadowReporter(
                activeClient,
                runBuildEnabled && !runBuildAuthoritative ? rustBuildExecutionClient : null,
                mismatchReporter,
                authoritative,
                runBuildEnabled && !runBuildAuthoritative,
                runBuildAuthoritative
            );
            TaskGraphShadowListener listener = new TaskGraphShadowListener(
                reporter,
                taskSelectionSnapshot,
                bootstrapClient
            );
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        @Nullable
        BuildResultShadowListener createBuildResultShadowListener(
            RustBuildResultClient rustBuildResultClient,
            RustBuildMetricsClient rustBuildMetricsClient,
            RustExecutionHistoryClient rustExecutionHistoryClient,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            BuildResultShadowListener listener = new BuildResultShadowListener(
                rustBuildResultClient,
                rustBuildMetricsClient,
                rustExecutionHistoryClient
            );
            RootBuildLifecycleBridge.register(listenerManager, listener);
            return listener;
        }

        @Provides
        BuildFinishMismatchLogger createBuildFinishMismatchLogger(
            HashMismatchReporter reporter,
            ListenerManager listenerManager
        ) {
            BuildFinishMismatchLogger logger = new BuildFinishMismatchLogger(reporter);
            RootBuildLifecycleBridge.register(listenerManager, logger);
            return logger;
        }

        @Provides
        @Nullable
        TestExecutionShadowListener createTestExecutionShadowListener(
            SubstrateClient client,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_TEST_EXECUTION)) {
                return null;
            }
            TestExecutionShadowListener listener = new TestExecutionShadowListener(client);
            Object proxy = listener.asListenerProxy();
            if (proxy == null) {
                LOGGER.warn("[substrate:testexec] runtime TestListener API unavailable; test execution shadowing disabled");
                return null;
            }
            listenerManager.addListener(proxy);
            return listener;
        }

        @Provides
        @Nullable
        BootstrapLifecycleListener createBootstrapLifecycleListener(
            RustBootstrapClient bootstrapClient,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            int parallelism = Runtime.getRuntime().availableProcessors();
            String projectDir = System.getProperty("user.dir", ".");
            BootstrapLifecycleListener listener = new BootstrapLifecycleListener(
                bootstrapClient,
                projectDir,
                parallelism
            );
            RootBuildLifecycleBridge.register(listenerManager, listener);
            return listener;
        }
    }

    @org.gradle.internal.service.scopes.ServiceScope(org.gradle.internal.service.scopes.Scope.Build.class)
    private static final class JvmHostBridgeWiring {
        private static final JvmHostBridgeWiring INSTANCE = new JvmHostBridgeWiring();

        private JvmHostBridgeWiring() {
        }
    }
}
