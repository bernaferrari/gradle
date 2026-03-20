package org.gradle.internal.rustbridge.configuration;

import gradle.substrate.v1.*;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * Client for the Rust configuration service.
 * Registers projects, resolves properties, and validates configuration cache.
 */
public class RustConfigurationClient {

    private static final Logger LOGGER = Logging.getLogger(RustConfigurationClient.class);

    private final SubstrateClient client;

    public RustConfigurationClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Register a project and its properties with the Rust substrate.
     */
    public boolean registerProject(
        String projectPath,
        String projectDir,
        Map<String, String> properties,
        List<String> appliedPlugins
    ) {
        if (client.isNoop()) {
            return false;
        }

        try {
            RegisterProjectRequest.Builder builder = RegisterProjectRequest.newBuilder()
                .setProjectPath(projectPath)
                .setProjectDir(projectDir)
                .putAllProperties(properties)
                .addAllAppliedPlugins(appliedPlugins);

            RegisterProjectResponse response = client.getConfigurationStub()
                .registerProject(builder.build());

            LOGGER.debug("[substrate:config] registerProject({}) = {}",
                projectPath, response.getSuccess());

            return response.getSuccess();
        } catch (Exception e) {
            LOGGER.debug("[substrate:config] registerProject failed", e);
            return false;
        }
    }

    /**
     * Resolve a property value via the Rust substrate.
     */
    public PropertyResult resolveProperty(String projectPath, String propertyName, String requestedBy) {
        if (client.isNoop()) {
            return PropertyResult.error("Substrate not available");
        }

        try {
            ResolvePropertyResponse response = client.getConfigurationStub()
                .resolveProperty(ResolvePropertyRequest.newBuilder()
                    .setProjectPath(projectPath)
                    .setPropertyName(propertyName)
                    .setRequestedBy(requestedBy)
                    .build());

            return new PropertyResult(
                response.getValue(),
                response.getSource(),
                response.getFound(),
                true,
                ""
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:config] resolveProperty failed", e);
            return PropertyResult.error("gRPC error: " + e.getMessage());
        }
    }

    /**
     * Validate the configuration cache for a project.
     */
    public ValidationResult validateConfigCache(
        String projectPath,
        byte[] expectedHash,
        List<String> inputFiles,
        List<String> buildScriptHashes
    ) {
        if (client.isNoop()) {
            return ValidationResult.error("Substrate not available");
        }

        try {
            ValidateConfigCacheResponse response = client.getConfigurationStub()
                .validateConfigCache(ValidateConfigCacheRequest.newBuilder()
                    .setProjectPath(projectPath)
                    .setExpectedHash(com.google.protobuf.ByteString.copyFrom(expectedHash))
                    .addAllInputFiles(inputFiles)
                    .addAllBuildScriptHashes(buildScriptHashes)
                    .build());

            return new ValidationResult(
                response.getValid(),
                response.getReason(),
                true
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:config] validateConfigCache failed", e);
            return ValidationResult.error("gRPC error: " + e.getMessage());
        }
    }

    /**
     * Cache resolved configuration state.
     */
    public boolean cacheConfiguration(String projectPath, byte[] configHash) {
        if (client.isNoop()) {
            return false;
        }

        try {
            CacheConfigurationResponse response = client.getConfigurationStub()
                .cacheConfiguration(CacheConfigurationRequest.newBuilder()
                    .setProjectPath(projectPath)
                    .setConfigHash(com.google.protobuf.ByteString.copyFrom(configHash))
                    .setTimestampMs(System.currentTimeMillis())
                    .build());

            return response.getCached();
        } catch (Exception e) {
            LOGGER.debug("[substrate:config] cacheConfiguration failed", e);
            return false;
        }
    }

    /**
     * Result of resolving a property.
     */
    public static class PropertyResult {
        private final String value;
        private final String source;
        private final boolean found;
        private final boolean success;
        private final String errorMessage;

        private PropertyResult(String value, String source, boolean found,
                               boolean success, String errorMessage) {
            this.value = value;
            this.source = source;
            this.found = found;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public static PropertyResult error(String message) {
            return new PropertyResult("", "", false, false, message);
        }

        public String getValue() { return value; }
        public String getSource() { return source; }
        public boolean isFound() { return found; }
        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }
    }

    /**
     * Result of validating the configuration cache.
     */
    public static class ValidationResult {
        private final boolean valid;
        private final String reason;
        private final boolean success;

        private ValidationResult(boolean valid, String reason, boolean success) {
            this.valid = valid;
            this.reason = reason;
            this.success = success;
        }

        public static ValidationResult error(String message) {
            return new ValidationResult(false, message, false);
        }

        public boolean isValid() { return valid; }
        public String getReason() { return reason; }
        public boolean isSuccess() { return success; }
    }
}
