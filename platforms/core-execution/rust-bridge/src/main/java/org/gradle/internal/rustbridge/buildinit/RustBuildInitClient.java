package org.gradle.internal.rustbridge.buildinit;

import gradle.substrate.v1.BuildInitServiceGrpc;
import gradle.substrate.v1.GetBuildInitStatusRequest;
import gradle.substrate.v1.GetBuildInitStatusResponse;
import gradle.substrate.v1.InitBuildSettingsRequest;
import gradle.substrate.v1.InitBuildSettingsResponse;
import gradle.substrate.v1.RecordInitScriptRequest;
import gradle.substrate.v1.RecordInitScriptResponse;
import gradle.substrate.v1.RecordSettingsDetailRequest;
import gradle.substrate.v1.RecordSettingsDetailResponse;
import gradle.substrate.v1.SettingsDetailEntry;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.gradle.internal.rustbridge.SubstrateException;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust build init service.
 * Tracks build initialization, settings, and init scripts via gRPC.
 */
public class RustBuildInitClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildInitClient.class);

    private final SubstrateClient client;

    public RustBuildInitClient(SubstrateClient client) {
        this.client = client;
    }

    public InitBuildSettingsResponse initBuildSettings(String buildId, String rootDir,
                                                        String settingsFile, String gradleUserHome,
                                                        List<String> initScripts,
                                                        List<String> requestedBuildFeatures,
                                                        String currentDir) {
        if (client.isNoop()) {
            throw unavailable("init build settings");
        }

        try {
            return client.getBuildInitStub()
                .initBuildSettings(InitBuildSettingsRequest.newBuilder()
                    .setBuildId(buildId)
                    .setRootDir(rootDir)
                    .setSettingsFile(settingsFile)
                    .setGradleUserHome(gradleUserHome)
                    .addAllInitScripts(initScripts)
                    .addAllRequestedBuildFeatures(requestedBuildFeatures)
                    .setCurrentDir(currentDir)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildinit] init build settings failed", e);
            throw failed("init build settings", e);
        }
    }

    public boolean recordSettingsDetail(String buildId, String key, String value) {
        if (client.isNoop()) {
            throw unavailable("record settings detail");
        }

        try {
            SettingsDetailEntry detail = SettingsDetailEntry.newBuilder()
                .setKey(key)
                .setValue(value)
                .build();

            RecordSettingsDetailResponse response = client.getBuildInitStub()
                .recordSettingsDetail(RecordSettingsDetailRequest.newBuilder()
                    .setBuildId(buildId)
                    .setDetail(detail)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildinit] record settings detail failed", e);
            throw failed("record settings detail", e);
        }
    }

    public GetBuildInitStatusResponse getBuildInitStatus(String buildId) {
        if (client.isNoop()) {
            throw unavailable("get build init status");
        }

        try {
            return client.getBuildInitStub()
                .getBuildInitStatus(GetBuildInitStatusRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildinit] get build init status failed", e);
            throw failed("get build init status", e);
        }
    }

    public boolean recordInitScript(String buildId, String scriptPath,
                                     boolean success, String errorMessage, long durationMs) {
        if (client.isNoop()) {
            throw unavailable("record init script");
        }

        try {
            RecordInitScriptResponse response = client.getBuildInitStub()
                .recordInitScript(RecordInitScriptRequest.newBuilder()
                    .setBuildId(buildId)
                    .setScriptPath(scriptPath)
                    .setSuccess(success)
                    .setErrorMessage(errorMessage != null ? errorMessage : "")
                    .setDurationMs(durationMs)
                    .build());
            return response.getAccepted();
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildinit] record init script failed", e);
            throw failed("record init script", e);
        }
    }

    private SubstrateException unavailable(String operation) {
        return new SubstrateException("Rust build init service is unavailable for " + operation + ": " + client.getNoopReason());
    }

    private SubstrateException failed(String operation, Exception cause) {
        if (cause instanceof SubstrateException) {
            return (SubstrateException) cause;
        }
        return new SubstrateException("Rust build init service failed to " + operation, cause);
    }
}
