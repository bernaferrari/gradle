package org.gradle.internal.rustbridge.plugin;

import gradle.substrate.v1.ApplyPluginRequest;
import gradle.substrate.v1.ApplyPluginResponse;
import gradle.substrate.v1.GetAppliedPluginsRequest;
import gradle.substrate.v1.GetAppliedPluginsResponse;
import gradle.substrate.v1.HasPluginRequest;
import gradle.substrate.v1.HasPluginResponse;
import gradle.substrate.v1.PluginInfo;
import gradle.substrate.v1.PluginServiceGrpc;
import gradle.substrate.v1.RegisterPluginRequest;
import gradle.substrate.v1.RegisterPluginResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;

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
            return java.util.Collections.emptyList();
        }
    }
}
