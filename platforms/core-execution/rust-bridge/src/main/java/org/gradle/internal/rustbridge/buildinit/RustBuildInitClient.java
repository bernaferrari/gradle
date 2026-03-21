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
            return InitBuildSettingsResponse.getDefaultInstance();
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
            return InitBuildSettingsResponse.getDefaultInstance();
        }
    }

    public boolean recordSettingsDetail(String buildId, String key, String value) {
        if (client.isNoop()) {
            return false;
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
            return false;
        }
    }

    public GetBuildInitStatusResponse getBuildInitStatus(String buildId) {
        if (client.isNoop()) {
            return GetBuildInitStatusResponse.getDefaultInstance();
        }

        try {
            return client.getBuildInitStub()
                .getBuildInitStatus(GetBuildInitStatusRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:buildinit] get build init status failed", e);
            return GetBuildInitStatusResponse.getDefaultInstance();
        }
    }

    public boolean recordInitScript(String buildId, String scriptPath,
                                     boolean success, String errorMessage, long durationMs) {
        if (client.isNoop()) {
            return false;
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
            return false;
        }
    }
}
