package org.gradle.internal.rustbridge.buildlayout;

import gradle.substrate.v1.BuildLayoutServiceGrpc;
import gradle.substrate.v1.GetBuildFilePathRequest;
import gradle.substrate.v1.GetBuildFilePathResponse;
import gradle.substrate.v1.GetProjectTreeRequest;
import gradle.substrate.v1.GetProjectTreeResponse;
import gradle.substrate.v1.InitBuildLayoutRequest;
import gradle.substrate.v1.InitBuildLayoutResponse;
import gradle.substrate.v1.ListProjectsRequest;
import gradle.substrate.v1.ListProjectsResponse;
import gradle.substrate.v1.AddSubprojectRequest;
import gradle.substrate.v1.AddSubprojectResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;

/**
 * Client for the Rust build layout service.
 * Manages project structure and layout via gRPC.
 */
public class RustBuildLayoutClient {

    private static final Logger LOGGER = Logging.getLogger(RustBuildLayoutClient.class);

    private final SubstrateClient client;

    public RustBuildLayoutClient(SubstrateClient client) {
        this.client = client;
    }

    public InitBuildLayoutResponse initBuildLayout(String rootDir, String settingsFile,
                                                     String buildFile, String buildName) {
        if (client.isNoop()) {
            return InitBuildLayoutResponse.getDefaultInstance();
        }

        try {
            return client.getBuildLayoutStub()
                .initBuildLayout(InitBuildLayoutRequest.newBuilder()
                    .setRootDir(rootDir)
                    .setSettingsFile(settingsFile)
                    .setBuildFile(buildFile)
                    .setBuildName(buildName)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:layout] init build layout failed", e);
            return InitBuildLayoutResponse.getDefaultInstance();
        }
    }

    public boolean addSubproject(String buildId, String projectPath, String projectDir,
                                  String buildFile, String displayName) {
        if (client.isNoop()) {
            return false;
        }

        try {
            AddSubprojectResponse response = client.getBuildLayoutStub()
                .addSubproject(AddSubprojectRequest.newBuilder()
                    .setBuildId(buildId)
                    .setProjectPath(projectPath)
                    .setProjectDir(projectDir)
                    .setBuildFile(buildFile)
                    .setDisplayName(displayName)
                    .build());
            return response.getAdded();
        } catch (Exception e) {
            LOGGER.debug("[substrate:layout] add subproject failed for {}", projectPath, e);
            return false;
        }
    }

    public GetProjectTreeResponse getProjectTree(String buildId) {
        if (client.isNoop()) {
            return GetProjectTreeResponse.getDefaultInstance();
        }

        try {
            return client.getBuildLayoutStub()
                .getProjectTree(GetProjectTreeRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:layout] get project tree failed", e);
            return GetProjectTreeResponse.getDefaultInstance();
        }
    }

    public GetBuildFilePathResponse getBuildFilePath(String buildId, String projectPath) {
        if (client.isNoop()) {
            return GetBuildFilePathResponse.getDefaultInstance();
        }

        try {
            return client.getBuildLayoutStub()
                .getBuildFilePath(GetBuildFilePathRequest.newBuilder()
                    .setBuildId(buildId)
                    .setProjectPath(projectPath)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:layout] get build file path failed", e);
            return GetBuildFilePathResponse.getDefaultInstance();
        }
    }

    public List<String> listProjects(String buildId) {
        if (client.isNoop()) {
            return java.util.Collections.emptyList();
        }

        try {
            ListProjectsResponse response = client.getBuildLayoutStub()
                .listProjects(ListProjectsRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
            return response.getProjectPathsList();
        } catch (Exception e) {
            LOGGER.debug("[substrate:layout] list projects failed", e);
            return java.util.Collections.emptyList();
        }
    }
}
