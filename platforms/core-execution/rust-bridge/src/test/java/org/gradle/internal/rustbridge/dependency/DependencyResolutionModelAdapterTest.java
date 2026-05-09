package org.gradle.internal.rustbridge.dependency;

import gradle.substrate.v1.DependencyDescriptor;

import org.gradle.api.artifacts.DependencyConstraint;
import org.gradle.api.artifacts.VersionConstraint;
import org.gradle.api.artifacts.repositories.MavenArtifactRepository;
import org.gradle.api.credentials.PasswordCredentials;
import org.junit.Test;

import java.lang.reflect.Proxy;
import java.util.Collections;
import java.util.List;

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
        return null;
    }
}
