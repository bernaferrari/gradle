package org.gradle.internal.rustbridge.buildlayout;

import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;

import java.util.List;

/**
 * Shadow adapter that compares JVM build layout data with Rust.
 *
 * <p>Tracks project structure (root dir, settings file, subprojects)
 * and compares against the Rust BuildLayoutService.</p>
 */
public class ShadowingBuildLayoutTracker {

    private final RustBuildLayoutClient rustClient;
    private final HashMismatchReporter mismatchReporter;

    public ShadowingBuildLayoutTracker(
        RustBuildLayoutClient rustClient,
        HashMismatchReporter mismatchReporter
    ) {
        this.rustClient = rustClient;
        this.mismatchReporter = mismatchReporter;
    }

    /**
     * Compare JVM project list with Rust project list.
     */
    public void compareProjectList(String buildId, List<String> javaProjectPaths) {
        try {
            List<String> rustProjectPaths = rustClient.listProjects(buildId);

            if (rustProjectPaths.size() != javaProjectPaths.size()) {
                mismatchReporter.reportMismatch(
                    "build-layout:projects:" + buildId,
                    String.valueOf(javaProjectPaths.size()),
                    String.valueOf(rustProjectPaths.size())
                );
                return;
            }

            boolean allMatch = true;
            for (int i = 0; i < javaProjectPaths.size(); i++) {
                if (!javaProjectPaths.get(i).equals(rustProjectPaths.get(i))) {
                    allMatch = false;
                    break;
                }
            }

            if (allMatch) {
                mismatchReporter.reportMatch();
            } else {
                mismatchReporter.reportMismatch(
                    "build-layout:projects:" + buildId,
                    String.join(",", javaProjectPaths),
                    String.join(",", rustProjectPaths)
                );
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("build-layout:projects:" + buildId, e);
        }
    }

    /**
     * Compare JVM build file path with Rust build file path.
     */
    public void compareBuildFilePath(String buildId, String projectPath,
                                      String javaBuildFile) {
        try {
            gradle.substrate.v1.GetBuildFilePathResponse rustResponse =
                rustClient.getBuildFilePath(buildId, projectPath);
            String rustBuildFile = rustResponse.getBuildFilePath();

            if (javaBuildFile.equals(rustBuildFile)) {
                mismatchReporter.reportMatch();
            } else {
                mismatchReporter.reportMismatch(
                    "build-layout:buildFile:" + buildId + ":" + projectPath,
                    javaBuildFile,
                    rustBuildFile
                );
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError(
                "build-layout:buildFile:" + buildId + ":" + projectPath, e);
        }
    }
}
