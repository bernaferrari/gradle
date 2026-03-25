package org.gradle.internal.rustbridge.publishing;

import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.slf4j.Logger;

/**
 * Shadow adapter that monitors JVM artifact publishing against Rust.
 *
 * <p>In shadow mode, shadows artifact registrations in fire-and-forget mode
 * and compares publishing status total counts between JVM and Rust.</p>
 */
public class ShadowingArtifactPublisher {

    private static final Logger LOGGER = Logging.getLogger(ShadowingArtifactPublisher.class);

    private final RustArtifactPublishingClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingArtifactPublisher(
        RustArtifactPublishingClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Shadow an artifact registration in fire-and-forget mode.
     *
     * <p>Registering an artifact has real side effects (repository tracking),
     * so the Rust call is best-effort. The Java result is authoritative.</p>
     *
     * @param buildId     the build identifier
     * @param group       the artifact group
     * @param name        the artifact name
     * @param version     the artifact version
     * @param javaResult  whether the JVM registration succeeded
     */
    public void shadowRegisterArtifact(String buildId, String group, String name,
                                        String version, boolean javaResult) {
        try {
            rustClient.registerArtifact(buildId, group, name, version, "", "", "", 0L, "");
            mismatchReporter.reportMatch();
            LOGGER.debug("[substrate:publishing] shadow registerArtifact (fire-and-forget): buildId={}, {}:{}:{}, success={}",
                buildId, group, name, version, javaResult);
        } catch (Exception e) {
            mismatchReporter.reportRustError("publishing:registerArtifact:" + buildId + ":" + group + ":" + name, e);
            LOGGER.debug("[substrate:publishing] shadow registerArtifact error for buildId={}, {}:{}:{}: {}",
                buildId, group, name, version, e.getMessage());
        }
    }

    /**
     * Shadow a publishing status query, comparing JVM and Rust total artifact counts.
     *
     * <p>Retrieves the Rust publishing status and compares the total count
     * against the JVM value. This catches discrepancies in artifact tracking
     * between the two implementations.</p>
     *
     * @param buildId          the build identifier
     * @param javaTotalCount   the total artifact count from the JVM
     */
    public void shadowGetPublishingStatus(String buildId, int javaTotalCount) {
        try {
            RustArtifactPublishingClient.PublishingStatus rustStatus =
                rustClient.getPublishingStatus(buildId);

            if (rustStatus == null) {
                mismatchReporter.reportRustError(
                    "publishing:status:" + buildId,
                    new RuntimeException("Rust returned null status")
                );
                LOGGER.debug("[substrate:publishing] shadow getPublishingStatus: Rust returned null for buildId={}",
                    buildId);
                return;
            }

            int rustTotalCount = rustStatus.getTotal();

            if (javaTotalCount == rustTotalCount) {
                mismatchReporter.reportMatch();
                LOGGER.debug("[substrate:publishing] shadow getPublishingStatus MATCH: buildId={}, count={}",
                    buildId, javaTotalCount);
            } else {
                mismatchReporter.reportMismatch(
                    "publishing:status:totalCount:" + buildId,
                    String.valueOf(javaTotalCount),
                    String.valueOf(rustTotalCount)
                );
                LOGGER.debug("[substrate:publishing] shadow getPublishingStatus MISMATCH: buildId={}, java={}, rust={}",
                    buildId, javaTotalCount, rustTotalCount);
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("publishing:status:" + buildId, e);
            LOGGER.debug("[substrate:publishing] shadow getPublishingStatus error for buildId={}: {}",
                buildId, e.getMessage());
        }
    }
}
