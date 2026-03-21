package org.gradle.internal.rustbridge.bootstrap;

import org.gradle.api.logging.Logging;
import org.gradle.initialization.RootBuildLifecycleListener;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.util.Collections;
import java.util.HashMap;
import java.util.Map;
import java.util.UUID;

/**
 * Build lifecycle listener that notifies the Rust substrate daemon
 * when a build starts and completes.
 */
public class BootstrapLifecycleListener implements RootBuildLifecycleListener {

    private static final Logger LOGGER = Logging.getLogger(BootstrapLifecycleListener.class);

    private final RustBootstrapClient bootstrapClient;
    private final String buildId;
    private final String projectDir;
    private final int parallelism;
    private final long startTimeMs;

    private volatile boolean buildInitialized = false;

    public BootstrapLifecycleListener(RustBootstrapClient bootstrapClient,
                                       String projectDir, int parallelism) {
        this.bootstrapClient = bootstrapClient;
        this.buildId = UUID.randomUUID().toString();
        this.projectDir = projectDir;
        this.parallelism = parallelism;
        this.startTimeMs = System.currentTimeMillis();
    }

    @Override
    public void afterStart() {
        try {
            Map<String, String> props = new HashMap<>();
            for (Map.Entry<Object, Object> entry : System.getProperties().entrySet()) {
                if (entry.getKey() instanceof String && entry.getValue() instanceof String) {
                    props.put((String) entry.getKey(), (String) entry.getValue());
                }
            }

            RustBootstrapClient.BuildInitResult result = bootstrapClient.initBuild(
                buildId, projectDir, startTimeMs, parallelism,
                props, Collections.emptyList()
            );
            buildInitialized = true;
            LOGGER.debug("[substrate:bootstrap] initialized build {}", result.getBuildId());
        } catch (Exception e) {
            LOGGER.debug("[substrate:bootstrap] failed to initialize build", e);
        }
    }

    @Override
    public void beforeComplete(@Nullable Throwable failure) {
        if (!buildInitialized) {
            return;
        }

        try {
            String outcome = failure != null ? "FAILED" : "SUCCESS";
            long durationMs = System.currentTimeMillis() - startTimeMs;
            bootstrapClient.completeBuild(buildId, outcome, durationMs);
        } catch (Exception e) {
            LOGGER.debug("[substrate:bootstrap] failed to complete build", e);
        }
    }
}
