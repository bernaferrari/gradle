package org.gradle.internal.rustbridge.dependency;

import org.gradle.api.artifacts.repositories.MavenArtifactRepository;
import org.gradle.api.credentials.PasswordCredentials;
import org.junit.Test;

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
}
