package org.gradle.internal.rustbridge;

import org.gradle.internal.buildoption.InternalOptions;
import org.gradle.internal.buildoption.RustSubstrateOptions;
import org.gradle.internal.rustbridge.cache.BuildCacheOrchestrationClient;
import org.gradle.internal.rustbridge.cache.RustBuildCacheServiceFactory;
import org.gradle.internal.rustbridge.cache.RustRemoteBuildCacheServiceFactory;
import org.gradle.internal.event.ListenerManager;
import org.gradle.internal.rustbridge.shadow.BuildFinishMismatchLogger;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.rustbridge.shadow.ShadowingBuildCacheKeyComputer;
import org.gradle.internal.rustbridge.configuration.PropertyShadowEvaluationListener;
import org.gradle.internal.rustbridge.configuration.RustConfigurationClient;
import org.gradle.internal.rustbridge.configuration.ShadowingPropertyResolver;
import org.gradle.internal.rustbridge.execution.ExecutionPlanClient;
import org.gradle.internal.rustbridge.fingerprint.RustFileFingerprintClient;
import org.gradle.internal.rustbridge.history.RustExecutionHistoryClient;
import org.gradle.internal.rustbridge.snapshot.RustValueSnapshotClient;
import org.gradle.internal.rustbridge.snapshot.ShadowingValueSnapshotter;
import org.gradle.internal.rustbridge.snapshot.SnapshotHashDelegate;
import org.gradle.internal.rustbridge.taskgraph.RustTaskGraphClient;
import org.gradle.internal.rustbridge.taskgraph.TaskGraphShadowListener;
import org.gradle.internal.rustbridge.taskgraph.TaskGraphShadowReporter;
import org.gradle.internal.rustbridge.watch.RustFileWatchClient;
import org.gradle.internal.rustbridge.work.WorkerSchedulerClient;
import org.gradle.internal.rustbridge.bootstrap.RustBootstrapClient;
import org.gradle.internal.rustbridge.bootstrap.BootstrapLifecycleListener;
import org.gradle.internal.rustbridge.configcache.RustConfigCacheClient;
import org.gradle.internal.rustbridge.worker.RustWorkerProcessClient;
import org.gradle.internal.rustbridge.publishing.RustArtifactPublishingClient;
import org.gradle.internal.rustbridge.dependency.RustDependencyResolutionClient;
import org.gradle.internal.rustbridge.dependency.DependencyResolutionShadowListener;
import org.gradle.internal.rustbridge.buildops.BuildOperationShadowListener;
import org.gradle.internal.rustbridge.buildresult.BuildResultShadowListener;
import org.gradle.internal.rustbridge.transform.TransformExecutionShadowListener;
import org.gradle.internal.rustbridge.output.OutputChangeShadowListener;
import org.gradle.internal.rustbridge.testexec.TestExecutionShadowListener;
import org.gradle.internal.operations.BuildOperationListenerManager;
import org.gradle.internal.rustbridge.plugin.RustPluginClient;
import org.gradle.internal.rustbridge.buildops.RustBuildOperationsClient;
import org.gradle.internal.rustbridge.toolchain.RustToolchainServiceClient;
import org.gradle.internal.rustbridge.eventstream.RustBuildEventStreamClient;
import org.gradle.internal.rustbridge.buildlayout.RustBuildLayoutClient;
import org.gradle.internal.rustbridge.buildresult.RustBuildResultClient;
import org.gradle.internal.rustbridge.problems.RustProblemReportingClient;
import org.gradle.internal.rustbridge.resources.RustResourceManagementClient;
import org.gradle.internal.rustbridge.comparison.RustBuildComparisonClient;
import org.gradle.internal.rustbridge.console.RustConsoleClient;
import org.gradle.internal.rustbridge.testexec.RustTestExecutionClient;
import org.gradle.internal.rustbridge.buildinit.RustBuildInitClient;
import org.gradle.internal.rustbridge.incremental.RustIncrementalCompilationClient;
import org.gradle.internal.rustbridge.metrics.RustBuildMetricsClient;
import org.gradle.internal.rustbridge.gc.RustGarbageCollectionClient;
import org.gradle.internal.service.Provides;
import org.gradle.internal.service.ServiceRegistration;
import org.gradle.internal.service.scopes.AbstractGradleModuleServices;
import org.gradle.internal.snapshot.ValueSnapshotter;
import org.jspecify.annotations.Nullable;

