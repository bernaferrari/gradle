package org.gradle.internal.rustbridge.dependency;

import gradle.substrate.v1.DependencyDescriptor;
import gradle.substrate.v1.RepositoryDescriptor;

import org.gradle.api.Project;
import org.gradle.api.Action;
import org.gradle.api.artifacts.DependencyConstraint;
import org.gradle.api.artifacts.VersionConstraint;
import org.gradle.api.artifacts.dsl.RepositoryHandler;
import org.gradle.api.artifacts.repositories.ArtifactRepository;
import org.gradle.api.artifacts.repositories.MavenArtifactRepository;
import org.gradle.api.artifacts.repositories.PasswordCredentials;
import org.junit.Test;

import java.lang.reflect.Proxy;
import java.net.URI;
import java.util.Arrays;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

public class DependencyResolutionModelAdapterTest {
    @Test
    public void metadataLayoutForMavenPomOnlyKeepsDefaultLayout() {
        assertEquals("", DependencyResolutionModelAdapter.metadataLayoutFor(sources(false, true, false)));
    }

    @Test
    public void metadataLayoutForGradleMetadataEnablesRustModuleMetadataPath() {
        assertEquals(
            "gradle-module-metadata",
            DependencyResolutionModelAdapter.metadataLayoutFor(sources(true, true, false))
        );
    }

    @Test
    public void metadataLayoutForArtifactOnlyFailsClosed() {
        assertNull(DependencyResolutionModelAdapter.metadataLayoutFor(sources(false, false, true)));
    }

    @Test
    public void metadataLayoutForEmptyMetadataSourcesFailsClosed() {
        assertNull(DependencyResolutionModelAdapter.metadataLayoutFor(sources(false, false, false)));
    }

    @Test
    public void emptyCredentialsAreAnonymous() {
        assertFalse(DependencyResolutionModelAdapter.hasConfiguredCredentials(credentials(null, null)));
        assertFalse(DependencyResolutionModelAdapter.hasConfiguredCredentials(credentials(" ", "\t")));
    }

    @Test
    public void configuredUsernameOrPasswordRequiresUnsupportedMarker() {
        assertTrue(DependencyResolutionModelAdapter.hasConfiguredCredentials(credentials("user", null)));
        assertTrue(DependencyResolutionModelAdapter.hasConfiguredCredentials(credentials(null, "secret")));
    }

    @Test
    public void durableRepositoryCaptureStillRejectsCredentials() {
        DependencyResolutionModelAdapter.RepositoryCapture capture =
            DependencyResolutionModelAdapter.repositoriesForProject(
                projectWithRepository(mavenRepository("private", "https://repo.example.test/maven", credentials("user", "secret"))),
                false
            );

        assertTrue(capture.getRepositories().isEmpty());
        assertEquals(Collections.singletonList("repository-credentials:private"), capture.getUnsupportedFeatures());
    }

    @Test
    public void sessionRepositoryCaptureCarriesCredentialsForNativeTransportOnly() {
        DependencyResolutionModelAdapter.RepositoryCapture capture =
            DependencyResolutionModelAdapter.repositoriesForProject(
                projectWithRepository(mavenRepository("private", "https://repo.example.test/maven", credentials("user", "secret"))),
                true
            );

        assertTrue(capture.getUnsupportedFeatures().isEmpty());
        RepositoryDescriptor repository = capture.getRepositories().get(0);
        assertEquals("private", repository.getId());
        assertEquals("user", repository.getCredentialsMap().get("username"));
        assertEquals("secret", repository.getCredentialsMap().get("password"));
    }

    @Test
    public void repositoryContentFilterFailsClosedForDurableCapture() {
        DependencyResolutionModelAdapter.RepositoryCapture capture =
            DependencyResolutionModelAdapter.repositoriesForProject(
                projectWithRepository(filteredMavenRepository(
                    "filtered",
                    "https://repo.example.test/maven",
                    repositoryContentFilter(),
                    Collections.emptySet(),
                    Collections.emptySet(),
                    Collections.emptyMap()
                )),
                false
            );

        assertTrue(capture.getRepositories().isEmpty());
        assertEquals(Collections.singletonList("repository-content-filter:filtered"), capture.getUnsupportedFeatures());
    }

