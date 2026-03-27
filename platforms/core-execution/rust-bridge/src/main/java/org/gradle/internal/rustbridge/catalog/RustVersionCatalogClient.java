package org.gradle.internal.rustbridge.catalog;

import org.gradle.substrate.v1.*;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * Client for the Rust version catalog service.
 * Parses Gradle version catalogs (libs.versions.toml) via gRPC,
 * returning typed versions, libraries, bundles, and plugins.
 */
public class RustVersionCatalogClient {

    private static final Logger LOGGER = Logging.getLogger(RustVersionCatalogClient.class);

    private final SubstrateClient client;

    public RustVersionCatalogClient(SubstrateClient client) {
        this.client = client;
    }

    /**
     * Result of parsing a version catalog.
     */
    public static class ParseResult {
        private final List<VersionEntry> versions;
        private final List<LibraryEntry> libraries;
        private final List<BundleEntry> bundles;
        private final List<PluginEntry> plugins;
        private final boolean success;
        private final String errorMessage;

        private ParseResult(List<VersionEntry> versions, List<LibraryEntry> libraries,
                            List<BundleEntry> bundles, List<PluginEntry> plugins,
                            boolean success, String errorMessage) {
            this.versions = versions;
            this.libraries = libraries;
            this.bundles = bundles;
            this.plugins = plugins;
            this.success = success;
            this.errorMessage = errorMessage;
        }

        public List<VersionEntry> getVersions() { return versions; }
        public List<LibraryEntry> getLibraries() { return libraries; }
        public List<BundleEntry> getBundles() { return bundles; }
        public List<PluginEntry> getPlugins() { return plugins; }
        public boolean isSuccess() { return success; }
        public String getErrorMessage() { return errorMessage; }

        public static ParseResult empty() {
            return new ParseResult(Collections.emptyList(), Collections.emptyList(),
                Collections.emptyList(), Collections.emptyList(), false, "Substrate not available");
        }

        public static ParseResult failure(String message) {
            return new ParseResult(Collections.emptyList(), Collections.emptyList(),
                Collections.emptyList(), Collections.emptyList(), false, message);
        }
    }

    /**
     * A version alias from the catalog.
     */
    public static class VersionEntry {
        private final String alias;
        private final String version;

        private VersionEntry(String alias, String version) {
            this.alias = alias;
            this.version = version;
        }

        public String getAlias() { return alias; }
        public String getVersion() { return version; }
    }

    /**
     * A library alias from the catalog.
     */
    public static class LibraryEntry {
        private final String alias;
        private final String group;
        private final String artifact;
        private final String versionRef;
        private final String versionLiteral;

        private LibraryEntry(String alias, String group, String artifact,
                             String versionRef, String versionLiteral) {
            this.alias = alias;
            this.group = group;
            this.artifact = artifact;
            this.versionRef = versionRef;
            this.versionLiteral = versionLiteral;
        }

        public String getAlias() { return alias; }
        public String getGroup() { return group; }
        public String getArtifact() { return artifact; }
        public String getVersionRef() { return versionRef; }
        public String getVersionLiteral() { return versionLiteral; }
    }

    /**
     * A bundle alias from the catalog.
     */
    public static class BundleEntry {
        private final String alias;
        private final List<String> libraryAliases;

        private BundleEntry(String alias, List<String> libraryAliases) {
            this.alias = alias;
            this.libraryAliases = libraryAliases;
        }

        public String getAlias() { return alias; }
        public List<String> getLibraryAliases() { return libraryAliases; }
    }

    /**
     * A plugin alias from the catalog.
     */
    public static class PluginEntry {
        private final String alias;
        private final String id;
        private final String versionRef;
        private final String versionLiteral;

        private PluginEntry(String alias, String id, String versionRef, String versionLiteral) {
            this.alias = alias;
            this.id = id;
            this.versionRef = versionRef;
            this.versionLiteral = versionLiteral;
        }

        public String getAlias() { return alias; }
        public String getId() { return id; }
        public String getVersionRef() { return versionRef; }
        public String getVersionLiteral() { return versionLiteral; }
    }

    /**
     * Parses a version catalog from TOML content.
     *
     * @param tomlContent the raw TOML content of the version catalog file
     * @return parsed result with versions, libraries, bundles, and plugins
     */
    public ParseResult parseVersionCatalog(String tomlContent) {
        if (client.isNoop()) {
            return ParseResult.empty();
        }

        try {
            ParseVersionCatalogRequest request = ParseVersionCatalogRequest.newBuilder()
                .setContent(tomlContent)
                .build();

            ParseVersionCatalogResponse response = client.getVersionCatalogStub()
                .parseVersionCatalog(request);

            List<VersionEntry> versions = new ArrayList<>();
            for (ProtoVersion pv : response.getVersionsList()) {
                versions.add(new VersionEntry(pv.getAlias(), pv.getVersion()));
            }

            List<LibraryEntry> libraries = new ArrayList<>();
            for (ProtoLibrary pl : response.getLibrariesList()) {
                libraries.add(new LibraryEntry(
                    pl.getAlias(), pl.getGroup(), pl.getArtifact(),
                    pl.getVersionRef(), pl.getVersionLiteral()
                ));
            }

            List<BundleEntry> bundles = new ArrayList<>();
            for (ProtoBundle pb : response.getBundlesList()) {
                bundles.add(new BundleEntry(pb.getAlias(), pb.getLibraryAliasesList()));
            }

            List<PluginEntry> plugins = new ArrayList<>();
            for (ProtoPlugin pp : response.getPluginsList()) {
                plugins.add(new PluginEntry(
                    pp.getAlias(), pp.getId(), pp.getVersionRef(), pp.getVersionLiteral()
                ));
            }

            LOGGER.debug("[substrate:catalog] parsed catalog: {} versions, {} libraries, {} bundles, {} plugins",
                versions.size(), libraries.size(), bundles.size(), plugins.size());

            return new ParseResult(versions, libraries, bundles, plugins, true, "");
        } catch (Exception e) {
            LOGGER.debug("[substrate:catalog] gRPC call failed", e);
            return ParseResult.failure("gRPC error: " + e.getMessage());
        }
    }
}