import java.io.File;

/**
 * Registers the Rust substrate bridge services into Gradle's service registry.
 */
public class RustBridgeServices extends AbstractGradleModuleServices {

    @Override
    public void registerGlobalServices(ServiceRegistration registration) {
        registration.addProvider(new GlobalServices());
    }

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

    private static class GlobalServices implements ServiceRegistrationProvider {
        @Provides
        @org.gradle.internal.service.scopes.PrivateService
        DaemonLauncher createDaemonLauncher(InternalOptions options) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_SUBSTRATE).get()) {
                return DaemonLauncher.noop();
            }

            String binaryPath = options.getOption(RustSubstrateOptions.DAEMON_BINARY_PATH).get();
            File daemonBinary;
            if (binaryPath.isEmpty()) {
                String javaHome = System.getProperty("java.home");
                File installDir = new File(javaHome).getParentFile();
                if (installDir == null) {
                    return DaemonLauncher.noop();
                }
                daemonBinary = new File(installDir, "lib/gradle-substrate-daemon");
            } else {
                daemonBinary = new File(binaryPath);
            }

            File socketDirectory = new File(
                System.getProperty("user.home"),
                ".gradle-substrate"
            );

            return DaemonLauncher.of(daemonBinary, socketDirectory);
        }
    }

    private static class UserHomeServices implements ServiceRegistrationProvider {
        @Provides
        SubstrateClient createSubstrateClient(
            DaemonLauncher launcher,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_SUBSTRATE).get()) {
                return SubstrateClient.noop();
            }
            try {
                return launcher.launchOrConnect();
            } catch (Exception e) {
                // Fail-open: fall back to Java implementations
                return SubstrateClient.noop();
            }
        }

        @Provides
        RustBootstrapClient createRustBootstrapClient(SubstrateClient client) {
            return new RustBootstrapClient(client);
        }

        @Provides
        @org.gradle.internal.service.scopes.PrivateService
        BuildOperationShadowListener createBuildOperationShadowListener(
            SubstrateClient client,
            BuildOperationListenerManager buildOpListenerManager,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_SUBSTRATE).get()) {
                return null;
            }
            BuildOperationShadowListener listener = new BuildOperationShadowListener(client);
            buildOpListenerManager.addListener(listener);
            return listener;
        }
    }

    private static class BuildServices implements ServiceRegistrationProvider {
        @Provides
        WorkerSchedulerClient createWorkerSchedulerClient(SubstrateClient client) {
            return new WorkerSchedulerClient(client);
        }

        @Provides
        @org.gradle.internal.service.scopes.PrivateService
        RustBuildCacheServiceFactory createRustBuildCacheServiceFactory(SubstrateClient client) {
            return new RustBuildCacheServiceFactory(client);
        }

        @Provides
        @org.gradle.internal.service.scopes.PrivateService
        RustRemoteBuildCacheServiceFactory createRustRemoteBuildCacheServiceFactory(SubstrateClient client) {
            return new RustRemoteBuildCacheServiceFactory(client);
        }

        @Provides
        ExecutionPlanClient createExecutionPlanClient(SubstrateClient client) {
            return new ExecutionPlanClient(client);
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
        RustConfigurationClient createRustConfigurationClient(SubstrateClient client) {
            return new RustConfigurationClient(client);
        }

        @Provides
        RustFileWatchClient createRustFileWatchClient(SubstrateClient client) {
            return new RustFileWatchClient(client);
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
        RustArtifactPublishingClient createRustArtifactPublishingClient(SubstrateClient client) {
            return new RustArtifactPublishingClient(client);
        }

        @Provides
        RustConfigCacheClient createRustConfigCacheClient(SubstrateClient client) {
            return new RustConfigCacheClient(client);
        }

        @Provides
        RustWorkerProcessClient createRustWorkerProcessClient(SubstrateClient client) {
            return new RustWorkerProcessClient(client);
        }

        @Provides
        RustPluginClient createRustPluginClient(SubstrateClient client) {
            return new RustPluginClient(client);
        }

        @Provides
        RustBuildOperationsClient createRustBuildOperationsClient(SubstrateClient client) {
            return new RustBuildOperationsClient(client);
        }

        @Provides
        RustToolchainServiceClient createRustToolchainServiceClient(SubstrateClient client) {
            return new RustToolchainServiceClient(client);
        }

        @Provides
        RustBuildEventStreamClient createRustBuildEventStreamClient(SubstrateClient client) {
            return new RustBuildEventStreamClient(client);
        }

        @Provides
        RustBuildLayoutClient createRustBuildLayoutClient(SubstrateClient client) {
            return new RustBuildLayoutClient(client);
        }

        @Provides
        RustBuildResultClient createRustBuildResultClient(SubstrateClient client) {
            return new RustBuildResultClient(client);
        }

        @Provides
        RustProblemReportingClient createRustProblemReportingClient(SubstrateClient client) {
            return new RustProblemReportingClient(client);
        }

        @Provides
        RustResourceManagementClient createRustResourceManagementClient(SubstrateClient client) {
            return new RustResourceManagementClient(client);
        }

        @Provides
        RustBuildComparisonClient createRustBuildComparisonClient(SubstrateClient client) {
            return new RustBuildComparisonClient(client);
        }

        @Provides
        RustConsoleClient createRustConsoleClient(SubstrateClient client) {
            return new RustConsoleClient(client);
        }

        @Provides
        RustTestExecutionClient createRustTestExecutionClient(SubstrateClient client) {
            return new RustTestExecutionClient(client);
        }

        @Provides
        RustBuildInitClient createRustBuildInitClient(SubstrateClient client) {
            return new RustBuildInitClient(client);
        }

        @Provides
        RustIncrementalCompilationClient createRustIncrementalCompilationClient(SubstrateClient client) {
            return new RustIncrementalCompilationClient(client);
        }

        @Provides
        RustBuildMetricsClient createRustBuildMetricsClient(SubstrateClient client) {
            return new RustBuildMetricsClient(client);
        }

        @Provides
        RustGarbageCollectionClient createRustGarbageCollectionClient(SubstrateClient client) {
            return new RustGarbageCollectionClient(client);
        }

        @Provides
        @Nullable
        BuildResultShadowListener createBuildResultShadowListener(
            RustBuildResultClient rustBuildResultClient,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_SUBSTRATE).get()) {
                return null;
            }
            BuildResultShadowListener listener = new BuildResultShadowListener(rustBuildResultClient);
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        @Nullable
        DependencyResolutionShadowListener createDependencyResolutionShadowListener(
            RustDependencyResolutionClient rustDependencyResolutionClient,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_RUST_DEPENDENCY_RESOLUTION).get()) {
                return null;
            }
            DependencyResolutionShadowListener listener =
                new DependencyResolutionShadowListener(rustDependencyResolutionClient);
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        @Nullable
        TransformExecutionShadowListener createTransformExecutionShadowListener(
            SubstrateClient client,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_SUBSTRATE).get()) {
                return null;
            }
            // TransformExecutionListener has @ServiceScope + @EventScope,
            // so Gradle auto-dispatches events to it. No manual listener registration needed.
            return new TransformExecutionShadowListener(client);
        }

        @Provides
        @Nullable
        OutputChangeShadowListener createOutputChangeShadowListener(
            SubstrateClient client,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_SUBSTRATE).get()) {
                return null;
            }
            // OutputChangeListener has @ServiceScope + @EventScope,
            // so Gradle auto-dispatches events to it. No manual listener registration needed.
            return new OutputChangeShadowListener(client);
        }

        @Provides
        @Nullable
        TestExecutionShadowListener createTestExecutionShadowListener(
            SubstrateClient client,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_RUST_TEST_EXECUTION).get()) {
                return null;
            }
            // TestListener has @EventScope but NOT @ServiceScope,
            // so we must manually register it via ListenerManager.
            TestExecutionShadowListener listener = new TestExecutionShadowListener(client);
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        BuildFinishMismatchLogger createBuildFinishMismatchLogger(
            HashMismatchReporter reporter,
            ListenerManager listenerManager
        ) {
            BuildFinishMismatchLogger logger = new BuildFinishMismatchLogger(reporter);
            listenerManager.addListener(logger);
            return logger;
        }

        @Provides
        BootstrapLifecycleListener createBootstrapLifecycleListener(
            RustBootstrapClient bootstrapClient,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_SUBSTRATE).get()) {
                return new BootstrapLifecycleListener(bootstrapClient, ".", 1);
            }
            int parallelism = Runtime.getRuntime().availableProcessors();
            String projectDir = System.getProperty("user.dir", ".");
            BootstrapLifecycleListener listener = new BootstrapLifecycleListener(
                bootstrapClient, projectDir, parallelism);
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        ShadowingBuildCacheKeyComputer createShadowingBuildCacheKeyComputer(
            BuildCacheOrchestrationClient cacheOrchestrationClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            boolean authoritative = options.getOption(RustSubstrateOptions.ENABLE_AUTHORITATIVE_EXECUTION).get();
            return new ShadowingBuildCacheKeyComputer(cacheOrchestrationClient, mismatchReporter, authoritative);
        }

        @Provides
        @Nullable
        TaskGraphShadowListener createTaskGraphShadowListener(
            RustTaskGraphClient rustTaskGraphClient,
            HashMismatchReporter mismatchReporter,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_RUST_TASK_GRAPH).get()) {
                return null;
            }
            TaskGraphShadowListener listener = new TaskGraphShadowListener(
                new TaskGraphShadowReporter(rustTaskGraphClient, mismatchReporter));
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        @Nullable
        PropertyShadowEvaluationListener createPropertyShadowEvaluationListener(
            RustConfigurationClient rustConfigurationClient,
            HashMismatchReporter mismatchReporter,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_RUST_CONFIGURATION).get()) {
                return null;
            }
            PropertyShadowEvaluationListener listener = new PropertyShadowEvaluationListener(
                new ShadowingPropertyResolver(rustConfigurationClient, mismatchReporter));
            listenerManager.addListener(listener);
            return listener;
        }
    }

    private static class BuildSessionServices implements ServiceRegistrationProvider {
        @Provides
        RustFileFingerprintClient createRustFileFingerprintClient(SubstrateClient client) {
            return new RustFileFingerprintClient(client);
        }

        @Provides
        RustValueSnapshotClient createRustValueSnapshotClient(SubstrateClient client) {
            return new RustValueSnapshotClient(client);
        }

        @Provides
        HashMismatchReporter createHashMismatchReporter() {
            return new HashMismatchReporter(true);
        }

        @Provides
        @Nullable
        ShadowingValueSnapshotter createShadowingValueSnapshotter(
            ValueSnapshotter valueSnapshotter,
            RustValueSnapshotClient rustValueSnapshotClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!options.getOption(RustSubstrateOptions.ENABLE_RUST_SNAPSHOTTING).get()) {
                return null;
            }
            boolean authoritative = options.getOption(RustSubstrateOptions.ENABLE_AUTHORITATIVE_EXECUTION).get();
            return new ShadowingValueSnapshotter(
                new SnapshotHashDelegate(valueSnapshotter),
                rustValueSnapshotClient,
                mismatchReporter,
                authoritative
            );
        }
    }
}