    @Test
    public void repositoryContentConfigurationsAndAttributesFailClosedForSessionCaptureToo() {
        DependencyResolutionModelAdapter.RepositoryCapture capture =
            DependencyResolutionModelAdapter.repositoriesForProject(
                projectWithRepository(filteredMavenRepository(
                    "filtered",
                    "https://repo.example.test/maven",
                    null,
                    Collections.singleton("runtimeClasspath"),
                    Collections.singleton("compileClasspath"),
                    Collections.singletonMap("usage", Collections.singleton("java-runtime"))
                )),
                true
            );

        assertTrue(capture.getRepositories().isEmpty());
        assertEquals(
            Arrays.asList("repository-content-configurations:filtered", "repository-content-attributes:filtered"),
            capture.getUnsupportedFeatures()
        );
    }

    @Test
    public void staticRepositoryGroupFiltersAreCapturedAsNativeContract() {
        DependencyResolutionModelAdapter.RepositoryContentCapture content =
            DependencyResolutionModelAdapter.repositoryContentCapture(
                new RepositoryWithContentSpecs(
                    setOf(
                        new ContentSpec("SIMPLE", "com.acme", null, null, true),
                        new ContentSpec("SIMPLE", "com.acme", "api", null, true),
                        new ContentSpec("SIMPLE", "com.acme", "api", "1.0", true),
                        new ContentSpec("SIMPLE", "org.example", null, null, true),
                        new ContentSpec("SUB_GROUP", "net.demo", null, null, true)
                    ),
                    setOf(
                        new ContentSpec("SIMPLE", "com.acme.internal", null, null, false),
                        new ContentSpec("SIMPLE", "com.acme", "secret", null, false),
                        new ContentSpec("SIMPLE", "com.acme", "secret", "1.0", false),
                        new ContentSpec("SUB_GROUP", "net.demo.internal", null, null, false)
                    )
                ),
                "filtered"
            );

        assertEquals(Arrays.asList("com.acme", "org.example"), content.getIncludeGroups());
        assertEquals(Collections.singletonList("com.acme.internal"), content.getExcludeGroups());
        assertEquals(Collections.singletonList("net.demo"), content.getIncludeGroupPrefixes());
        assertEquals(Collections.singletonList("net.demo.internal"), content.getExcludeGroupPrefixes());
        assertEquals(Collections.singletonList("com.acme:api"), content.getIncludeModules());
        assertEquals(Collections.singletonList("com.acme:secret"), content.getExcludeModules());
        assertEquals(Collections.singletonList("com.acme:api:1.0"), content.getIncludeModuleVersions());
        assertEquals(Collections.singletonList("com.acme:secret:1.0"), content.getExcludeModuleVersions());
        assertTrue(content.getUnsupportedFeatures().isEmpty());
    }

    @Test
    public void nonGroupRepositoryContentSpecsRemainUnsupported() {
        DependencyResolutionModelAdapter.RepositoryContentCapture content =
            DependencyResolutionModelAdapter.repositoryContentCapture(
                new RepositoryWithContentSpecs(
                    setOf(new ContentSpec("REGEX", "com\\..*", null, null, true)),
                    Collections.emptySet()
                ),
                "filtered"
            );

        assertTrue(content.getIncludeGroups().isEmpty());
        assertTrue(content.getUnsupportedFeatures().contains("repository-content-filter:filtered"));
    }

    @Test
    public void repositoryContentSpecsWithRegexVersionRemainUnsupported() {
        DependencyResolutionModelAdapter.RepositoryContentCapture content =
            DependencyResolutionModelAdapter.repositoryContentCapture(
                new RepositoryWithContentSpecs(
                    setOf(new ContentSpec("REGEX", "com\\.acme", "api", "1\\..*", true)),
                    Collections.emptySet()
                ),
                "filtered"
            );

        assertTrue(content.getIncludeModuleVersions().isEmpty());
        assertTrue(content.getUnsupportedFeatures().contains("repository-content-filter:filtered"));
    }

    @Test
    public void repositoryMetadataSupplierAndVersionListerFieldsFailClosed() {
        assertEquals(
            Arrays.asList("repository-metadata-supplier:custom", "repository-version-lister:custom"),
            DependencyResolutionModelAdapter.unsupportedMetadataRuleMarkers(new RepositoryWithMetadataRules(), "custom")
        );
    }

