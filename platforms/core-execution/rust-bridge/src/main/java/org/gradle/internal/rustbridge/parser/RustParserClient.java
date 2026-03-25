package org.gradle.internal.rustbridge.parser;

import gradle.substrate.v1.*;
import org.gradle.api.logging.Logging;
import org.gradle.internal.rustbridge.SubstrateClient;
import org.slf4j.Logger;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.stream.Collectors;

/**
 * Client for the Rust parser service.
 * Provides methods for parsing Groovy DSL scripts and extracting
 * build script elements (dependencies, plugins, repositories, tasks, source sets).
 *
 * <p>Wraps the gRPC {@link ParserServiceGrpc} blocking stub and converts
 * proto responses into plain Java DTOs.</p>
 */
public class RustParserClient {

    private static final Logger LOGGER = Logging.getLogger(RustParserClient.class);

    private final SubstrateClient client;

    public RustParserClient(SubstrateClient client) {
        this.client = client;
    }

    // ---- Groovy AST parsing ----

    /**
     * Parse a Groovy DSL script into an AST representation.
     *
     * @param scriptContent the Groovy script source text
     * @param filePath      optional file path for error reporting
     * @return parse result containing AST nodes and error info
     */
    public ParseResult parseGroovy(String scriptContent, String filePath) {
        if (client.isNoop()) {
            return ParseResult.empty();
        }
        try {
            ParseGroovyResponse response = client.getParserStub().parseGroovy(
                ParseGroovyRequest.newBuilder()
                    .setScriptContent(scriptContent)
                    .setFilePath(filePath != null ? filePath : "")
                    .build());
            return ParseResult.fromProto(response);
        } catch (Exception e) {
            LOGGER.debug("[substrate:parser] Groovy parse failed", e);
            return ParseResult.empty();
        }
    }

    // ---- Build script parsing ----

    /**
     * Parse a Gradle build script and extract all elements.
     *
     * @param scriptContent the build script source text
     * @param filePath      optional file path for error reporting
     * @return parse result containing build script elements and error info
     */
    public BuildScriptParseResult parseBuildScript(String scriptContent, String filePath) {
        if (client.isNoop()) {
            return BuildScriptParseResult.empty();
        }
        try {
            ParseBuildScriptResponse response = client.getParserStub().parseBuildScript(
                ParseBuildScriptRequest.newBuilder()
                    .setScriptContent(scriptContent)
                    .setFilePath(filePath != null ? filePath : "")
                    .build());
            return BuildScriptParseResult.fromProto(response);
        } catch (Exception e) {
            LOGGER.debug("[substrate:parser] Build script parse failed", e);
            return BuildScriptParseResult.empty();
        }
    }

    // ---- Dependency parsing ----

    /**
     * Extract dependency declarations from a build script.
     *
     * @param scriptContent      the build script source text
     * @param configurationName  optional configuration name to filter by
     * @return list of dependency info extracted from the script
     */
    public List<DependencyInfo> parseDependencies(String scriptContent, String configurationName) {
        if (client.isNoop()) {
            return Collections.emptyList();
        }
        try {
            ParseBuildScriptDependenciesResponse response = client.getParserStub()
                .parseBuildScriptDependencies(
                    ParseBuildScriptDependenciesRequest.newBuilder()
                        .setScriptContent(scriptContent)
                        .setConfigurationName(configurationName != null ? configurationName : "")
                        .build());
            return response.getDependenciesList().stream()
                .map(DependencyInfo::fromProto)
                .collect(Collectors.toList());
        } catch (Exception e) {
            LOGGER.debug("[substrate:parser] Dependency parse failed", e);
            return Collections.emptyList();
        }
    }

    // ---- Plugin parsing ----

