package org.gradle.internal.rustbridge.dependency;

import gradle.substrate.v1.DependencyDescriptor;
import gradle.substrate.v1.RepositoryDescriptor;
import org.gradle.api.artifacts.ArtifactCollection;
import org.gradle.api.artifacts.Dependency;
import org.gradle.api.artifacts.DependencyArtifact;
import org.gradle.api.artifacts.DependencyConstraint;
import org.gradle.api.artifacts.DependencyResolutionListener;
import org.gradle.api.artifacts.ExternalModuleDependency;
import org.gradle.api.artifacts.ModuleDependency;
import org.gradle.api.artifacts.ResolvableDependencies;
import org.gradle.api.artifacts.VersionConstraint;
import org.gradle.api.artifacts.component.ComponentArtifactIdentifier;
import org.gradle.api.artifacts.component.ComponentIdentifier;
import org.gradle.api.artifacts.component.ModuleComponentIdentifier;
import org.gradle.api.artifacts.result.ResolvedArtifactResult;
import org.gradle.api.artifacts.result.ResolvedComponentResult;
import org.gradle.api.artifacts.result.ResolutionResult;
import org.gradle.api.artifacts.result.UnresolvedDependencyResult;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.shadow.HashMismatchReporter;
import org.gradle.internal.service.scopes.Scope;
import org.gradle.internal.service.scopes.ServiceScope;
import org.slf4j.Logger;

