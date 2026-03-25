package org.gradle.internal.rustbridge.plugin;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.Collections;

/**
 * Shadow adapter that compares JVM plugin registry operations with Rust.
 *
 * <p>In shadow mode, runs both JVM and Rust plugin operations and reports mismatches.
 * Covers plugin registration, application, and existence checks.</p>
 */
public class ShadowingPluginRegistry {

    private static final Logger LOGGER = Logging.getLogger(ShadowingPluginRegistry.class);

    private final RustPluginClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingPluginRegistry(
        RustPluginClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Shadow a plugin registration, comparing JVM and Rust results.
     *
     * @param pluginId    the plugin identifier
     * @param pluginClass the plugin implementation class name
     * @param javaResult  the boolean result from the JVM registry
     */
    public void shadowRegisterPlugin(String pluginId, String pluginClass, boolean javaResult) {
        try {
            boolean rustResult = rustClient.registerPlugin(
                pluginId, pluginClass, "", false, Collections.emptyList()
            );

            if (javaResult == rustResult) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:plugin] shadow register MATCH: id={}, class={}", pluginId, pluginClass);
            } else {
                mismatchReporter.reportMismatch(
                    "plugin:register:" + pluginId,
                    String.valueOf(javaResult),
                    String.valueOf(rustResult)
                );
                LOGGER.debug("[substrate:plugin] shadow register MISMATCH: id={}, java={}, rust={}",
                    pluginId, javaResult, rustResult);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("plugin:register:" + pluginId, e);
            LOGGER.debug("[substrate:plugin] shadow register error for id={}: {}", pluginId, e.getMessage());
        }
    }

    /**
     * Shadow a plugin application, comparing JVM and Rust results.
     *
     * @param pluginId    the plugin identifier
     * @param projectPath the project path where the plugin is applied
     * @param javaResult  the boolean result from the JVM registry
     */
    public void shadowApplyPlugin(String pluginId, String projectPath, boolean javaResult) {
        try {
            boolean rustResult = rustClient.applyPlugin(pluginId, projectPath, 0);

            if (javaResult == rustResult) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:plugin] shadow apply MATCH: id={}, project={}", pluginId, projectPath);
            } else {
                mismatchReporter.reportMismatch(
                    "plugin:apply:" + pluginId + ":" + projectPath,
                    String.valueOf(javaResult),
                    String.valueOf(rustResult)
                );
                LOGGER.debug("[substrate:plugin] shadow apply MISMATCH: id={}, project={}, java={}, rust={}",
                    pluginId, projectPath, javaResult, rustResult);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("plugin:apply:" + pluginId + ":" + projectPath, e);
            LOGGER.debug("[substrate:plugin] shadow apply error for id={}, project={}: {}",
                pluginId, projectPath, e.getMessage());
        }
    }

    /**
     * Shadow a plugin existence check, comparing JVM and Rust results.
     *
     * @param pluginId    the plugin identifier
     * @param projectPath the project path to check
     * @param javaResult  the boolean result from the JVM registry
     */
    public void shadowHasPlugin(String pluginId, String projectPath, boolean javaResult) {
        try {
            boolean rustResult = rustClient.hasPlugin(pluginId, projectPath);

            if (javaResult == rustResult) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:plugin] shadow hasPlugin MATCH: id={}, project={}", pluginId, projectPath);
            } else {
                mismatchReporter.reportMismatch(
                    "plugin:hasPlugin:" + pluginId + ":" + projectPath,
                    String.valueOf(javaResult),
                    String.valueOf(rustResult)
                );
                LOGGER.debug("[substrate:plugin] shadow hasPlugin MISMATCH: id={}, project={}, java={}, rust={}",
                    pluginId, projectPath, javaResult, rustResult);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("plugin:hasPlugin:" + pluginId + ":" + projectPath, e);
            LOGGER.debug("[substrate:plugin] shadow hasPlugin error for id={}, project={}: {}",
                pluginId, projectPath, e.getMessage());
        }
    }
}
