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
import org.gradle.internal.rustbridge.parser.RustParserClient;
import org.gradle.internal.rustbridge.parser.ShadowingScriptParser;
import org.gradle.internal.rustbridge.work.WorkerSchedulerClient;
import org.gradle.internal.rustbridge.bootstrap.RustBootstrapClient;
import org.gradle.internal.rustbridge.bootstrap.BootstrapLifecycleListener;
import org.gradle.internal.rustbridge.configcache.ConfigurationCacheShadowListener;
import org.gradle.internal.rustbridge.configcache.RustConfigCacheClient;
import org.gradle.internal.rustbridge.incrementalcompilation.IncrementalCompilationShadowListener;
import org.gradle.internal.rustbridge.incremental.RustIncrementalCompilationClient;
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
import org.gradle.internal.rustbridge.toolchain.ShadowingToolchainProvider;
import org.gradle.internal.rustbridge.eventstream.RustBuildEventStreamClient;
import org.gradle.internal.rustbridge.buildlayout.RustBuildLayoutClient;
import org.gradle.internal.rustbridge.buildlayout.ShadowingBuildLayoutTracker;
import org.gradle.internal.rustbridge.buildresult.RustBuildResultClient;
import org.gradle.internal.rustbridge.problems.RustProblemReportingClient;
import org.gradle.internal.rustbridge.resources.RustResourceManagementClient;
import org.gradle.internal.rustbridge.comparison.RustBuildComparisonClient;
import org.gradle.internal.rustbridge.console.RustConsoleClient;
import org.gradle.internal.rustbridge.testexec.RustTestExecutionClient;
import org.gradle.internal.rustbridge.buildinit.RustBuildInitClient;
import org.gradle.internal.rustbridge.buildinit.ShadowingBuildInitTracker;
import org.gradle.internal.rustbridge.incremental.RustIncrementalCompilationClient;
import org.gradle.internal.rustbridge.metrics.RustBuildMetricsClient;
import org.gradle.internal.rustbridge.gc.RustGarbageCollectionClient;
import org.gradle.internal.rustbridge.gc.ShadowingGarbageCollector;
import org.gradle.internal.rustbridge.jvmhost.JvmHostServiceImpl;
import org.gradle.internal.rustbridge.jvmhost.ProjectModelProviderAdapter;
import org.gradle.internal.rustbridge.evaluation.ProjectEvaluationShadowListener;
import org.gradle.internal.rustbridge.graph.TaskExecutionGraphShadowListener;
import org.gradle.internal.rustbridge.hash.RustGrpcFileHasher;
import org.gradle.internal.rustbridge.hash.ShadowingFileHasher;
import org.gradle.internal.rustbridge.history.ShadowingExecutionHistoryStore;
import org.gradle.internal.rustbridge.history.BinaryEncoderExecutionHistorySerializer;
import org.gradle.internal.rustbridge.snapshot.ShadowingInputFingerprinter;
import org.gradle.internal.rustbridge.watch.ShadowingFileWatcherRegistryFactory;
import org.gradle.internal.rustbridge.work.ShadowingWorkAvoidanceChecker;
import org.gradle.internal.rustbridge.execution.ShadowingExecutionPlanAdvisor;
import org.gradle.internal.rustbridge.plugin.ShadowingPluginRegistry;
import org.gradle.internal.rustbridge.worker.ShadowingWorkerPool;
import org.gradle.internal.rustbridge.eventstream.ShadowingBuildEventLogger;
import org.gradle.internal.rustbridge.eventstream.BuildLifecycleEventForwarder;
import org.gradle.internal.rustbridge.eventstream.TaskExecutionEventForwarder;
import org.gradle.internal.rustbridge.problems.ShadowingProblemCollector;
import org.gradle.internal.rustbridge.resources.ShadowingResourceCoordinator;
import org.gradle.internal.rustbridge.comparison.ShadowingBuildComparator;
import org.gradle.internal.rustbridge.console.ShadowingConsoleOutput;
import org.gradle.internal.rustbridge.metrics.ShadowingMetricsRecorder;
import org.gradle.internal.rustbridge.publishing.ShadowingArtifactPublisher;
import org.gradle.internal.rustbridge.exec.ShadowingExecActionFactory;
import org.gradle.internal.service.Provides;
import org.gradle.internal.service.ServiceRegistration;
import org.gradle.internal.service.scopes.AbstractGradleModuleServices;
import org.gradle.internal.snapshot.ValueSnapshotter;
import org.gradle.api.internal.project.ProjectStateRegistry;
import org.gradle.internal.hash.FileHasher;
import org.gradle.internal.execution.history.ExecutionHistoryStore;
import org.gradle.internal.execution.FileCollectionSnapshotter;
import org.gradle.internal.execution.InputFingerprinter;
import org.gradle.internal.watch.registry.FileWatcherRegistryFactory;
import org.gradle.process.internal.ExecActionFactory;
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
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return DaemonLauncher.noop();
            }

            String binaryPath = options.getValue(RustSubstrateOptions.DAEMON_BINARY_PATH);
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

            boolean enableJvmHost = RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_JVM_HOST);

            if (enableJvmHost) {
                return DaemonLauncher.withJvmHost(daemonBinary, socketDirectory);
            }

            return DaemonLauncher.of(daemonBinary, socketDirectory);
        }
    }

    private static class UserHomeServices implements ServiceRegistrationProvider {
        @Provides
        SubstrateClient createSubstrateClient(
            DaemonLauncher launcher,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
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
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
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

        // --- Execution history shadow wiring (ExecutionHistoryStore is Build scoped) ---

        @Provides
        @Nullable
        ShadowingExecutionHistoryStore createShadowingExecutionHistoryStore(
            ExecutionHistoryStore javaStore,
            RustExecutionHistoryClient rustClient,
            com.google.common.collect.Interner<String> stringInterner,
            org.gradle.internal.hash.ClassLoaderHierarchyHasher classLoaderHasher,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_HISTORY)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isAuthoritative(options);
            return new ShadowingExecutionHistoryStore(
                javaStore,
                rustClient,
                new BinaryEncoderExecutionHistorySerializer(stringInterner, classLoaderHasher),
                authoritative
            );
        }

        // --- File fingerprint shadow wiring ---

        @Provides
        @Nullable
        ShadowingFileCollectionSnapshotter createShadowingFileCollectionSnapshotter(
            FileCollectionSnapshotter javaSnapshotter,
            RustFileFingerprintClient rustFingerprintClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_FINGERPRINTING)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_FINGERPRINTING
            );
            return new ShadowingFileCollectionSnapshotter(
                javaSnapshotter,
                rustFingerprintClient,
                mismatchReporter,
                authoritative
            );
        }

        // --- Input fingerprinter shadow wiring ---

        @Provides
        @Nullable
        ShadowingInputFingerprinter createShadowingInputFingerprinter(
            InputFingerprinter javaFingerprinter,
            @Nullable ShadowingValueSnapshotter shadowingValueSnapshotter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_SNAPSHOTTING)) {
                return null;
            }
            return new ShadowingInputFingerprinter(javaFingerprinter, shadowingValueSnapshotter);
        }

        // --- File watch shadow wiring ---

        @Provides
        @Nullable
        ShadowingFileWatcherRegistryFactory createShadowingFileWatcherRegistryFactory(
            FileWatcherRegistryFactory delegateFactory,
            RustFileWatchClient rustFileWatchClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_FILE_WATCH)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_FILE_WATCH
            );
            return new ShadowingFileWatcherRegistryFactory(
                delegateFactory,
                rustFileWatchClient,
                mismatchReporter,
                authoritative
            );
        }

        // --- Work / up-to-date shadow wiring ---

        @Provides
        @Nullable
        ShadowingWorkAvoidanceChecker createShadowingWorkAvoidanceChecker(
            WorkerSchedulerClient workerSchedulerClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingWorkAvoidanceChecker(workerSchedulerClient, mismatchReporter);
        }

        // --- Execution plan shadow wiring ---

        @Provides
        @Nullable
        ShadowingExecutionPlanAdvisor createShadowingExecutionPlanAdvisor(
            ExecutionPlanClient executionPlanClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_ADVISORY_EXECUTION)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_EXECUTION_PLAN
            );
            return new ShadowingExecutionPlanAdvisor(executionPlanClient, mismatchReporter, authoritative);
        }

        // --- Plugin shadow wiring ---

        @Provides
        @Nullable
        ShadowingPluginRegistry createShadowingPluginRegistry(
            RustPluginClient rustPluginClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingPluginRegistry(rustPluginClient, mismatchReporter);
        }

        // --- Worker pool shadow wiring ---

        @Provides
        @Nullable
        ShadowingWorkerPool createShadowingWorkerPool(
            RustWorkerProcessClient rustWorkerProcessClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingWorkerPool(rustWorkerProcessClient, mismatchReporter);
        }

        // --- Build event stream shadow wiring ---

        @Provides
        @Nullable
        ShadowingBuildEventLogger createShadowingBuildEventLogger(
            RustBuildEventStreamClient rustBuildEventStreamClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingBuildEventLogger(rustBuildEventStreamClient, mismatchReporter);
        }

        // --- Problem reporting shadow wiring ---

        @Provides
        @Nullable
        ShadowingProblemCollector createShadowingProblemCollector(
            RustProblemReportingClient rustProblemReportingClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingProblemCollector(rustProblemReportingClient, mismatchReporter);
        }

        // --- Resource management shadow wiring ---

        @Provides
        @Nullable
        ShadowingResourceCoordinator createShadowingResourceCoordinator(
            RustResourceManagementClient rustResourceManagementClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingResourceCoordinator(rustResourceManagementClient, mismatchReporter);
        }

        // --- Build comparison shadow wiring ---

        @Provides
        @Nullable
        ShadowingBuildComparator createShadowingBuildComparator(
            RustBuildComparisonClient rustBuildComparisonClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingBuildComparator(rustBuildComparisonClient, mismatchReporter);
        }

        // --- Console shadow wiring ---

        @Provides
        @Nullable
        ShadowingConsoleOutput createShadowingConsoleOutput(
            RustConsoleClient rustConsoleClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingConsoleOutput(rustConsoleClient, mismatchReporter);
        }

        // --- Metrics shadow wiring ---

        @Provides
        @Nullable
        ShadowingMetricsRecorder createShadowingMetricsRecorder(
            RustBuildMetricsClient rustBuildMetricsClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingMetricsRecorder(rustBuildMetricsClient, mismatchReporter);
        }

        // --- Artifact publishing shadow wiring ---

        @Provides
        @Nullable
        ShadowingArtifactPublisher createShadowingArtifactPublisher(
            RustArtifactPublishingClient rustArtifactPublishingClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingArtifactPublisher(rustArtifactPublishingClient, mismatchReporter);
        }

        // --- Parser service wiring ---

        @Provides
        @Nullable
        RustParserClient createRustParserClient(
            SubstrateClient client,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return new RustParserClient(SubstrateClient.noop());
            }
            return new RustParserClient(client);
        }

        @Provides
        @Nullable
        ShadowingScriptParser createShadowingScriptParser(
            RustParserClient rustParserClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingScriptParser(rustParserClient, mismatchReporter);
        }

        @Provides
        @org.gradle.internal.service.scopes.PrivateService
        void wireProjectModelProvider(
            DaemonLauncher daemonLauncher,
            @Nullable ProjectStateRegistry projectStateRegistry,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return;
            }
            JvmHostServiceImpl serviceImpl = daemonLauncher.getJvmHostServiceImpl();
            if (serviceImpl != null && projectStateRegistry != null) {
                serviceImpl.setProjectModelProvider(
                    new ProjectModelProviderAdapter(projectStateRegistry));
            }
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
                rustBuildResultClient, rustBuildMetricsClient, rustExecutionHistoryClient);
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        @Nullable
        DependencyResolutionShadowListener createDependencyResolutionShadowListener(
            RustDependencyResolutionClient rustDependencyResolutionClient,
            HashMismatchReporter mismatchReporter,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_DEPENDENCY_RESOLUTION)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_DEPENDENCY_RESOLUTION
            );
            DependencyResolutionShadowListener listener =
                new DependencyResolutionShadowListener(rustDependencyResolutionClient, mismatchReporter, authoritative);
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        @Nullable
        TransformExecutionShadowListener createTransformExecutionShadowListener(
            SubstrateClient client,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
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
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
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
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_TEST_EXECUTION)) {
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
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
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
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_CACHE
            );
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
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_TASK_GRAPH)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_TASK_GRAPH
            );
            TaskGraphShadowListener listener = new TaskGraphShadowListener(
                new TaskGraphShadowReporter(rustTaskGraphClient, mismatchReporter, authoritative));
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
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_CONFIGURATION)) {
                return null;
            }
            PropertyShadowEvaluationListener listener = new PropertyShadowEvaluationListener(
                new ShadowingPropertyResolver(rustConfigurationClient, mismatchReporter));
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        @Nullable
        ProjectEvaluationShadowListener createProjectEvaluationShadowListener(
            SubstrateClient client,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            ProjectEvaluationShadowListener listener = new ProjectEvaluationShadowListener(client);
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        @Nullable
        TaskExecutionGraphShadowListener createTaskExecutionGraphShadowListener(
            SubstrateClient client,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            TaskExecutionGraphShadowListener listener = new TaskExecutionGraphShadowListener(client);
            listenerManager.addListener(listener);
            return listener;
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
        IncrementalCompilationShadowListener createIncrementalCompilationShadowListener(
            RustIncrementalCompilationClient rustIncrementalCompilationClient,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_INCREMENTAL)) {
                return null;
            }
            return new IncrementalCompilationShadowListener(rustIncrementalCompilationClient);
        }

        @Provides
        @Nullable
        ShadowingToolchainProvider createShadowingToolchainProvider(
            RustToolchainServiceClient rustToolchainServiceClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingToolchainProvider(rustToolchainServiceClient, mismatchReporter);
        }

        @Provides
        @Nullable
        ShadowingBuildLayoutTracker createShadowingBuildLayoutTracker(
            RustBuildLayoutClient rustBuildLayoutClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingBuildLayoutTracker(rustBuildLayoutClient, mismatchReporter);
        }

        @Provides
        @Nullable
        ShadowingBuildInitTracker createShadowingBuildInitTracker(
            RustBuildInitClient rustBuildInitClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingBuildInitTracker(rustBuildInitClient, mismatchReporter);
        }

        @Provides
        @Nullable
        ShadowingGarbageCollector createShadowingGarbageCollector(
            RustGarbageCollectionClient rustGarbageCollectionClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            return new ShadowingGarbageCollector(rustGarbageCollectionClient, mismatchReporter);
        }

        // --- Build lifecycle event forwarding ---

        @Provides
        @Nullable
        BuildLifecycleEventForwarder createBuildLifecycleEventForwarder(
            RustBuildEventStreamClient rustBuildEventStreamClient,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            BuildLifecycleEventForwarder listener = new BuildLifecycleEventForwarder(rustBuildEventStreamClient);
            listenerManager.addListener(listener);
            return listener;
        }

        @Provides
        @Nullable
        TaskExecutionEventForwarder createTaskExecutionEventForwarder(
            RustBuildEventStreamClient rustBuildEventStreamClient,
            ListenerManager listenerManager,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubstrateEnabled(options)) {
                return null;
            }
            TaskExecutionEventForwarder listener = new TaskExecutionEventForwarder(rustBuildEventStreamClient);
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

        // --- Exec service wiring ---

        @Provides
        @org.gradle.internal.service.scopes.PrivateService
        @Nullable
        ShadowingExecActionFactory createShadowingExecActionFactory(
            ExecActionFactory javaFactory,
            SubstrateClient client,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_EXEC)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isAuthoritative(options);
            return new ShadowingExecActionFactory(javaFactory, client, mismatchReporter, authoritative);
        }

        // --- Hash service wiring (FileHasher is UserHome/BuildSession scoped) ---

        @Provides
        @Nullable
        ShadowingFileHasher createShadowingFileHasher(
            FileHasher javaFileHasher,
            SubstrateClient client,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_HASHING)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isAuthoritative(options);
            return new ShadowingFileHasher(
                javaFileHasher,
                new RustGrpcFileHasher(client),
                mismatchReporter,
                authoritative
            );
        }

        @Provides
        @Nullable
        ShadowingValueSnapshotter createShadowingValueSnapshotter(
            ValueSnapshotter valueSnapshotter,
            RustValueSnapshotClient rustValueSnapshotClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            if (!RustSubstrateOptions.isSubsystemEnabled(options, RustSubstrateOptions.ENABLE_RUST_SNAPSHOTTING)) {
                return null;
            }
            boolean authoritative = RustSubstrateOptions.isSubsystemAuthoritative(
                options,
                RustSubstrateOptions.ENABLE_RUST_AUTHORITATIVE_SNAPSHOTTING
            );
            return new ShadowingValueSnapshotter(
                new SnapshotHashDelegate(valueSnapshotter),
                rustValueSnapshotClient,
                mismatchReporter,
                authoritative
            );
        }
    }
}