import java.io.File;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
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
    private final boolean prefetchArtifacts;
    private final RepositoryProvider repositoryProvider;

    // Track resolution start times for timing measurement
    private final Map<String, Long> resolutionStartTimes = new ConcurrentHashMap<>();
    private final java.util.concurrent.atomic.AtomicLong totalResolutionTimeMs =
        new java.util.concurrent.atomic.AtomicLong(0);
    private final java.util.concurrent.atomic.AtomicLong resolutionCount =
        new java.util.concurrent.atomic.AtomicLong(0);
    private final java.util.concurrent.atomic.AtomicLong mirroredArtifactCount =
        new java.util.concurrent.atomic.AtomicLong(0);
    private final java.util.concurrent.atomic.AtomicLong prefetchedArtifactCount =
        new java.util.concurrent.atomic.AtomicLong(0);

    public interface RepositoryProvider {
        List<RepositoryDescriptor> repositoriesFor(ResolvableDependencies dependencies);
    }

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
        this(client, mismatchReporter, authoritative, mirrorArtifacts, false, dependencies -> Collections.emptyList());
    }

    public DependencyResolutionShadowListener(
        RustDependencyResolutionClient client,
        HashMismatchReporter mismatchReporter,
        boolean authoritative,
        boolean mirrorArtifacts,
        boolean prefetchArtifacts,
        RepositoryProvider repositoryProvider
    ) {
        this.client = client;
        this.mismatchReporter = mismatchReporter;
        this.authoritative = authoritative;
        this.mirrorArtifacts = mirrorArtifacts;
        this.prefetchArtifacts = prefetchArtifacts;
        this.repositoryProvider = repositoryProvider;
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
            int prefetchedArtifacts = 0;

            ResolutionResult result = null;
            try {
                result = dependencies.getResolutionResult();
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

            if (prefetchArtifacts && javaSuccess && result != null) {
                prefetchedArtifacts = prefetchStaticMavenArtifacts(dependencies, result);
                prefetchedArtifactCount.addAndGet(prefetchedArtifacts);
            }

            if (mirrorArtifacts && javaSuccess) {
                mirroredArtifacts = mirrorResolvedArtifacts(dependencies);
                mirroredArtifactCount.addAndGet(mirroredArtifacts);
            }

            String source = recordResolutionInMode(configName, durationMs, artifactCount, javaSuccess, failureCount);
            LOGGER.debug(
                "[substrate:dep-resolve] shadow OK: {} ({}ms, {} components, {} failures, {} prefetched artifacts, {} mirrored artifacts, source={})",
                configName, durationMs, artifactCount, failureCount, prefetchedArtifacts, mirroredArtifacts, source
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

    public boolean isPrefetchArtifacts() {
        return prefetchArtifacts;
    }

    public long getMirroredArtifactCount() {
        return mirroredArtifactCount.get();
    }

    public long getPrefetchedArtifactCount() {
        return prefetchedArtifactCount.get();
    }

    private int prefetchStaticMavenArtifacts(ResolvableDependencies dependencies, ResolutionResult result) {
        List<RepositoryDescriptor> repositories = repositoryProvider.repositoriesFor(dependencies);
        if (repositories.size() != 1) {
            return 0;
        }

        List<DependencyDescriptor> declaredDescriptors = staticMavenDependencyDescriptors(dependencies);
        if (declaredDescriptors.isEmpty()) {
            return 0;
        }
        List<DependencyDescriptor> constraintDescriptors = staticMavenDependencyConstraintDescriptors(dependencies);
        if (!resolvedGraphContainsAll(declaredDescriptors, result)) {
            return 0;
        }
        List<DependencyDescriptor> descriptors = staticMavenDependencyDescriptors(result);
        if (descriptors.isEmpty()) {
            return 0;
        }

        RustDependencyResolutionClient.ResolutionResult rustResult = client.resolveDependencies(
            dependencies.getName(),
            descriptors,
            constraintDescriptors,
            repositories,
            false,
            true
        );
        if (!rustResult.isSuccess()) {
            mismatchReporter.reportRustError(
                "dep-artifact-prefetch:" + dependencies.getName(),
                new IllegalStateException(rustResult.getErrorMessage())
            );
            return 0;
        }
        return rustResult.getTotalArtifacts();
    }

    private static boolean resolvedGraphContainsAll(List<DependencyDescriptor> descriptors, ResolutionResult result) {
        List<ModuleComponentIdentifier> resolvedModules = new ArrayList<>();
        for (ResolvedComponentResult component : result.getAllComponents()) {
            ComponentIdentifier id = component.getId();
            if (id instanceof ModuleComponentIdentifier) {
                resolvedModules.add((ModuleComponentIdentifier) id);
            }
        }
        for (DependencyDescriptor descriptor : descriptors) {
            if (!resolvedModules.stream().anyMatch(module ->
                descriptor.getGroup().equals(module.getGroup())
                    && descriptor.getName().equals(module.getModule())
                    && descriptor.getVersion().equals(module.getVersion())
            )) {
                return false;
            }
        }
        return true;
    }

    private List<DependencyDescriptor> staticMavenDependencyDescriptors(ResolutionResult result) {
        List<DependencyDescriptor> descriptors = new ArrayList<>();
        try {
            for (ResolvedComponentResult component : result.getAllComponents()) {
                ComponentIdentifier id = component.getId();
                if (!(id instanceof ModuleComponentIdentifier)) {
                    continue;
                }
                DependencyDescriptor descriptor = staticMavenDependencyDescriptor((ModuleComponentIdentifier) id);
                if (descriptor == null) {
                    return Collections.emptyList();
                }
                if (!containsDescriptor(descriptors, descriptor)) {
                    descriptors.add(descriptor);
                }
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] resolved graph prefetch contract capture failed", e);
            return Collections.emptyList();
        }
        return Collections.unmodifiableList(descriptors);
    }

    private static boolean containsDescriptor(List<DependencyDescriptor> descriptors, DependencyDescriptor candidate) {
        return descriptors.stream().anyMatch(existing ->
            existing.getGroup().equals(candidate.getGroup())
                && existing.getName().equals(candidate.getName())
                && existing.getVersion().equals(candidate.getVersion())
                && existing.getClassifier().equals(candidate.getClassifier())
                && existing.getExtension().equals(candidate.getExtension())
        );
    }

    private List<DependencyDescriptor> staticMavenDependencyDescriptors(ResolvableDependencies dependencies) {
        List<DependencyDescriptor> descriptors = new ArrayList<>();
        try {
            for (Dependency dependency : dependencies.getDependencies()) {
                DependencyDescriptor descriptor = staticMavenDependencyDescriptor(dependency);
                if (descriptor == null) {
                    return Collections.emptyList();
                }
                descriptors.add(descriptor);
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] dependency prefetch contract capture failed", e);
            return Collections.emptyList();
        }
        return Collections.unmodifiableList(descriptors);
    }

    private List<DependencyDescriptor> staticMavenDependencyConstraintDescriptors(ResolvableDependencies dependencies) {
        List<DependencyDescriptor> descriptors = new ArrayList<>();
        try {
            for (DependencyConstraint constraint : dependencies.getDependencyConstraints()) {
                DependencyDescriptor descriptor = staticMavenDependencyConstraintDescriptor(constraint);
                if (descriptor == null) {
                    return Collections.emptyList();
                }
                descriptors.add(descriptor);
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] dependency constraint capture failed", e);
            return Collections.emptyList();
        }
        return Collections.unmodifiableList(descriptors);
    }

    private DependencyDescriptor staticMavenDependencyConstraintDescriptor(DependencyConstraint constraint) {
        String group = constraint.getGroup();
        String name = constraint.getName();
        String version = staticMavenConstraintVersion(constraint.getVersionConstraint());
        if (isBlank(group) || isBlank(name) || isBlank(version) || !isStaticVersion(version)) {
            return null;
        }
        return DependencyDescriptor.newBuilder()
            .setGroup(group)
            .setName(name)
            .setVersion(version)
            .setClassifier("")
            .setExtension("jar")
            .setTransitive(false)
            .build();
    }

    private String staticMavenConstraintVersion(VersionConstraint versionConstraint) {
        if (versionConstraint == null || versionConstraint.getBranch() != null || !versionConstraint.getRejectedVersions().isEmpty()) {
            return null;
        }
        String strictVersion = versionConstraint.getStrictVersion();
        if (!isBlank(strictVersion)) {
            return strictVersion;
        }
        String requiredVersion = versionConstraint.getRequiredVersion();
        if (!isBlank(requiredVersion)) {
            return requiredVersion;
        }
        String preferredVersion = versionConstraint.getPreferredVersion();
        if (!isBlank(preferredVersion)) {
            return preferredVersion;
        }
        return null;
    }

    private DependencyDescriptor staticMavenDependencyDescriptor(Dependency dependency) {
        if (!(dependency instanceof ExternalModuleDependency)) {
            return null;
        }
        ExternalModuleDependency external = (ExternalModuleDependency) dependency;
        if (external.isChanging()) {
            return null;
        }
        ModuleDependency module = (ModuleDependency) external;
        if (module.getTargetConfiguration() != null || !module.getExcludeRules().isEmpty()) {
            return null;
        }

        String group = dependency.getGroup();
        String name = dependency.getName();
        String version = dependency.getVersion();
        if (isBlank(group) || isBlank(name) || isBlank(version) || !isStaticVersion(version)) {
            return null;
        }

        String classifier = "";
        String extension = "jar";
        if (module.getArtifacts().size() > 1) {
            return null;
        }
        if (module.getArtifacts().size() == 1) {
            DependencyArtifact artifact = module.getArtifacts().iterator().next();
            if (artifact.getUrl() != null || !name.equals(artifact.getName())) {
                return null;
            }
            classifier = artifact.getClassifier() == null ? "" : artifact.getClassifier();
            extension = artifact.getExtension() == null ? artifact.getType() : artifact.getExtension();
            if (isBlank(extension)) {
                return null;
            }
        }

        return DependencyDescriptor.newBuilder()
            .setGroup(group)
            .setName(name)
            .setVersion(version)
            .setClassifier(classifier)
            .setExtension(extension)
            .setTransitive(false)
            .build();
    }

    private DependencyDescriptor staticMavenDependencyDescriptor(ModuleComponentIdentifier id) {
        String group = id.getGroup();
        String name = id.getModule();
        String version = id.getVersion();
        if (isBlank(group) || isBlank(name) || isBlank(version) || !isStaticVersion(version)) {
            return null;
        }
        return DependencyDescriptor.newBuilder()
            .setGroup(group)
            .setName(name)
            .setVersion(version)
            .setClassifier("")
            .setExtension("jar")
            .setTransitive(false)
            .build();
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
        if (!file.isFile()) {
            return false;
        }

        String extension = inferExtension(file.getName());
        if (extension.isEmpty()) {
            return false;
        }
        String classifier = inferClassifier(file.getName(), moduleId.getModule(), moduleId.getVersion(), extension);
        try {
            boolean accepted = client.addArtifactToCache(
                moduleId.getGroup(),
                moduleId.getModule(),
                moduleId.getVersion(),
                classifier,
                extension,
                file.getAbsolutePath(),
                file.length(),
                ""
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
        return inferClassifier(fileName, moduleName, version, inferExtension(fileName));
    }

    static String inferClassifier(String fileName, String moduleName, String version, String extension) {
        String prefix = moduleName + "-" + version;
        String suffix = "." + extension;
        if (extension.isEmpty() || !fileName.startsWith(prefix) || !fileName.endsWith(suffix)) {
            return "";
        }
        String classifierSuffix = fileName.substring(prefix.length(), fileName.length() - suffix.length());
        if (classifierSuffix.isEmpty()) {
            return "";
        }
        if (classifierSuffix.startsWith("-")) {
            return classifierSuffix.substring(1);
        }
        return "";
    }

    static String inferExtension(String fileName) {
        int dot = fileName.lastIndexOf('.');
        if (dot < 0 || dot == fileName.length() - 1) {
            return "";
        }
        return fileName.substring(dot + 1);
    }

    private static boolean isBlank(String value) {
        return value == null || value.isEmpty();
    }

    private static boolean isStaticVersion(String version) {
        return version.indexOf('+') < 0
            && version.indexOf('[') < 0
            && version.indexOf(']') < 0
            && version.indexOf('(') < 0
            && version.indexOf(')') < 0
            && !"latest.release".equalsIgnoreCase(version)
            && !"latest.integration".equalsIgnoreCase(version)
            && !"release".equalsIgnoreCase(version)
            && !"latest".equalsIgnoreCase(version)
            && !version.endsWith("-SNAPSHOT");
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