    /**
     * Extract plugin declarations from a build script.
     *
     * @param scriptContent the build script source text
     * @return list of plugin info extracted from the script
     */
    public List<PluginInfo> parsePlugins(String scriptContent) {
        if (client.isNoop()) {
            return Collections.emptyList();
        }
        try {
            ParseBuildScriptPluginsResponse response = client.getParserStub()
                .parseBuildScriptPlugins(
                    ParseBuildScriptPluginsRequest.newBuilder()
                        .setScriptContent(scriptContent)
                        .build());
            return response.getPluginsList().stream()
                .map(PluginInfo::fromProto)
                .collect(Collectors.toList());
        } catch (Exception e) {
            LOGGER.debug("[substrate:parser] Plugin parse failed", e);
            return Collections.emptyList();
        }
    }

    // ---- Repository parsing ----

    /**
     * Extract repository declarations from a build script.
     *
     * @param scriptContent the build script source text
     * @return list of repository info extracted from the script
     */
    public List<RepositoryInfo> parseRepositories(String scriptContent) {
        if (client.isNoop()) {
            return Collections.emptyList();
        }
        try {
            ParseBuildScriptRepositoriesResponse response = client.getParserStub()
                .parseBuildScriptRepositories(
                    ParseBuildScriptRepositoriesRequest.newBuilder()
                        .setScriptContent(scriptContent)
                        .build());
            return response.getRepositoriesList().stream()
                .map(RepositoryInfo::fromProto)
                .collect(Collectors.toList());
        } catch (Exception e) {
            LOGGER.debug("[substrate:parser] Repository parse failed", e);
            return Collections.emptyList();
        }
    }

    // ---- Task parsing ----

    /**
     * Extract task definitions from a build script.
     *
     * @param scriptContent the build script source text
     * @return list of task info extracted from the script
     */
    public List<TaskInfo> parseTasks(String scriptContent) {
        if (client.isNoop()) {
            return Collections.emptyList();
        }
        try {
            ParseBuildScriptTasksResponse response = client.getParserStub()
                .parseBuildScriptTasks(
                    ParseBuildScriptTasksRequest.newBuilder()
                        .setScriptContent(scriptContent)
                        .build());
            return response.getTasksList().stream()
                .map(TaskInfo::fromProto)
                .collect(Collectors.toList());
        } catch (Exception e) {
            LOGGER.debug("[substrate:parser] Task parse failed", e);
            return Collections.emptyList();
        }
    }

    // ---- Source set parsing ----

    /**
     * Extract source set declarations from a build script.
     *
     * @param scriptContent the build script source text
     * @return list of source set info extracted from the script
     */
    public List<SourceSetInfo> parseSourceSets(String scriptContent) {
        if (client.isNoop()) {
            return Collections.emptyList();
        }
        try {
            ParseBuildScriptSourceSetsResponse response = client.getParserStub()
                .parseBuildScriptSourceSets(
                    ParseBuildScriptSourceSetsRequest.newBuilder()
                        .setScriptContent(scriptContent)
                        .build());
            return response.getSourceSetsList().stream()
                .map(SourceSetInfo::fromProto)
                .collect(Collectors.toList());
        } catch (Exception e) {
            LOGGER.debug("[substrate:parser] Source set parse failed", e);
            return Collections.emptyList();
        }
    }

    // ========== DTO inner classes ==========

    /**
     * Result of parsing a Groovy script into an AST.
     */
    public static class ParseResult {
        private final List<GroovyNodeInfo> nodes;
        private final int errorCount;
        private final String errorMessage;

        private ParseResult(List<GroovyNodeInfo> nodes, int errorCount, String errorMessage) {
            this.nodes = nodes;
            this.errorCount = errorCount;
            this.errorMessage = errorMessage;
        }

        public static ParseResult empty() {
            return new ParseResult(Collections.emptyList(), 0, "");
        }

        public static ParseResult fromProto(ParseGroovyResponse response) {
            List<GroovyNodeInfo> nodes = response.getNodesList().stream()
                .map(GroovyNodeInfo::fromProto)
                .collect(Collectors.toList());
            return new ParseResult(nodes, response.getErrorCount(), response.getErrorMessage());
        }

        public List<GroovyNodeInfo> getNodes() {
            return nodes;
        }

        public int getErrorCount() {
            return errorCount;
        }

        public String getErrorMessage() {
            return errorMessage;
        }

        public boolean hasErrors() {
            return errorCount > 0;
        }
    }

