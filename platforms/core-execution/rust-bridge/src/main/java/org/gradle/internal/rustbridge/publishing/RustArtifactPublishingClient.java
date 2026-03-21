package org.gradle.internal.rustbridge.publishing;

import gradle.substrate.v1.ArtifactDescriptor;
import gradle.substrate.v1.ArtifactPublishingServiceGrpc;
import gradle.substrate.v1.GetArtifactChecksumsRequest;
import gradle.substrate.v1.GetArtifactChecksumsResponse;
import gradle.substrate.v1.GetPublishingStatusRequest;
import gradle.substrate.v1.GetPublishingStatusResponse;
import gradle.substrate.v1.RecordUploadResultRequest;
import gradle.substrate.v1.RecordUploadResultResponse;
import gradle.substrate.v1.RegisterArtifactRequest;
import gradle.substrate.v1.RegisterArtifactResponse;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.List;
import java.util.Map;
import java.util.stream.Collectors;

/**
 * Client for the Rust artifact publishing service.
 * Tracks artifact registrations, upload results, and checksums via gRPC.
 */
public class RustArtifactPublishingClient {

    private static final Logger LOGGER = Logging.getLogger(RustArtifactPublishingClient.class);

    private final SubstrateClient client;

    public RustArtifactPublishingClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of registering an artifact.
     */
    public static class RegistrationResult {
        private final boolean accepted;

        private RegistrationResult(boolean accepted) {
            this.accepted = accepted;
        }

        public boolean isAccepted() { return accepted; }
    }

    /**
     * Publishing status for a build.
     */
    public static class PublishingStatus {
        private final int total;
        private final int uploaded;
        private final int failed;
        private final int pending;
        private final List<GetPublishingStatusResponse.ArtifactPublishStatus> artifacts;

        private PublishingStatus(int total, int uploaded, int failed, int pending,
                                 List<GetPublishingStatusResponse.ArtifactPublishStatus> artifacts) {
            this.total = total;
            this.uploaded = uploaded;
            this.failed = failed;
            this.pending = pending;
            this.artifacts = artifacts;
        }

        public int getTotal() { return total; }
        public int getUploaded() { return uploaded; }
        public int getFailed() { return failed; }
        public int getPending() { return pending; }
        public List<GetPublishingStatusResponse.ArtifactPublishStatus> getArtifacts() { return artifacts; }
    }

    /**
     * Register an artifact for publishing.
     */
    public RegistrationResult registerArtifact(String buildId, String group, String name,
                                                String version, String classifier, String extension,
                                                String filePath, long fileSizeBytes, String repositoryId) {
        if (client.isNoop()) {
            return new RegistrationResult(false);
        }

        try {
            ArtifactDescriptor descriptor = ArtifactDescriptor.newBuilder()
                .setArtifactId(buildId + ":" + group + ":" + name + ":" + version + ":" + classifier)
                .setGroup(group)
                .setName(name)
                .setVersion(version)
                .setClassifier(classifier)
                .setExtension(extension)
                .setFilePath(filePath)
                .setFileSizeBytes(fileSizeBytes)
                .setRepositoryId(repositoryId)
                .build();

            RegisterArtifactResponse response = client.getArtifactPublishingStub()
                .registerArtifact(RegisterArtifactRequest.newBuilder()
                    .setBuildId(buildId)
                    .setArtifact(descriptor)
                    .build());

            LOGGER.debug("[substrate:publishing] registered artifact {}:{}:{}:{}",
                group, name, version, classifier);
            return new RegistrationResult(response.getAccepted());
        } catch (Exception e) {
            LOGGER.debug("[substrate:publishing] register artifact failed", e);
            return new RegistrationResult(false);
        }
    }

    /**
     * Record the result of an artifact upload.
     */
    public void recordUploadResult(String artifactId, boolean success, String errorMessage,
                                    long uploadDurationMs, long bytesTransferred) {
        if (client.isNoop()) {
            return;
        }

        try {
            client.getArtifactPublishingStub()
                .recordUploadResult(RecordUploadResultRequest.newBuilder()
                    .setArtifactId(artifactId)
                    .setSuccess(success)
                    .setErrorMessage(errorMessage != null ? errorMessage : "")
                    .setUploadDurationMs(uploadDurationMs)
                    .setBytesTransferred(bytesTransferred)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:publishing] record upload result failed", e);
        }
    }

    /**
     * Get publishing status for a build.
     */
    public PublishingStatus getPublishingStatus(String buildId) {
        if (client.isNoop()) {
            return new PublishingStatus(0, 0, 0, 0, List.of());
        }

        try {
            GetPublishingStatusResponse response = client.getArtifactPublishingStub()
                .getPublishingStatus(GetPublishingStatusRequest.newBuilder()
                    .setBuildId(buildId)
                    .build());

            return new PublishingStatus(
                response.getTotal(),
                response.getUploaded(),
                response.getFailed(),
                response.getPending(),
                response.getArtifactsList()
            );
        } catch (Exception e) {
            LOGGER.debug("[substrate:publishing] get publishing status failed", e);
            return new PublishingStatus(0, 0, 0, 0, List.of());
        }
    }

    /**
     * Get checksums for an artifact.
     */
    public GetArtifactChecksumsResponse getArtifactChecksums(String artifactId) {
        if (client.isNoop()) {
            return GetArtifactChecksumsResponse.getDefaultInstance();
        }

        try {
            return client.getArtifactPublishingStub()
                .getArtifactChecksums(GetArtifactChecksumsRequest.newBuilder()
                    .setArtifactId(artifactId)
                    .build());
        } catch (Exception e) {
            LOGGER.debug("[substrate:publishing] get artifact checksums failed", e);
            return GetArtifactChecksumsResponse.getDefaultInstance();
        }
    }
}
