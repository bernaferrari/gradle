package org.gradle.internal.rustbridge.dependency;

import org.gradle.api.artifacts.DependencyResolutionListener;
import org.gradle.api.artifacts.ResolvableDependencies;
import org.gradle.api.logging.Logging;
import org.slf4j.Logger;

/**
 * A {@link DependencyResolutionListener} that shadows dependency resolution results
 * to the Rust substrate. Fire-and-forget: never affects build correctness.
 */
public class DependencyResolutionShadowListener implements DependencyResolutionListener {

    private static final Logger LOGGER = Logging.getLogger(DependencyResolutionShadowListener.class);

    private final RustDependencyResolutionClient client;

    public DependencyResolutionShadowListener(RustDependencyResolutionClient client) {
        this.client = client;
    }

    @Override
    public void beforeResolve(ResolvableDependencies dependencies) {
        // Nothing to do before resolution
    }

    @Override
    public void afterResolve(ResolvableDependencies dependencies) {
        if (client == null) {
            return;
        }

        try {
            String configName = dependencies.getName();
            long startTime = System.currentTimeMillis();

            // Fire-and-forget: record the resolution event to Rust
            client.recordResolution(configName, 0, 0, true, 0);

            LOGGER.debug("[substrate:dep-resolve] shadow recorded resolution for {}", configName);
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] shadow resolution failed", e);
        }
    }
}
