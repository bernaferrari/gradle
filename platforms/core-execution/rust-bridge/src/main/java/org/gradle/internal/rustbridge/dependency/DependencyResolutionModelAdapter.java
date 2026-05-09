package org.gradle.internal.rustbridge.dependency;

import gradle.substrate.v1.RepositoryDescriptor;
import org.gradle.api.Project;
import org.gradle.api.artifacts.ResolvableDependencies;
import org.gradle.api.artifacts.repositories.ArtifactRepository;
import org.gradle.api.artifacts.repositories.MavenArtifactRepository;
import org.gradle.api.logging.Logging;
import org.gradle.internal.service.ServiceRegistry;
import org.jspecify.annotations.Nullable;
import org.slf4j.Logger;

import java.lang.reflect.Method;
import java.net.URI;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.List;

/**
 * Captures the narrow repository contract that Rust static artifact prefetch can represent.
 */
public class DependencyResolutionModelAdapter implements DependencyResolutionShadowListener.RepositoryProvider {
    private static final Logger LOGGER = Logging.getLogger(DependencyResolutionModelAdapter.class);
    private static final String PROJECT_STATE_REGISTRY_CLASS = "org.gradle.api.internal.project.ProjectStateRegistry";

    @Nullable
    private final Object projectStateRegistry;

    public DependencyResolutionModelAdapter(@Nullable Object projectStateRegistry) {
        this.projectStateRegistry = projectStateRegistry;
    }

    public static DependencyResolutionModelAdapter fromServiceRegistry(ServiceRegistry services) {
        return new DependencyResolutionModelAdapter(findProjectStateRegistry(services));
    }

    @Override
    public List<RepositoryDescriptor> repositoriesFor(ResolvableDependencies dependencies) {
        if (projectStateRegistry == null) {
            return Collections.emptyList();
        }
        String projectPath = projectPathFor(dependencies.getPath(), dependencies.getName());
        try {
            for (Object projectState : getAllProjects(projectStateRegistry)) {
                if (!projectPath.equals(nestedString(projectState, "getProjectPath", "getPath"))) {
                    continue;
                }
                if (!booleanValue(projectState, "isCreated")) {
                    return Collections.emptyList();
                }
                Object mutableModel = invoke(projectState, "getMutableModel");
                if (!(mutableModel instanceof Project)) {
                    return Collections.emptyList();
                }
                RepositoryCapture capture = repositoriesForProject((Project) mutableModel);
                return capture.getRepositories();
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] failed to capture repositories for {}", dependencies.getPath(), e);
        }
        return Collections.emptyList();
    }

    @SuppressWarnings("deprecation")
    public static RepositoryCapture repositoriesForProject(Project project) {
        List<RepositoryDescriptor> repositories = new ArrayList<>();
        List<String> unsupportedFeatures = new ArrayList<>();
        for (ArtifactRepository repository : project.getRepositories()) {
            if (!(repository instanceof MavenArtifactRepository)) {
                unsupportedFeatures.add("repository-type:" + repository.getClass().getName());
                continue;
            }
            MavenArtifactRepository maven = (MavenArtifactRepository) repository;
            if (!maven.getArtifactUrls().isEmpty()) {
                unsupportedFeatures.add("maven-artifact-urls:" + maven.getName());
                continue;
            }
            URI url = maven.getUrl();
            String scheme = url == null ? "" : url.getScheme();
            if (!"http".equalsIgnoreCase(scheme) && !"https".equalsIgnoreCase(scheme)) {
                unsupportedFeatures.add("repository-url-scheme:" + maven.getName() + ":" + scheme);
                continue;
            }
            String layout = metadataLayoutFor(maven.getMetadataSources());
            if (layout == null) {
                unsupportedFeatures.add("repository-metadata-sources:" + maven.getName());
                continue;
            }
            RepositoryDescriptor.Builder builder = RepositoryDescriptor.newBuilder()
                .setId(maven.getName())
                .setUrl(url.toString())
                .setM2Compatible(true)
                .setAllowInsecureProtocol(maven.isAllowInsecureProtocol());
            if (!layout.isEmpty()) {
                builder.setLayout(layout);
            }
            repositories.add(builder.build());
        }
        return new RepositoryCapture(repositories, unsupportedFeatures);
    }

    public static final class RepositoryCapture {
        private final List<RepositoryDescriptor> repositories;
        private final List<String> unsupportedFeatures;

        RepositoryCapture(List<RepositoryDescriptor> repositories, List<String> unsupportedFeatures) {
            this.repositories = Collections.unmodifiableList(new ArrayList<>(repositories));
            this.unsupportedFeatures = Collections.unmodifiableList(new ArrayList<>(unsupportedFeatures));
        }

        public List<RepositoryDescriptor> getRepositories() {
            return repositories;
        }

        public List<String> getUnsupportedFeatures() {
            return unsupportedFeatures;
        }
    }

    @Nullable
    static String metadataLayoutFor(MavenArtifactRepository.MetadataSources metadataSources) {
        if (metadataSources.isArtifactEnabled()) {
            return null;
        }
        if (metadataSources.isGradleMetadataEnabled()) {
            return "gradle-module-metadata";
        }
        if (metadataSources.isMavenPomEnabled()) {
            return "";
        }
        return null;
    }

    static String projectPathFor(String resolvablePath, String configurationName) {
        String suffix = ":" + configurationName;
        if (resolvablePath.endsWith(suffix)) {
            String projectPath = resolvablePath.substring(0, resolvablePath.length() - suffix.length());
            return projectPath.isEmpty() ? ":" : projectPath;
        }
        return ":";
    }

    @Nullable
    private static Object findProjectStateRegistry(ServiceRegistry services) {
        try {
            Class<?> registryType = Class.forName(PROJECT_STATE_REGISTRY_CLASS);
            return services.find(registryType);
        } catch (ClassNotFoundException e) {
            LOGGER.debug("[substrate:dep-resolve] ProjectStateRegistry unavailable", e);
            return null;
        }
    }

    private static Collection<Object> getAllProjects(Object projectStateRegistry) {
        return asCollection(invoke(projectStateRegistry, "getAllProjects"));
    }

    private static String nestedString(Object target, String method, String nestedMethod) {
        Object nested = invoke(target, method);
        return nested == null ? "" : stringValue(nested, nestedMethod);
    }

    private static String stringValue(Object target, String method) {
        Object value = invoke(target, method);
        return value == null ? "" : value.toString();
    }

    private static boolean booleanValue(Object target, String method) {
        Object value = invoke(target, method);
        return value instanceof Boolean && (Boolean) value;
    }

    private static Object invoke(Object target, String method) {
        try {
            Method m = target.getClass().getMethod(method);
            m.setAccessible(true);
            return m.invoke(target);
        } catch (Exception e) {
            throw new IllegalStateException("Could not invoke " + method + " on " + target.getClass().getName(), e);
        }
    }

    @SuppressWarnings("unchecked")
    private static Collection<Object> asCollection(@Nullable Object value) {
        if (value instanceof Collection) {
            return (Collection<Object>) value;
        }
        return Collections.emptyList();
    }
}
