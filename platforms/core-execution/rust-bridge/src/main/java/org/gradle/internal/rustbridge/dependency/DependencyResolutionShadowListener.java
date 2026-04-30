package org.gradle.internal.rustbridge.dependency;

import org.gradle.api.artifacts.ArtifactCollection;
import org.gradle.api.artifacts.DependencyResolutionListener;
import org.gradle.api.artifacts.ResolvableDependencies;
import org.gradle.api.artifacts.component.ComponentArtifactIdentifier;
import org.gradle.api.artifacts.component.ComponentIdentifier;
import org.gradle.api.artifacts.component.ModuleComponentIdentifier;
import org.gradle.api.artifacts.result.ResolvedArtifactResult;
import org.gradle.api.artifacts.result.ResolutionResult;
import org.gradle.api.artifacts.result.UnresolvedDependencyResult;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;
import org.slf4j.Logger;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.concurrent.ConcurrentHashMap;
import java.util.Map;

/**
 * A {@link DependencyResolutionListener} that shadows dependency resolution results
 * to the Rust substrate. Tracks resolution timing, resolved artifact counts, and
 * compares resolution success between Java and Rust.
 */
@ServiceScope(Scope.Build.class)
public class DependencyResolutionShadowListener implements DependencyResolutionListener {

    private static final Logger LOGGER = Logging.getLogger(DependencyResolutionShadowListener.class);

    private final RustDependencyResolutionClient client;
    private final HashMismatchReporter mismatchReporter;
    private final boolean authoritative;
    private final boolean mirrorArtifacts;

    // Track resolution start times for timing measurement
    private final Map<String, Long> resolutionStartTimes = new ConcurrentHashMap<>();
    private final java.util.concurrent.atomic.AtomicLong totalResolutionTimeMs =
        new java.util.concurrent.atomic.AtomicLong(0);
    private final java.util.concurrent.atomic.AtomicLong resolutionCount =
        new java.util.concurrent.atomic.AtomicLong(0);
    private final java.util.concurrent.atomic.AtomicLong mirroredArtifactCount =
        new java.util.concurrent.atomic.AtomicLong(0);

    public DependencyResolutionShadowListener(
        RustDependencyResolutionClient client,
        HashMismatchReporter mismatchReporter
    ) {
        this(client, mismatchReporter, false);
    }

    public DependencyResolutionShadowListener(
        RustDependencyResolutionClient client,
        HashMismatchReporter mismatchReporter,
        boolean authoritative
    ) {
        this(client, mismatchReporter, authoritative, false);
    }

    public DependencyResolutionShadowListener(
        RustDependencyResolutionClient client,
        HashMismatchReporter mismatchReporter,
        boolean authoritative,
        boolean mirrorArtifacts
    ) {
        this.client = client;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
        this.mirrorArtifacts = mirrorArtifacts;
    }

