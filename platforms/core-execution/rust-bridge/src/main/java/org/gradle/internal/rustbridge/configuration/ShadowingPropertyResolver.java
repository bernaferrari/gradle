package org.gradle.internal.rustbridge.configuration;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

import java.util.List;
import java.util.Map;

/**
 * A property resolver that shadows Java property resolution against the Rust
 * ConfigurationService. After each project is evaluated, this validates that
 * the Rust substrate resolves properties identically to Gradle's Java implementation.
 *
 * <p>Always returns the Java result. Mismatches are reported but do not affect
 * build correctness.</p>
 */
public class ShadowingPropertyResolver {

    private static final Logger LOGGER = Logging.getLogger(ShadowingPropertyResolver.class);

    private final RustConfigurationClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingPropertyResolver(
        RustConfigurationClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Register a project's configuration with the Rust substrate.
     */
    public void registerProject(
        String projectPath,
        String projectDir,
        Map<String, String> properties,
        List<String> appliedPlugins
    ) {
        try {
            rustClient.registerProject(projectPath, projectDir, properties, appliedPlugins);
        } catch (Exception e) {
            LOGGER.debug("[substrate:config] failed to register project with Rust", e);
        }
    }

    /**
     * Shadow-resolve a property: resolve via both Java and Rust, compare, return Java result.
     *
     * @param projectPath the project path (e.g., ":")
     * @param propertyName the property name (e.g., "version")
     * @param javaValue the Java-resolved value (authoritative)
     */
    public void shadowResolveProperty(String projectPath, String propertyName, String javaValue) {
        if (rustClient == null) {
            return;
        }

        try {
            RustConfigurationClient.PropertyResult rustResult =
                rustClient.resolveProperty(projectPath, propertyName, "shadow");

            if (rustResult.isSuccess() && rustResult.isFound()) {
                String rustValue = rustResult.getValue();
                if (javaValue != null && javaValue.equals(rustValue)) {
                    mismatchReporter.reportMatch();
                    LOGGER.debug("[substrate:config] shadow OK: {}.{} = {}",
                        projectPath, propertyName, javaValue);
                } else {
                    mismatchReporter.reportRustError(
                        projectPath + ":" + propertyName,
                        new RuntimeException(
                            "property mismatch: java=" + javaValue + " rust=" + rustValue
                        )
                    );
                    LOGGER.debug("[substrate:config] shadow MISMATCH: {}.{} java={} rust={}",
                        projectPath, propertyName, javaValue, rustValue);
                }
            } else {
                mismatchReporter.reportRustError(
                    projectPath + ":" + propertyName,
                    new RuntimeException(rustResult.getErrorMessage())
                );
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError(projectPath + ":" + propertyName, e);
            LOGGER.debug("[substrate:config] shadow comparison failed for {}.{}",
                projectPath, propertyName, e);
        }
    }

    /**
     * Shadow-validate the configuration cache.
     */
    public void shadowValidateConfigCache(
        String projectPath,
        byte[] configHash,
        List<String> inputFiles,
        List<String> buildScriptHashes,
        boolean javaValid
    ) {
        if (rustClient == null) {
            return;
        }

        try {
            RustConfigurationClient.ValidationResult rustResult =
                rustClient.validateConfigCache(projectPath, configHash, inputFiles, buildScriptHashes);

            if (rustResult.isSuccess()) {
                if (javaValid == rustResult.isValid()) {
                    mismatchReporter.reportMatch();
                } else {
                    mismatchReporter.reportRustError(
                        "config-cache:" + projectPath,
                        new RuntimeException(
                            "cache validation mismatch: java=" + javaValid
                                + " rust=" + rustResult.isValid()
                                + " reason=" + rustResult.getReason()
                        )
                    );
                }
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("config-cache:" + projectPath, e);
        }
    }
}