    /**
     * A node in the Groovy AST tree.
     */
    public static class GroovyNodeInfo {
        private final String nodeType;
        private final String text;
        private final int line;
        private final int column;
        private final List<GroovyNodeInfo> children;
        private final Map<String, String> properties;

        private GroovyNodeInfo(String nodeType, String text, int line, int column,
                               List<GroovyNodeInfo> children, Map<String, String> properties) {
            this.nodeType = nodeType;
            this.text = text;
            this.line = line;
            this.column = column;
            this.children = children;
            this.properties = properties;
        }

        public static GroovyNodeInfo fromProto(GroovyNode proto) {
            List<GroovyNodeInfo> children = proto.getChildrenList().stream()
                .map(GroovyNodeInfo::fromProto)
                .collect(Collectors.toList());
            return new GroovyNodeInfo(
                proto.getNodeType(),
                proto.getText(),
                proto.getLine(),
                proto.getColumn(),
                children,
                new HashMap<>(proto.getPropertiesMap())
            );
        }

        public String getNodeType() { return nodeType; }
        public String getText() { return text; }
        public int getLine() { return line; }
        public int getColumn() { return column; }
        public List<GroovyNodeInfo> getChildren() { return children; }
        public Map<String, String> getProperties() { return properties; }
    }

    /**
     * Result of parsing a build script into structural elements.
     */
    public static class BuildScriptParseResult {
        private final List<BuildScriptElementInfo> elements;
        private final int errorCount;

        private BuildScriptParseResult(List<BuildScriptElementInfo> elements, int errorCount) {
            this.elements = elements;
            this.errorCount = errorCount;
        }

        public static BuildScriptParseResult empty() {
            return new BuildScriptParseResult(Collections.emptyList(), 0);
        }

        public static BuildScriptParseResult fromProto(ParseBuildScriptResponse response) {
            List<BuildScriptElementInfo> elements = response.getElementsList().stream()
                .map(BuildScriptElementInfo::fromProto)
                .collect(Collectors.toList());
            return new BuildScriptParseResult(elements, response.getErrorCount());
        }

        public List<BuildScriptElementInfo> getElements() { return elements; }
        public int getElementCount() { return elements.size(); }
        public int getErrorCount() { return errorCount; }
        public boolean hasErrors() { return errorCount > 0; }
    }

    /**
     * A single element extracted from a build script.
     */
    public static class BuildScriptElementInfo {
        private final String elementType;
        private final Map<String, String> properties;
        private final String rawText;
        private final int line;

        private BuildScriptElementInfo(String elementType, Map<String, String> properties,
                                       String rawText, int line) {
            this.elementType = elementType;
            this.properties = properties;
            this.rawText = rawText;
            this.line = line;
        }

        public static BuildScriptElementInfo fromProto(BuildScriptElement proto) {
            return new BuildScriptElementInfo(
                proto.getElementType(),
                new HashMap<>(proto.getPropertiesMap()),
                proto.getRawText(),
                proto.getLine()
            );
        }

        public String getElementType() { return elementType; }
        public Map<String, String> getProperties() { return properties; }
        public String getRawText() { return rawText; }
        public int getLine() { return line; }
    }

    /**
     * A dependency declaration extracted from a build script.
     */
    public static class DependencyInfo {
        private final String group;
        private final String artifact;
        private final String version;
        private final String configuration;
        private final boolean isProject;
        private final String rawText;

        private DependencyInfo(String group, String artifact, String version,
                               String configuration, boolean isProject, String rawText) {
            this.group = group;
            this.artifact = artifact;
            this.version = version;
            this.configuration = configuration;
            this.isProject = isProject;
            this.rawText = rawText;
        }

        public static DependencyInfo fromProto(DependencyEntry proto) {
            return new DependencyInfo(
                proto.getGroup(),
                proto.getArtifact(),
                proto.getVersion(),
                proto.getConfiguration(),
                proto.getIsProject(),
                proto.getRawText()
            );
        }

