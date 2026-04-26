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
import org.gradle.internal.rustbridge.SubstrateException;
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
            throw unavailable("init build layout");
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
            throw failed("init build layout", e);
        }
    }

    public boolean addSubproject(String buildId, String projectPath, String projectDir,
                                  String buildFile, String displayName) {
        if (client.isNoop()) {
            throw unavailable("add subproject");
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
            throw failed("add subproject", e);
        }
    }

    public GetProjectTreeResponse getProjectTree(String buildId) {
        if (client.isNoop()) {
            throw unavailable("get project tree");
        }

        try {
            return client.getBuildLayoutStub()
                .getProjectTree(GetProjectTreeRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:layout] get project tree failed", e);
            throw failed("get project tree", e);
        }
    }

    public GetBuildFilePathResponse getBuildFilePath(String buildId, String projectPath) {
        if (client.isNoop()) {
            throw unavailable("get build file path");
        }

        try {
            return client.getBuildLayoutStub()
                .getBuildFilePath(GetBuildFilePathRequest.newBuilder()
                    .setBuildId(buildId)
                    .setProjectPath(projectPath)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:layout] get build file path failed", e);
            throw failed("get build file path", e);
        }
    }

    public List<String> listProjects(String buildId) {
        if (client.isNoop()) {
            throw unavailable("list projects");
        }

        try {
            ListProjectsResponse response = client.getBuildLayoutStub()
                .listProjects(ListProjectsRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());
            return response.getProjectPathsList();
        } catch (Exception e) {
            LOGGER.debug("[substrate:layout] list projects failed", e);
            throw failed("list projects", e);
        }
    }

    private SubstrateException unavailable(String operation) {
        return new SubstrateException("Rust build layout service is unavailable for " + operation + ": " + client.getNoopReason());
    }

    private SubstrateException failed(String operation, Exception cause) {
        if (cause instanceof SubstrateException) {
            return (SubstrateException) cause;
        }
        return new SubstrateException("Rust build layout service failed to " + operation, cause);
    }
}
