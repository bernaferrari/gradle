package org.gradle.internal.rustbridge.bootstrap;

import gradle.substrate.v1.BootstrapServiceGrpc;
import gradle.substrate.v1.BuildPlan;
import gradle.substrate.v1.BuildPlanProject;
import gradle.substrate.v1.BuildPlanTask;
import gradle.substrate.v1.CompleteBuildRequest;
import gradle.substrate.v1.CompleteBuildResponse;
import gradle.substrate.v1.GetSubstrateInfoRequest;
import gradle.substrate.v1.GetSubstrateInfoResponse;
import gradle.substrate.v1.HealthCheckRequest;
import gradle.substrate.v1.HealthCheckResponse;
import gradle.substrate.v1.InitBuildRequest;
import gradle.substrate.v1.InitBuildResponse;
import gradle.substrate.v1.RefreshBuildPlanShadowRequest;
import gradle.substrate.v1.RefreshBuildPlanShadowResponse;
import org.gradle.api.Task;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;
import org.slf4j.Logger;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Client for the Rust bootstrap service.
 * Manages build session lifecycle (init/complete), health checks, and substrate info via gRPC.
 */
@ServiceScope(Scope.Build.class)
public class RustBootstrapClient {

    private static final Logger LOGGER = Logging.getLogger(RustBootstrapClient.class);

    private final SubstrateClient client;

    public RustBootstrapClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of initializing a build session.
     */
    public static class BuildInitResult {
        private final String buildId;
        private final String substrateVersion;
        private final String protocolVersion;
        private final int maxParallelism;

        private BuildInitResult(String buildId, String substrateVersion,
                                String protocolVersion, int maxParallelism) {
            this.buildId = buildId;
            this.substrateVersion = substrateVersion;
            this.protocolVersion = protocolVersion;
            this.maxParallelism = maxParallelism;
        }

        public String getBuildId() { return buildId; }
        public String getSubstrateVersion() { return substrateVersion; }
        public String getProtocolVersion() { return protocolVersion; }
        public int getMaxParallelism() { return maxParallelism; }
    }

    /**
     * Health check result.
     */
    public static class HealthStatus {
        private final boolean healthy;
        private final String version;
        private final String uptime;
        private final long activeBuilds;

        private HealthStatus(boolean healthy, String version, String uptime, long activeBuilds) {
            this.healthy = healthy;
            this.version = version;
            this.uptime = uptime;
            this.activeBuilds = activeBuilds;
        }

        public boolean isHealthy() { return healthy; }
        public String getVersion() { return version; }
        public String getUptime() { return uptime; }
        public long getActiveBuilds() { return activeBuilds; }
    }

    /**
     * Initialize a build session with the substrate daemon.
     */
    public BuildInitResult initBuild(String buildId, String projectDir, long startTimeMs,
                                      int requestedParallelism, Map<String, String> systemProperties,
                                      List<String> requestedFeatures) {
        if (client.isNoop()) {
            return new BuildInitResult(buildId, "noop", "0", requestedParallelism);
        }

        try {
            InitBuildResponse response = client.getBootstrapStub()
                .initBuild(InitBuildRequest.newBuilder()
                    .setBuildId(buildId)
                    .setProjectDir(projectDir)
                    .setStartTimeMs(startTimeMs)
                    .setRequestedParallelism(requestedParallelism)
                    .putAllSystemProperties(systemProperties)
                    .addAllRequestedFeatures(requestedFeatures)
                    .build());

            LOGGER.info("[substrate:bootstrap] build {} initialized (substrate={}, protocol={}, maxParallelism={})",
                response.getBuildId(), response.getSubstrateVersion(),
                response.getProtocolVersion(), response.getMaxParallelism());
            return new BuildInitResult(
                response.getBuildId(),
                response.getSubstrateVersion(),
                response.getProtocolVersion(),
                response.getMaxParallelism()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:bootstrap] init build failed", e);
            return new BuildInitResult(buildId, "error", "0", requestedParallelism);
        }
    }

    /**
     * Notify the substrate daemon that a build has completed.
     */
    public boolean completeBuild(String buildId, String outcome, long durationMs) {
        if (client.isNoop()) {
            return false;
        }

        try {
            CompleteBuildResponse response = client.getBootstrapStub()
                .completeBuild(CompleteBuildRequest.newBuilder()
                    .setBuildId(buildId)
                    .setOutcome(outcome)
                    .setDurationMs(durationMs)
                    .build());

            LOGGER.info("[substrate:bootstrap] build {} completed ({}, {}ms, acked={})",
                buildId, outcome, durationMs, response.getAcknowledged());
            return response.getAcknowledged();
        } catch (Exception e) {
            LOGGER.debug("[substrate:bootstrap] complete build failed", e);
            return false;
        }
    }

