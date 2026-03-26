package org.gradle.internal.rustbridge.plugin;

import gradle.substrate.v1.ApplyPluginRequest;
import gradle.substrate.v1.ApplyPluginResponse;
import gradle.substrate.v1.ExtensionInfo;
import gradle.substrate.v1.GetAppliedPluginsRequest;
import gradle.substrate.v1.GetAppliedPluginsResponse;
import gradle.substrate.v1.GetExtensionRequest;
import gradle.substrate.v1.GetExtensionResponse;
import gradle.substrate.v1.GetExtensionsRequest;
import gradle.substrate.v1.GetExtensionsResponse;
import gradle.substrate.v1.HasPluginRequest;
import gradle.substrate.v1.HasPluginResponse;
import gradle.substrate.v1.PluginInfo;
import gradle.substrate.v1.PluginServiceGrpc;
import gradle.substrate.v1.RegisterConventionRequest;
import gradle.substrate.v1.RegisterConventionResponse;
import gradle.substrate.v1.RegisterPluginRequest;
import gradle.substrate.v1.RegisterPluginResponse;
import gradle.substrate.v1.ResolveConventionRequest;
import gradle.substrate.v1.ResolveConventionResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.Collections;
import java.util.List;
import java.util.Map;

/**
 * Client for the Rust plugin service.
 * Tracks plugin registrations and applications via gRPC.
 */
public class RustPluginClient {

    private static final Logger LOGGER = Logging.getLogger(RustPluginClient.class);

    private final SubstrateClient client;

    public RustPluginClient(SubstrateClient client) {
        this.client = client;
    }

    public boolean registerPlugin(String pluginId, String pluginClass, String version,
                                   boolean isImperative, List<String> appliesTo) {
        if (client.isNoop()) {
            return false;
        }

        try {
            RegisterPluginResponse response = client.getPluginStub()
                .registerPlugin(RegisterPluginRequest.newBuilder()
                    .setPluginId(pluginId)
                    .setPluginClass(pluginClass)
                    .setVersion(version)
                    .setIsImperative(isImperative)
                    .addAllAppliesTo(appliesTo)
                    .build());
            return response.getSuccess();
        } catch (Exception e) {
            LOGGER.debug("[substrate:plugin] register failed for {}", pluginId, e);
            return false;
        }
    }

    public boolean applyPlugin(String pluginId, String projectPath, long applyOrder) {
        if (client.isNoop()) {
            return false;
        }

        try {
            ApplyPluginResponse response = client.getPluginStub()
                .applyPlugin(ApplyPluginRequest.newBuilder()
                    .setPluginId(pluginId)
                    .setProjectPath(projectPath)
                    .setApplyOrder(applyOrder)
                    .build());
            return response.getSuccess();
        } catch (Exception e) {
            LOGGER.debug("[substrate:plugin] apply failed for {}", pluginId, e);
            return false;
        }
    }

    public boolean hasPlugin(String pluginId, String projectPath) {
        if (client.isNoop()) {
            return false;
        }

        try {
            HasPluginResponse response = client.getPluginStub()
                .hasPlugin(HasPluginRequest.newBuilder()
                    .setPluginId(pluginId)
                    .setProjectPath(projectPath)
                    .build());
            return response.getHasPlugin();
        } catch (Exception e) {
            LOGGER.debug("[substrate:plugin] hasPlugin check failed", e);
            return false;
        }
    }

    public List<PluginInfo> getAppliedPlugins(String projectPath) {
        if (client.isNoop()) {
            return java.util.Collections.emptyList();
        }

        try {
            GetAppliedPluginsResponse response = client.getPluginStub()
                .getAppliedPlugins(GetAppliedPluginsRequest.newBuilder()
                    .setProjectPath(projectPath)
                    .build());
            return response.getPluginsList();
        } catch (Exception e) {
            LOGGER.debug("[substrate:plugin] getAppliedPlugins failed", e);
            return Collections.emptyList();
        }
    }

    /**
     * Read an extension value from the Rust plugin service.
     *
     * @return the extension response (found, value, properties)
     */
    public GetExtensionResponse getExtension(String projectPath, String extensionName,
                                              String propertyPath) {
        if (client.isNoop()) {
            return GetExtensionResponse.getDefaultInstance();
        }

        try {
            GetExtensionRequest.Builder builder = GetExtensionRequest.newBuilder()
                .setProjectPath(projectPath)
                .setExtensionName(extensionName);
            if (propertyPath != null && !propertyPath.isEmpty()) {
                builder.setPropertyPath(propertyPath);
            }
            return client.getPluginStub().getExtension(builder.build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:plugin] getExtension failed for {}", extensionName, e);
            return GetExtensionResponse.getDefaultInstance();
        }
    }

    /**
     * List all extensions registered for a project.
     */
    public List<ExtensionInfo> getExtensions(String projectPath) {
        if (client.isNoop()) {
            return Collections.emptyList();
        }

        try {
            GetExtensionsResponse response = client.getPluginStub()
                .getExtensions(GetExtensionsRequest.newBuilder()
                    .setProjectPath(projectPath)
                    .build());
            return response.getExtensionsList();
        } catch (Exception e) {
            LOGGER.debug("[substrate:plugin] getExtensions failed", e);
            return Collections.emptyList();
        }
    }

    /**
     * Register convention mappings from a plugin.
     */
    public boolean registerConvention(String projectPath, String pluginId,
                                       Map<String, String> conventions, String conventionSource) {
        if (client.isNoop()) {
            return false;
        }

        try {
            RegisterConventionResponse response = client.getPluginStub()
                .registerConvention(RegisterConventionRequest.newBuilder()
                    .setProjectPath(projectPath)
                    .setPluginId(pluginId)
                    .putAllConventions(conventions)
                    .setConventionSource(conventionSource)
                    .build());
            return response.getRegistered();
        } catch (Exception e) {
            LOGGER.debug("[substrate:plugin] registerConvention failed for {}", pluginId, e);
            return false;
        }
    }

    /**
     * Resolve a property value via convention mapping.
     */
    public ResolveConventionResponse resolveConvention(String projectPath, String propertyName,
                                                        List<String> preferredSources) {
        if (client.isNoop()) {
            return ResolveConventionResponse.getDefaultInstance();
        }

        try {
            ResolveConventionRequest.Builder builder = ResolveConventionRequest.newBuilder()
                .setProjectPath(projectPath)
                .setPropertyName(propertyName);
            if (preferredSources != null) {
                builder.addAllPreferredSources(preferredSources);
            }
            return client.getPluginStub().resolveConvention(builder.build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:plugin] resolveConvention failed", e);
            return ResolveConventionResponse.getDefaultInstance();
        }
    }
}