    @Test
    public void absentRepositoryMetadataRuleFieldsAreSupported() {
        assertTrue(DependencyResolutionModelAdapter.unsupportedMetadataRuleMarkers(new Object(), "plain").isEmpty());
        assertFalse(DependencyResolutionModelAdapter.hasNonNullFieldInHierarchy(new RepositoryWithoutMetadataRules(), "componentMetadataSupplierRuleClass"));
    }

    @Test
    public void dependencyConstraintDescriptorPreservesStaticRichVersionFields() {
        DependencyDescriptor descriptor = DependencyResolutionShadowListener.staticMavenDependencyConstraintDescriptor(
            dependencyConstraint("org.example", "demo", versionConstraint("2.0", "1.5", "1.4", null, Collections.singletonList("1.3")))
        );

        assertEquals("org.example", descriptor.getGroup());
        assertEquals("demo", descriptor.getName());
        assertEquals("2.0", descriptor.getVersion());
        assertEquals("2.0", descriptor.getStrictVersion());
        assertEquals("1.5", descriptor.getRequiredVersion());
        assertEquals("1.4", descriptor.getPreferredVersion());
        assertEquals(Collections.singletonList("1.3"), descriptor.getRejectedVersionsList());
    }

    @Test
    public void dependencyConstraintDescriptorRejectsDynamicRichRejects() {
        assertNull(DependencyResolutionShadowListener.staticMavenDependencyConstraintDescriptor(
            dependencyConstraint("org.example", "demo", versionConstraint("", "1.5", "", null, Collections.singletonList("1.+")))
        ));
    }

    private static MavenArtifactRepository.MetadataSources sources(
        boolean gradleMetadata,
        boolean mavenPom,
        boolean artifact
    ) {
        return new MavenArtifactRepository.MetadataSources() {
            @Override
            public void gradleMetadata() {
            }

            @Override
            public void mavenPom() {
            }

            @Override
            public void artifact() {
            }

            @Override
            public void ignoreGradleMetadataRedirection() {
            }

            @Override
            public boolean isGradleMetadataEnabled() {
                return gradleMetadata;
            }

            @Override
            public boolean isMavenPomEnabled() {
                return mavenPom;
            }

            @Override
            public boolean isArtifactEnabled() {
                return artifact;
            }

            @Override
            public boolean isIgnoreGradleMetadataRedirectionEnabled() {
                return false;
            }
        };
    }

    private static PasswordCredentials credentials(String username, String password) {
        return new PasswordCredentials() {
            @Override
            public String getUsername() {
                return username;
            }

            @Override
            public void setUsername(String userName) {
                throw new UnsupportedOperationException();
            }

            @Override
            public String getPassword() {
                return password;
            }

            @Override
            public void setPassword(String password) {
                throw new UnsupportedOperationException();
            }
        };
    }

    private static Project projectWithRepository(ArtifactRepository repository) {
        RepositoryHandler repositories = proxy(RepositoryHandler.class, (proxy, method, args) -> {
            if (method.getName().equals("iterator")) {
                return Collections.singletonList(repository).iterator();
            }
            return defaultValue(method.getReturnType());
        });
        return proxy(Project.class, (proxy, method, args) -> {
            if (method.getName().equals("getRepositories")) {
                return repositories;
            }
            return defaultValue(method.getReturnType());
        });
    }

