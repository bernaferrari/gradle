package org.gradle.internal.rustbridge.plugin;

import gradle.substrate.v1.GetExtensionResponse;
import gradle.substrate.v1.GetExtensionsResponse;
import gradle.substrate.v1.ResolveConventionResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.Collections;
import java.util.List;
import java.util.Map;

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

    /**
     * Shadow an extension value read, comparing JVM and Rust results.
     *
     * @param projectPath   the project path
     * @param extensionName the extension name
     * @param propertyPath  the property path within the extension
     * @param javaValue     the value from the JVM
     */
    public void shadowGetExtension(String projectPath, String extensionName,
                                    String propertyPath, String javaValue) {
        try {
            GetExtensionResponse rustResp = rustClient.getExtension(
                projectPath, extensionName, propertyPath
            );

            if (javaValue.equals(rustResp.getValue())) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:plugin] shadow getExtension MATCH: ext={}, prop={}",
                    extensionName, propertyPath);
            } else {
                mismatchReporter.reportMismatch(
                    "plugin:getExtension:" + projectPath + ":" + extensionName,
                    javaValue, rustResp.getValue()
                );
                LOGGER.debug("[substrate:plugin] shadow getExtension MISMATCH: ext={}, prop={}, java={}, rust={}",
                    extensionName, propertyPath, javaValue, rustResp.getValue());
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("plugin:getExtension:" + extensionName, e);
            LOGGER.debug("[substrate:plugin] shadow getExtension error for ext={}: {}",
                extensionName, e.getMessage());
        }
    }

    /**
     * Shadow a convention registration.
     *
     * @param projectPath      the project path
     * @param pluginId         the registering plugin
     * @param conventions      the convention mappings
     * @param conventionSource the source of the conventions
     */
    public void shadowRegisterConvention(String projectPath, String pluginId,
                                          Map<String, String> conventions, String conventionSource) {
        try {
            boolean rustResult = rustClient.registerConvention(
                projectPath, pluginId, conventions, conventionSource
            );

            if (rustResult) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:plugin] shadow registerConvention MATCH: plugin={}, source={}",
                    pluginId, conventionSource);
            } else {
                mismatchReporter.reportMismatch(
                    "plugin:registerConvention:" + pluginId,
                    "true", "false"
                );
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("plugin:registerConvention:" + pluginId, e);
            LOGGER.debug("[substrate:plugin] shadow registerConvention error for plugin={}: {}",
                pluginId, e.getMessage());
        }
    }

    /**
     * Shadow a convention resolution.
     *
     * @param projectPath       the project path
     * @param propertyName      the property to resolve
     * @param preferredSources  preferred convention sources
     * @param javaValue         the value from the JVM
     */
    public void shadowResolveConvention(String projectPath, String propertyName,
                                         List<String> preferredSources, String javaValue) {
        try {
            ResolveConventionResponse rustResp = rustClient.resolveConvention(
                projectPath, propertyName, preferredSources
            );

            if (javaValue.equals(rustResp.getValue())) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:plugin] shadow resolveConvention MATCH: prop={}", propertyName);
            } else {
                mismatchReporter.reportMismatch(
                    "plugin:resolveConvention:" + projectPath + ":" + propertyName,
                    javaValue, rustResp.getValue()
                );
                LOGGER.debug("[substrate:plugin] shadow resolveConvention MISMATCH: prop={}, java={}, rust={}",
                    propertyName, javaValue, rustResp.getValue());
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("plugin:resolveConvention:" + propertyName, e);
            LOGGER.debug("[substrate:plugin] shadow resolveConvention error for prop={}: {}",
                propertyName, e.getMessage());
        }
    }
}