    /**
     * Refresh the persisted build-plan shadow artifact after Gradle has populated
     * the selected task graph.
     */
    public boolean refreshBuildPlanShadow(String buildId) {
        return refreshBuildPlanShadow(buildId, null);
    }

    public boolean refreshBuildPlanShadow(
        String buildId,
        List<BuildPlanTask> taskContracts,
        List<Task> taskReferences,
        String source,
        String taskSource
    ) {
        return refreshBuildPlanShadow(
            buildId,
            inlineBuildPlan(buildId, taskContracts, taskReferences, source, taskSource)
        );
    }

    public boolean refreshBuildPlanShadow(String buildId, BuildPlan inlinePlan) {
        if (client.isNoop()) {
            return false;
        }

        try {
            RefreshBuildPlanShadowRequest.Builder request = RefreshBuildPlanShadowRequest.newBuilder()
                .setBuildId(buildId != null ? buildId : "");
            if (inlinePlan != null) {
                request.setInlinePlan(inlinePlan);
            }
            RefreshBuildPlanShadowResponse response = client.getBootstrapStub()
                .refreshBuildPlanShadow(request.build());

            if (response.getRefreshed()) {
                LOGGER.debug("[substrate:bootstrap] refreshed build-plan shadow for {} at {}",
                    response.getBuildId(), response.getArtifactPath());
            } else {
                LOGGER.warn("[substrate:bootstrap] build-plan shadow refresh skipped for {}: {}",
                    response.getBuildId(), response.getErrorMessage());
            }
            return response.getRefreshed();
        } catch (Exception e) {
            LOGGER.warn("[substrate:bootstrap] refresh build-plan shadow failed", e);
            return false;
        }
    }

    private static BuildPlan inlineBuildPlan(
        String buildId,
        List<BuildPlanTask> taskContracts,
        List<Task> taskReferences,
        String source,
        String taskSource
    ) {
        BuildPlan.Builder plan = BuildPlan.newBuilder()
            .setSchemaVersion(2)
            .setBuildId(buildId != null ? buildId : "")
            .addAllTasks(taskContracts)
            .putMetadata("source", source)
            .putMetadata("taskSource", taskSource)
            .putMetadata("selectedTaskReferenceCount", Integer.toString(taskReferences.size()))
            .putMetadata("selectedTaskContractCount", Integer.toString(taskContracts.size()))
            .putMetadata("dependencyCount", "0")
            .putMetadata("dependencyCapture", "deferred");

        Map<String, String> projectNamesByPath = new LinkedHashMap<>();
        for (BuildPlanTask task : taskContracts) {
            String projectPath = task.getProjectPath();
            if (projectPath == null || projectPath.isEmpty()) {
                continue;
            }
            projectNamesByPath.putIfAbsent(projectPath, projectName(projectPath));
        }
        for (Map.Entry<String, String> project : projectNamesByPath.entrySet()) {
            plan.addProjects(BuildPlanProject.newBuilder()
                .setPath(project.getKey())
                .setName(project.getValue())
                .build());
        }
        return plan.build();
    }

    private static String projectName(String projectPath) {
        if (":".equals(projectPath)) {
            return "root";
        }
        int lastSeparator = projectPath.lastIndexOf(':');
        return lastSeparator >= 0 ? projectPath.substring(lastSeparator + 1) : projectPath;
    }

    /**
     * Check the health of the substrate daemon.
     */
    public HealthStatus healthCheck() {
        if (client.isNoop()) {
            return new HealthStatus(false, "noop", "0", 0);
        }

        try {
            HealthCheckResponse response = client.getBootstrapStub()
                .healthCheck(HealthCheckRequest.newBuilder().build());

            return new HealthStatus(
                response.getHealthy(),
                response.getVersion(),
                response.getUptime(),
                response.getActiveBuilds()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:bootstrap] health check failed", e);
            return new HealthStatus(false, "error", "0", 0);
        }
    }

    /**
     * Get information about the substrate daemon and its services.
     */
    public GetSubstrateInfoResponse getSubstrateInfo() {
        if (client.isNoop()) {
            return GetSubstrateInfoResponse.getDefaultInstance();
        }

        try {
            return client.getBootstrapStub()
                .getSubstrateInfo(GetSubstrateInfoRequest.newBuilder().build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:bootstrap] get substrate info failed", e);
            return GetSubstrateInfoResponse.getDefaultInstance();
        }
    }
}