    private static MavenArtifactRepository mavenRepository(String name, String url, PasswordCredentials credentials) {
        return proxy(MavenArtifactRepository.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getName":
                    return name;
                case "getUrl":
                    return URI.create(url);
                case "getArtifactUrls":
                    return Collections.emptySet();
                case "getCredentials":
                    return credentials;
                case "getMetadataSources":
                    return sources(false, true, false);
                case "isAllowInsecureProtocol":
                    return false;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static MavenArtifactRepository filteredMavenRepository(
        String name,
        String url,
        Action<Object> contentFilter,
        Set<String> includedConfigurations,
        Set<String> excludedConfigurations,
        Map<?, ?> requiredAttributes
    ) {
        return proxy(new Class<?>[]{MavenArtifactRepository.class, TestContentFilteringRepository.class}, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getName":
                    return name;
                case "getUrl":
                    return URI.create(url);
                case "getArtifactUrls":
                    return Collections.emptySet();
                case "getCredentials":
                    return credentials(null, null);
                case "getMetadataSources":
                    return sources(false, true, false);
                case "isAllowInsecureProtocol":
                    return false;
                case "getContentFilter":
                    return contentFilter;
                case "getIncludedConfigurations":
                    return includedConfigurations;
                case "getExcludedConfigurations":
                    return excludedConfigurations;
                case "getRequiredAttributes":
                    return requiredAttributes;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static Action<Object> repositoryContentFilter() {
        return details -> {
        };
    }

    private interface TestContentFilteringRepository {
        Action<Object> getContentFilter();

        Set<String> getIncludedConfigurations();

        Set<String> getExcludedConfigurations();

        Map<?, ?> getRequiredAttributes();
    }

    private static class RepositoryWithContentSpecs {
        @SuppressWarnings("unused")
        private final Set<ContentSpec> includeSpecs;

        @SuppressWarnings("unused")
        private final Set<ContentSpec> excludeSpecs;

        RepositoryWithContentSpecs(Set<ContentSpec> includeSpecs, Set<ContentSpec> excludeSpecs) {
            this.includeSpecs = includeSpecs;
            this.excludeSpecs = excludeSpecs;
        }
    }

    private static class ContentSpec {
        @SuppressWarnings("unused")
        private final Object matcherKind;

        @SuppressWarnings("unused")
        private final String group;

        @SuppressWarnings("unused")
        private final String module;

        @SuppressWarnings("unused")
        private final String version;

        @SuppressWarnings("unused")
        private final boolean inclusive;

        ContentSpec(Object matcherKind, String group, String module, String version, boolean inclusive) {
            this.matcherKind = matcherKind;
            this.group = group;
            this.module = module;
            this.version = version;
            this.inclusive = inclusive;
        }
    }

    private static class RepositoryWithoutMetadataRules {
        @SuppressWarnings("unused")
        private Object componentMetadataSupplierRuleClass;
    }

    private static class RepositoryWithMetadataRules extends RepositoryWithoutMetadataRules {
        @SuppressWarnings("unused")
        private final Object componentMetadataSupplierRuleClass = Object.class;

        @SuppressWarnings("unused")
        private final Object componentMetadataListerRuleClass = Object.class;
    }

    private static DependencyConstraint dependencyConstraint(String group, String name, VersionConstraint versionConstraint) {
        return proxy(DependencyConstraint.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getGroup":
                    return group;
                case "getName":
                    return name;
                case "getVersionConstraint":
                    return versionConstraint;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    private static VersionConstraint versionConstraint(
        String strict,
        String required,
        String preferred,
        String branch,
        List<String> rejected
    ) {
        return proxy(VersionConstraint.class, (proxy, method, args) -> {
            switch (method.getName()) {
                case "getStrictVersion":
                    return strict;
                case "getRequiredVersion":
                    return required;
                case "getPreferredVersion":
                    return preferred;
                case "getBranch":
                    return branch;
                case "getRejectedVersions":
                    return rejected;
                default:
                    return defaultValue(method.getReturnType());
            }
        });
    }

    @SuppressWarnings("unchecked")
    private static <T> T proxy(Class<T> type, java.lang.reflect.InvocationHandler handler) {
        return (T) Proxy.newProxyInstance(type.getClassLoader(), new Class<?>[]{type}, handler);
    }

    @SuppressWarnings("unchecked")
    private static <T> T proxy(Class<?>[] types, java.lang.reflect.InvocationHandler handler) {
        return (T) Proxy.newProxyInstance(types[0].getClassLoader(), types, handler);
    }

    private static Object defaultValue(Class<?> type) {
        if (type.equals(boolean.class)) {
            return false;
        }
        if (type.equals(int.class)) {
            return 0;
        }
        if (type.equals(long.class)) {
            return 0L;
        }
        if (type.equals(float.class)) {
            return 0f;
        }
        if (type.equals(double.class)) {
            return 0d;
        }
        if (type.equals(void.class)) {
            return null;
        }
        if (type.equals(String.class)) {
            return "";
        }
        if (List.class.isAssignableFrom(type)) {
            return Collections.emptyList();
        }
        if (Iterable.class.isAssignableFrom(type)) {
            return Collections.emptyList();
        }
        return null;
    }

    private static Set<ContentSpec> setOf(ContentSpec... values) {
        Set<ContentSpec> result = new HashSet<>();
        Collections.addAll(result, values);
        return result;
    }
}