    @Override
    public void beforeResolve(ResolvableDependencies dependencies) {
        if (client == null) {
            return;
        }

        try {
            String configName = dependencies.getName();
            resolutionStartTimes.put(configName, System.currentTimeMillis());
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] beforeResolve failed", e);
        }
    }

    @Override
    public void afterResolve(ResolvableDependencies dependencies) {
        if (client == null) {
            return;
        }

        try {
            String configName = dependencies.getName();
            Long startTime = resolutionStartTimes.remove(configName);
            long durationMs = startTime != null
                ? System.currentTimeMillis() - startTime
                : 0;

            resolutionCount.incrementAndGet();
            totalResolutionTimeMs.addAndGet(durationMs);

            // Extract resolution details
            boolean javaSuccess = true;
            int artifactCount = 0;
            int failureCount = 0;
            int mirroredArtifacts = 0;

            try {
                ResolutionResult result = dependencies.getResolutionResult();
                javaSuccess = result != null;

                if (result != null) {
                    artifactCount = result.getAllComponents().size();
                    failureCount = result.getAllDependencies().stream()
                        .mapToInt(dep -> dep instanceof UnresolvedDependencyResult ? 1 : 0)
                        .sum();
                }
            } catch (Exception e) {
                // Resolution may not be complete yet
                LOGGER.debug("[substrate:dep-resolve] could not extract resolution result", e);
            }

            if (mirrorArtifacts && javaSuccess) {
                mirroredArtifacts = mirrorResolvedArtifacts(dependencies);
                mirroredArtifactCount.addAndGet(mirroredArtifacts);
            }

            String source = recordResolutionInMode(configName, durationMs, artifactCount, javaSuccess, failureCount);
            LOGGER.debug(
                "[substrate:dep-resolve] shadow OK: {} ({}ms, {} components, {} failures, {} mirrored artifacts, source={})",
                configName, durationMs, artifactCount, failureCount, mirroredArtifacts, source
            );
        } catch (Exception e) {
            mismatchReporter.reportRustError(
                "dep-resolve:" + dependencies.getName(),
                e
            );
            LOGGER.debug("[substrate:dep-resolve] shadow resolution failed", e);
        }
    }

    /**
     * Get the total time spent in dependency resolution across all configurations.
     */
    public long getTotalResolutionTimeMs() {
        return totalResolutionTimeMs.get();
    }

    /**
     * Get the number of configurations resolved.
     */
    public long getResolutionCount() {
        return resolutionCount.get();
    }

    public boolean isAuthoritative() {
        return authoritative;
    }

    public boolean isMirrorArtifacts() {
        return mirrorArtifacts;
    }

    public long getMirroredArtifactCount() {
        return mirroredArtifactCount.get();
    }

    private int mirrorResolvedArtifacts(ResolvableDependencies dependencies) {
        int mirrored = 0;
        try {
            ArtifactCollection artifacts = dependencies.getArtifacts();
            for (ResolvedArtifactResult artifact : artifacts.getArtifacts()) {
                if (mirrorResolvedArtifact(artifact)) {
                    mirrored++;
                }
            }
        } catch (Exception e) {
            mismatchReporter.reportRustError("dep-artifact-mirror:" + dependencies.getName(), e);
            LOGGER.debug("[substrate:dep-resolve] artifact mirror failed", e);
        }
        return mirrored;
    }

    private boolean mirrorResolvedArtifact(ResolvedArtifactResult artifact) {
        ComponentArtifactIdentifier artifactId = artifact.getId();
        ComponentIdentifier componentId = artifactId.getComponentIdentifier();
        if (!(componentId instanceof ModuleComponentIdentifier)) {
            return false;
        }
        ModuleComponentIdentifier moduleId = (ModuleComponentIdentifier) componentId;

        File file = artifact.getFile();
        if (!file.isFile() || !file.getName().endsWith(".jar")) {
            return false;
        }

        String classifier = inferClassifier(file.getName(), moduleId.getModule(), moduleId.getVersion());
        try {
            String sha256 = sha256(file);
            boolean accepted = client.addArtifactToCache(
                moduleId.getGroup(),
                moduleId.getModule(),
                moduleId.getVersion(),
                classifier,
                file.getAbsolutePath(),
                file.length(),
                sha256
            );
            if (!accepted) {
                mismatchReporter.reportRustError(
                    "dep-artifact-mirror:" + artifactId.getDisplayName(),
                    new IllegalStateException("Rust artifact cache rejected artifact")
                );
            }
            return accepted;
        } catch (Exception e) {
            mismatchReporter.reportRustError("dep-artifact-mirror:" + artifactId.getDisplayName(), e);
            LOGGER.debug("[substrate:dep-resolve] failed to mirror artifact {}", artifactId.getDisplayName(), e);
            return false;
        }
    }

    static String inferClassifier(String fileName, String moduleName, String version) {
        String prefix = moduleName + "-" + version;
        if (!fileName.startsWith(prefix) || !fileName.endsWith(".jar")) {
            return "";
        }
        String suffix = fileName.substring(prefix.length(), fileName.length() - ".jar".length());
        if (suffix.isEmpty()) {
            return "";
        }
        if (suffix.startsWith("-")) {
            return suffix.substring(1);
        }
        return "";
    }

    static String sha256(File file) throws IOException {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            byte[] buffer = new byte[64 * 1024];
            try (FileInputStream input = new FileInputStream(file)) {
                int read;
                while ((read = input.read(buffer)) >= 0) {
                    if (read > 0) {
                        digest.update(buffer, 0, read);
                    }
                }
            }
            return toHex(digest.digest());
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException("SHA-256 digest is not available", e);
        }
    }

    private static String toHex(byte[] bytes) {
        StringBuilder out = new StringBuilder(bytes.length * 2);
        for (byte value : bytes) {
            out.append(Character.forDigit((value >> 4) & 0x0f, 16));
            out.append(Character.forDigit(value & 0x0f, 16));
        }
        return out.toString();
    }

    private String recordResolutionInMode(
        String configName,
        long durationMs,
        int artifactCount,
        boolean javaSuccess,
        long failureCount
    ) {
        if (authoritative) {
            try {
                client.recordResolutionStrict(configName, durationMs, artifactCount, javaSuccess, failureCount);
                mismatchReporter.reportMatch();
                return "rust";
            } catch (Exception e) {
                mismatchReporter.reportRustError("dep-resolve:" + configName, e);
                return "rust-error";
            }
        }

        client.recordResolution(configName, durationMs, artifactCount, javaSuccess, failureCount);
        mismatchReporter.reportMatch();
        return "java-shadow";
    }
}