        public String getGroup() { return group; }
        public String getArtifact() { return artifact; }
        public String getVersion() { return version; }
        public String getConfiguration() { return configuration; }
        public boolean isProject() { return isProject; }
        public String getRawText() { return rawText; }

        /**
         * Returns the dependency coordinate in group:artifact:version form,
         * or project path if this is a project dependency.
         */
        public String toCoordinate() {
            if (isProject) {
                return artifact;
            }
            StringBuilder sb = new StringBuilder();
            if (group != null && !group.isEmpty()) {
                sb.append(group).append(':');
            }
            sb.append(artifact);
            if (version != null && !version.isEmpty()) {
                sb.append(':').append(version);
            }
            return sb.toString();
        }
    }

    /**
     * A plugin declaration extracted from a build script.
     */
    public static class PluginInfo {
        private final String id;
        private final String version;
        private final boolean apply;
        private final String rawText;
        private final int line;

        private PluginInfo(String id, String version, boolean apply,
                           String rawText, int line) {
            this.id = id;
            this.version = version;
            this.apply = apply;
            this.rawText = rawText;
            this.line = line;
        }

        public static PluginInfo fromProto(PluginEntry proto) {
            return new PluginInfo(
                proto.getId(),
                proto.getVersion(),
                proto.getApply(),
                proto.getRawText(),
                proto.getLine()
            );
        }

        public String getId() { return id; }
        public String getVersion() { return version; }
        public boolean isApply() { return apply; }
        public String getRawText() { return rawText; }
        public int getLine() { return line; }
    }

    /**
     * A repository declaration extracted from a build script.
     */
    public static class RepositoryInfo {
        private final String name;
        private final String url;
        private final String type;
        private final String rawText;

        private RepositoryInfo(String name, String url, String type, String rawText) {
            this.name = name;
            this.url = url;
            this.type = type;
            this.rawText = rawText;
        }

        public static RepositoryInfo fromProto(RepositoryEntry proto) {
            return new RepositoryInfo(
                proto.getName(),
                proto.getUrl(),
                proto.getType(),
                proto.getRawText()
            );
        }

        public String getName() { return name; }
        public String getUrl() { return url; }
        public String getType() { return type; }
        public String getRawText() { return rawText; }
    }

    /**
     * A task definition extracted from a build script.
     */
    public static class TaskInfo {
        private final String name;
        private final String type;
        private final List<String> dependsOn;
        private final Map<String, String> properties;
        private final String rawText;
        private final int line;

        private TaskInfo(String name, String type, List<String> dependsOn,
                         Map<String, String> properties, String rawText, int line) {
            this.name = name;
            this.type = type;
            this.dependsOn = dependsOn;
            this.properties = properties;
            this.rawText = rawText;
            this.line = line;
        }

        public static TaskInfo fromProto(TaskEntry proto) {
            return new TaskInfo(
                proto.getName(),
                proto.getType(),
                new ArrayList<>(proto.getDependsOnList()),
                new HashMap<>(proto.getPropertiesMap()),
                proto.getRawText(),
                proto.getLine()
            );
        }

        public String getName() { return name; }
        public String getType() { return type; }
        public List<String> getDependsOn() { return dependsOn; }
        public Map<String, String> getProperties() { return properties; }
        public String getRawText() { return rawText; }
        public int getLine() { return line; }
    }

    /**
     * A source set declaration extracted from a build script.
     */
    public static class SourceSetInfo {
        private final String name;
        private final List<String> srcDirs;
        private final List<String> resourceDirs;

        private SourceSetInfo(String name, List<String> srcDirs, List<String> resourceDirs) {
            this.name = name;
            this.srcDirs = srcDirs;
            this.resourceDirs = resourceDirs;
        }

        public static SourceSetInfo fromProto(SourceSetEntry proto) {
            return new SourceSetInfo(
                proto.getName(),
                new ArrayList<>(proto.getSrcDirsList()),
                new ArrayList<>(proto.getResourceDirsList())
            );
        }

        public String getName() { return name; }
        public List<String> getSrcDirs() { return srcDirs; }
        public List<String> getResourceDirs() { return resourceDirs; }
    }
}
