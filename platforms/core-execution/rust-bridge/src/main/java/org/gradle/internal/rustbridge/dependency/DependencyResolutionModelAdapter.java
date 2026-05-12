package org.gradle.internal.rustbridge.dependency;

import gradle.substrate.v1.RepositoryDescriptor;
import org.gradle.api.Project;
import org.gradle.api.artifacts.ResolvableDependencies;
import org.gradle.api.artifacts.repositories.ArtifactRepository;
import org.gradle.api.artifacts.repositories.MavenArtifactRepository;
import org.gradle.api.credentials.PasswordCredentials;
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
import java.util.Map;
import java.util.Set;

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
                RepositoryCapture capture = repositoriesForProject((Project) mutableModel, true);
                return capture.getRepositories();
            }
        } catch (Exception e) {
            LOGGER.debug("[substrate:dep-resolve] failed to capture repositories for {}", dependencies.getPath(), e);
        }
        return Collections.emptyList();
    }

    @SuppressWarnings("deprecation")
    public static RepositoryCapture repositoriesForProject(Project project) {
        return repositoriesForProject(project, false);
    }

    @SuppressWarnings("deprecation")
    static RepositoryCapture repositoriesForProject(Project project, boolean includeSessionCredentials) {
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
            RepositoryContentCapture contentCapture = repositoryContentCapture(maven);
            List<String> contentMarkers = contentCapture.getUnsupportedFeatures();
            if (!contentMarkers.isEmpty()) {
                unsupportedFeatures.addAll(contentMarkers);
                continue;
            }
            List<String> metadataRuleMarkers = unsupportedMetadataRuleMarkers(maven, maven.getName());
            if (!metadataRuleMarkers.isEmpty()) {
                unsupportedFeatures.addAll(metadataRuleMarkers);
                continue;
            }
            if (hasConfiguredCredentials(maven.getCredentials())) {
                if (!includeSessionCredentials) {
                    unsupportedFeatures.add("repository-credentials:" + maven.getName());
                    continue;
                }
            }
            URI url = maven.getUrl();
            String scheme = url == null ? "" : url.getScheme();
            if (!"http".equalsIgnoreCase(scheme) && !"https".equalsIgnoreCase(scheme) && !"file".equalsIgnoreCase(scheme)) {
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
            builder.addAllIncludeGroups(contentCapture.getIncludeGroups());
            builder.addAllExcludeGroups(contentCapture.getExcludeGroups());
            builder.addAllIncludeGroupPrefixes(contentCapture.getIncludeGroupPrefixes());
            builder.addAllExcludeGroupPrefixes(contentCapture.getExcludeGroupPrefixes());
            builder.addAllIncludeModules(contentCapture.getIncludeModules());
            builder.addAllExcludeModules(contentCapture.getExcludeModules());
            builder.addAllIncludeModuleVersions(contentCapture.getIncludeModuleVersions());
            builder.addAllExcludeModuleVersions(contentCapture.getExcludeModuleVersions());
            if (includeSessionCredentials && hasConfiguredCredentials(maven.getCredentials())) {
                builder.putCredentials("username", nullToEmpty(maven.getCredentials().getUsername()));
                builder.putCredentials("password", nullToEmpty(maven.getCredentials().getPassword()));
            }
            if (!layout.isEmpty()) {
                builder.setLayout(layout);
            }
            repositories.add(builder.build());
        }
        return new RepositoryCapture(repositories, unsupportedFeatures);
    }

    static boolean hasConfiguredCredentials(PasswordCredentials credentials) {
        return hasText(credentials.getUsername()) || hasText(credentials.getPassword());
    }

    private static boolean hasText(@Nullable String value) {
        return value != null && !value.trim().isEmpty();
    }

    private static String nullToEmpty(@Nullable String value) {
        return value == null ? "" : value;
    }

    static RepositoryContentCapture repositoryContentCapture(Object repository, String name) {
        List<String> unsupported = new ArrayList<>();
        List<String> includeGroups = new ArrayList<>();
        List<String> excludeGroups = new ArrayList<>();
        List<String> includeGroupPrefixes = new ArrayList<>();
        List<String> excludeGroupPrefixes = new ArrayList<>();
        List<String> includeModules = new ArrayList<>();
        List<String> excludeModules = new ArrayList<>();
        List<String> includeModuleVersions = new ArrayList<>();
        List<String> excludeModuleVersions = new ArrayList<>();
        if (!extractContentSpecs(readFieldInHierarchy(repository, "includeSpecs"), true, includeGroups, includeGroupPrefixes, includeModules, includeModuleVersions)) {
            unsupported.add("repository-content-filter:" + name);
        }
        if (!extractContentSpecs(readFieldInHierarchy(repository, "excludeSpecs"), false, excludeGroups, excludeGroupPrefixes, excludeModules, excludeModuleVersions)) {
            unsupported.add("repository-content-filter:" + name);
        }
        if (nonEmptyCollection(invokeIfPresent(repository, "getIncludedConfigurations"))
            || nonEmptyCollection(invokeIfPresent(repository, "getExcludedConfigurations"))) {
            unsupported.add("repository-content-configurations:" + name);
        }
        if (nonEmptyMap(invokeIfPresent(repository, "getRequiredAttributes"))) {
            unsupported.add("repository-content-attributes:" + name);
        }
        return new RepositoryContentCapture(includeGroups, excludeGroups, includeGroupPrefixes, excludeGroupPrefixes, includeModules, excludeModules, includeModuleVersions, excludeModuleVersions, unsupported);
    }

    private static RepositoryContentCapture repositoryContentCapture(MavenArtifactRepository repository) {
        return repositoryContentCapture(repository, repository.getName());
    }

    private static boolean extractContentSpecs(
        @Nullable Object specs,
        boolean expectedInclusive,
        List<String> groups,
        List<String> groupPrefixes,
        List<String> modules,
        List<String> moduleVersions
    ) {
        if (specs == null) {
            return true;
        }
        if (!(specs instanceof Set)) {
            return false;
        }
        for (Object spec : (Set<?>) specs) {
            if (isGroupSpec(spec, expectedInclusive, "SIMPLE")) {
                Object group = readFieldInHierarchy(spec, "group");
                groups.add(group.toString());
            } else if (isModuleSpec(spec, expectedInclusive, "SIMPLE")) {
                Object group = readFieldInHierarchy(spec, "group");
                Object module = readFieldInHierarchy(spec, "module");
                modules.add(group + ":" + module);
            } else if (isModuleVersionSpec(spec, expectedInclusive, "SIMPLE")) {
                Object group = readFieldInHierarchy(spec, "group");
                Object module = readFieldInHierarchy(spec, "module");
                Object version = readFieldInHierarchy(spec, "version");
                moduleVersions.add(group + ":" + module + ":" + version);
            } else if (isGroupSpec(spec, expectedInclusive, "SUB_GROUP")) {
                Object group = readFieldInHierarchy(spec, "group");
                groupPrefixes.add(group.toString());
            } else {
                return false;
            }
        }
        Collections.sort(groups);
        Collections.sort(groupPrefixes);
        Collections.sort(modules);
        Collections.sort(moduleVersions);
        return true;
    }

    private static boolean isGroupSpec(Object spec, boolean expectedInclusive, String expectedMatcherKind) {
        Object matcherKind = readFieldInHierarchy(spec, "matcherKind");
        Object group = readFieldInHierarchy(spec, "group");
        Object module = readFieldInHierarchy(spec, "module");
        Object version = readFieldInHierarchy(spec, "version");
        Object inclusive = readFieldInHierarchy(spec, "inclusive");
        return matcherKind != null
            && expectedMatcherKind.equals(matcherKind.toString())
            && group != null
            && module == null
            && version == null
            && Boolean.valueOf(expectedInclusive).equals(inclusive);
    }

    private static boolean isModuleVersionSpec(Object spec, boolean expectedInclusive, String expectedMatcherKind) {
        Object matcherKind = readFieldInHierarchy(spec, "matcherKind");
        Object group = readFieldInHierarchy(spec, "group");
        Object module = readFieldInHierarchy(spec, "module");
        Object version = readFieldInHierarchy(spec, "version");
        Object inclusive = readFieldInHierarchy(spec, "inclusive");
        return matcherKind != null
            && expectedMatcherKind.equals(matcherKind.toString())
            && group != null
            && module != null
            && version != null
            && Boolean.valueOf(expectedInclusive).equals(inclusive);
    }

    private static boolean isModuleSpec(Object spec, boolean expectedInclusive, String expectedMatcherKind) {
        Object matcherKind = readFieldInHierarchy(spec, "matcherKind");
        Object group = readFieldInHierarchy(spec, "group");
        Object module = readFieldInHierarchy(spec, "module");
        Object version = readFieldInHierarchy(spec, "version");
        Object inclusive = readFieldInHierarchy(spec, "inclusive");
        return matcherKind != null
            && expectedMatcherKind.equals(matcherKind.toString())
            && group != null
            && module != null
            && version == null
            && Boolean.valueOf(expectedInclusive).equals(inclusive);
    }

    private static boolean nonEmptyCollection(@Nullable Object value) {
        return value instanceof Collection && !((Collection<?>) value).isEmpty();
    }

    private static boolean nonEmptyMap(@Nullable Object value) {
        return value instanceof Map && !((Map<?, ?>) value).isEmpty();
    }

    static List<String> unsupportedMetadataRuleMarkers(Object repository, String name) {
        List<String> markers = new ArrayList<>();
        if (hasNonNullFieldInHierarchy(repository, "componentMetadataSupplierRuleClass")) {
            markers.add("repository-metadata-supplier:" + name);
        }
        if (hasNonNullFieldInHierarchy(repository, "componentMetadataListerRuleClass")) {
            markers.add("repository-version-lister:" + name);
        }
        return markers;
    }

    static boolean hasNonNullFieldInHierarchy(Object target, String fieldName) {
        return readFieldInHierarchy(target, fieldName) != null;
    }

    @Nullable
    private static Object readFieldInHierarchy(Object target, String fieldName) {
        Class<?> type = target.getClass();
        while (type != null) {
            try {
                java.lang.reflect.Field field = type.getDeclaredField(fieldName);
                field.setAccessible(true);
                return field.get(target);
            } catch (NoSuchFieldException e) {
                type = type.getSuperclass();
            } catch (Exception e) {
                throw new IllegalStateException("Could not read " + fieldName + " on " + target.getClass().getName(), e);
            }
        }
        return null;
    }

    static final class RepositoryContentCapture {
        private final List<String> includeGroups;
        private final List<String> excludeGroups;
        private final List<String> includeGroupPrefixes;
        private final List<String> excludeGroupPrefixes;
        private final List<String> includeModules;
        private final List<String> excludeModules;
        private final List<String> includeModuleVersions;
        private final List<String> excludeModuleVersions;
        private final List<String> unsupportedFeatures;

        RepositoryContentCapture(
            List<String> includeGroups,
            List<String> excludeGroups,
            List<String> includeGroupPrefixes,
            List<String> excludeGroupPrefixes,
            List<String> includeModules,
            List<String> excludeModules,
            List<String> includeModuleVersions,
            List<String> excludeModuleVersions,
            List<String> unsupportedFeatures
        ) {
            this.includeGroups = Collections.unmodifiableList(new ArrayList<>(includeGroups));
            this.excludeGroups = Collections.unmodifiableList(new ArrayList<>(excludeGroups));
            this.includeGroupPrefixes = Collections.unmodifiableList(new ArrayList<>(includeGroupPrefixes));
            this.excludeGroupPrefixes = Collections.unmodifiableList(new ArrayList<>(excludeGroupPrefixes));
            this.includeModules = Collections.unmodifiableList(new ArrayList<>(includeModules));
            this.excludeModules = Collections.unmodifiableList(new ArrayList<>(excludeModules));
            this.includeModuleVersions = Collections.unmodifiableList(new ArrayList<>(includeModuleVersions));
            this.excludeModuleVersions = Collections.unmodifiableList(new ArrayList<>(excludeModuleVersions));
            this.unsupportedFeatures = Collections.unmodifiableList(new ArrayList<>(unsupportedFeatures));
        }

        List<String> getIncludeGroups() {
            return includeGroups;
        }

        List<String> getExcludeGroups() {
            return excludeGroups;
        }

        List<String> getIncludeGroupPrefixes() {
            return includeGroupPrefixes;
        }

        List<String> getExcludeGroupPrefixes() {
            return excludeGroupPrefixes;
        }

        List<String> getIncludeModules() {
            return includeModules;
        }

        List<String> getExcludeModules() {
            return excludeModules;
        }

        List<String> getIncludeModuleVersions() {
            return includeModuleVersions;
        }

        List<String> getExcludeModuleVersions() {
            return excludeModuleVersions;
        }

        List<String> getUnsupportedFeatures() {
            return unsupportedFeatures;
        }
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

    @Nullable
    private static Object invokeIfPresent(Object target, String method) {
        try {
            Method m = target.getClass().getMethod(method);
            m.setAccessible(true);
            return m.invoke(target);
        } catch (NoSuchMethodException e) {
            return null;
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
