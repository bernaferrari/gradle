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
import org.gradle.internal.rustbridge.configuration.RustConfigurationClient;
import org.gradle.internal.rustbridge.execution.ExecutionPlanClient;
import org.gradle.internal.rustbridge.fingerprint.RustFileFingerprintClient;
import org.gradle.internal.rustbridge.history.RustExecutionHistoryClient;
import org.gradle.internal.rustbridge.snapshot.RustValueSnapshotClient;
import org.gradle.internal.rustbridge.taskgraph.RustTaskGraphClient;
import org.gradle.internal.rustbridge.watch.RustFileWatchClient;
import org.gradle.internal.rustbridge.work.WorkerSchedulerClient;
import org.gradle.internal.service.ServiceRegistration;
import org.gradle.internal.service.scopes.AbstractGradleModuleServices;
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
        RustFileFingerprintClient createRustFileFingerprintClient(SubstrateClient client) {
            return new RustFileFingerprintClient(client);
        }

        @Provides
        RustExecutionHistoryClient createRustExecutionHistoryClient(SubstrateClient client) {
            return new RustExecutionHistoryClient(client);
        }

        @Provides
        RustValueSnapshotClient createRustValueSnapshotClient(SubstrateClient client) {
            return new RustValueSnapshotClient(client);
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
        HashMismatchReporter createHashMismatchReporter() {
            return new HashMismatchReporter(true);
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
        ShadowingBuildCacheKeyComputer createShadowingBuildCacheKeyComputer(
            BuildCacheOrchestrationClient cacheOrchestrationClient,
            HashMismatchReporter mismatchReporter,
            InternalOptions options
        ) {
            boolean authoritative = options.getOption(RustSubstrateOptions.ENABLE_AUTHORITATIVE_EXECUTION).get();
            return new ShadowingBuildCacheKeyComputer(cacheOrchestrationClient, mismatchReporter, authoritative);
        }
    }
}
