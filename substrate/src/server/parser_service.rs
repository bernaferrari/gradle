use std::collections::HashMap;

use tonic::{Request, Response, Status};

use crate::proto::{
    parser_service_server::ParserService, BuildScriptElement, DependencyEntry, GroovyNode,
    ParseBuildScriptDependenciesRequest, ParseBuildScriptDependenciesResponse,
    ParseBuildScriptPluginsRequest, ParseBuildScriptPluginsResponse,
    ParseBuildScriptRepositoriesRequest, ParseBuildScriptRepositoriesResponse,
    ParseBuildScriptRequest, ParseBuildScriptResponse, ParseBuildScriptSourceSetsRequest,
    ParseBuildScriptSourceSetsResponse, ParseBuildScriptTasksRequest,
    ParseBuildScriptTasksResponse, ParseBuildScriptTypedRequest, ParseBuildScriptTypedResponse,
    ParseGroovyRequest, ParseGroovyResponse, PluginEntry, RepositoryEntry, TaskEntry,
    TypedBuildScriptDep, TypedDependency, TypedDependencyResolutionManagement,
    TypedPlugin, TypedPluginManagement, TypedRepository, TypedSubproject, TypedTaskConfig,
    TypedVersionCatalogRef,
};
use crate::server::build_script_parser;
use crate::server::groovy_parser::{self, ast::Stmt};

pub struct ParserServiceImpl;

impl ParserServiceImpl {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ParserServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the span (line, column) from a `Stmt` variant.
fn statement_span(stmt: &Stmt) -> (u32, u32) {
    match stmt {
        Stmt::Expr(e) => (e.span.line, e.span.column),
        Stmt::VarDecl(v) => (v.span.line, v.span.column),
        Stmt::Import(i) => (i.span.line, i.span.column),
        Stmt::Block(b) => (b.span.line, b.span.column),
    }
}

/// Return a human-readable type name for a `Stmt` variant.
fn statement_type(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::Expr(_) => "Expr",
        Stmt::VarDecl(_) => "VarDecl",
        Stmt::Import(_) => "Import",
        Stmt::Block(_) => "Block",
    }
}

/// Build a short summary string for display in the node text.
fn statement_summary(stmt: &Stmt) -> String {
    match stmt {
        Stmt::Expr(e) => format!("{:?}", e.expr),
        Stmt::VarDecl(v) => format!(
            "{} {}",
            match &v.kind {
                groovy_parser::ast::VarKind::Def => "def".to_string(),
                groovy_parser::ast::VarKind::Val => "val".to_string(),
                groovy_parser::ast::VarKind::Var => "var".to_string(),
                groovy_parser::ast::VarKind::Typed { type_name } => type_name.clone(),
            },
            v.name
        ),
        Stmt::Import(i) => {
            let suffix = if i.is_wildcard { ".*" } else { "" };
            format!("import {}{}", i.path, suffix)
        }
        Stmt::Block(b) => format!("block ({} statements)", b.statements.len()),
    }
}

/// Convert a parsed Groovy `Script` into proto `GroovyNode`s.
fn to_groovy_nodes(script: &groovy_parser::Script) -> Vec<GroovyNode> {
    let mut nodes = Vec::new();
    for stmt in &script.statements {
        let (line, col) = statement_span(stmt);
        nodes.push(GroovyNode {
            node_type: statement_type(stmt).to_string(),
            text: statement_summary(stmt),
            line: line as i32,
            column: col as i32,
            children: Vec::new(),
            properties: HashMap::new(),
        });
    }
    nodes
}

/// Parse a dependency notation string like "group:artifact:version" into parts.
/// Returns (group, artifact, version, is_project).
fn parse_dep_notation(notation: &str) -> (String, String, String, bool) {
    let trimmed = notation.trim();
    // Handle project dependency: "project(':other')"
    if trimmed.starts_with("project(") {
        return (String::new(), trimmed.to_string(), String::new(), true);
    }
    // Split on ':'
    let parts: Vec<&str> = trimmed.splitn(3, ':').collect();
    match parts.as_slice() {
        [group, artifact, version] => (
            group.to_string(),
            artifact.to_string(),
            version.to_string(),
            false,
        ),
        [group, artifact] => (
            group.to_string(),
            artifact.to_string(),
            String::new(),
            false,
        ),
        _ => (String::new(), trimmed.to_string(), String::new(), false),
    }
}

/// Build the full `BuildScriptParseResult` from the given content and file path.
fn parse_script(content: &str, file_path: &str) -> build_script_parser::BuildScriptParseResult {
    build_script_parser::parse_build_script(content, file_path)
}

// ---------------------------------------------------------------------------
// Service implementation
// ---------------------------------------------------------------------------

#[tonic::async_trait]
impl ParserService for ParserServiceImpl {
    // ----- ParseGroovy ------------------------------------------------------

    async fn parse_groovy(
        &self,
        req: Request<ParseGroovyRequest>,
    ) -> Result<Response<ParseGroovyResponse>, Status> {
        let content = req.get_ref().script_content.clone();
        let file_path = req.get_ref().file_path.clone();

        let _dialect = req.get_ref().dialect();
        let _is_kotlin = _dialect == crate::proto::Dialect::KotlinDsl
            || file_path.ends_with(".gradle.kts")
            || file_path.ends_with(".kts");

        let result = groovy_parser::parse(&content);
        let mut nodes = to_groovy_nodes(&result.script);

        // Annotate nodes with dialect information for downstream consumers
        if _is_kotlin {
            for node in &mut nodes {
                node.properties
                    .insert("dialect".to_string(), "kotlin_dsl".to_string());
            }
        }

        let error_count = result.errors.len() as i32;
        let error_message = result
            .errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_default();

        Ok(Response::new(ParseGroovyResponse {
            nodes,
            error_count,
            error_message,
        }))
    }

    // ----- ParseBuildScript -------------------------------------------------

    async fn parse_build_script(
        &self,
        req: Request<ParseBuildScriptRequest>,
    ) -> Result<Response<ParseBuildScriptResponse>, Status> {
        let content = req.get_ref().script_content.clone();
        let file_path = req.get_ref().file_path.clone();

        let result = parse_script(&content, &file_path);
        let mut elements = Vec::new();

        // Plugins
        for plugin in &result.plugins {
            let mut props = HashMap::new();
            props.insert("id".to_string(), plugin.id.clone());
            props.insert("apply".to_string(), plugin.apply.to_string());
            elements.push(BuildScriptElement {
                element_type: "plugin".to_string(),
                properties: props,
                raw_text: format!("plugin id: {} apply: {}", plugin.id, plugin.apply),
                line: plugin.line.unwrap_or(0) as i32,
            });
        }

        // Dependencies
        for dep in &result.dependencies {
            let mut props = HashMap::new();
            props.insert("configuration".to_string(), dep.configuration.clone());
            props.insert("notation".to_string(), dep.notation.clone());
            elements.push(BuildScriptElement {
                element_type: "dependency".to_string(),
                properties: props,
                raw_text: dep.notation.clone(),
                line: 0,
            });
        }

        // Task configs
        for task in &result.task_configs {
            let mut props = HashMap::new();
            props.insert("name".to_string(), task.task_name.clone());
            props.insert("enabled".to_string(), task.enabled.to_string());
            if !task.depends_on.is_empty() {
                props.insert("dependsOn".to_string(), task.depends_on.join(", "));
            }
            if !task.should_run_after.is_empty() {
                props.insert(
                    "shouldRunAfter".to_string(),
                    task.should_run_after.join(", "),
                );
            }
            elements.push(BuildScriptElement {
                element_type: "task".to_string(),
                properties: props,
                raw_text: format!("task {}", task.task_name),
                line: 0,
            });
        }

        // Repositories
        for repo in &result.repositories {
            let mut props = HashMap::new();
            props.insert("type".to_string(), repo.repo_type.clone());
            props.insert("name".to_string(), repo.name.clone());
            elements.push(BuildScriptElement {
                element_type: "repository".to_string(),
                properties: props,
                raw_text: format!("{}({})", repo.repo_type, repo.name),
                line: 0,
            });
        }

        // Subprojects
        for sub in &result.subprojects {
            let mut props = HashMap::new();
            props.insert("path".to_string(), sub.path.clone());
            elements.push(BuildScriptElement {
                element_type: "subproject".to_string(),
                properties: props,
                raw_text: format!("include {}", sub.path),
                line: 0,
            });
        }

        // Properties (group, version, source/target compatibility)
        if let Some(ref group) = result.group {
            let mut props = HashMap::new();
            props.insert("value".to_string(), group.clone());
            elements.push(BuildScriptElement {
                element_type: "property".to_string(),
                properties: props,
                raw_text: format!("group = {}", group),
                line: 0,
            });
        }
        if let Some(ref ver) = result.version {
            let mut props = HashMap::new();
            props.insert("value".to_string(), ver.clone());
            elements.push(BuildScriptElement {
                element_type: "property".to_string(),
                properties: props,
                raw_text: format!("version = {}", ver),
                line: 0,
            });
        }
        if let Some(ref sc) = result.source_compatibility {
            let mut props = HashMap::new();
            props.insert("value".to_string(), sc.clone());
            elements.push(BuildScriptElement {
                element_type: "property".to_string(),
                properties: props,
                raw_text: format!("sourceCompatibility = {}", sc),
                line: 0,
            });
        }
        if let Some(ref tc) = result.target_compatibility {
            let mut props = HashMap::new();
            props.insert("value".to_string(), tc.clone());
            elements.push(BuildScriptElement {
                element_type: "property".to_string(),
                properties: props,
                raw_text: format!("targetCompatibility = {}", tc),
                line: 0,
            });
        }

        Ok(Response::new(ParseBuildScriptResponse {
            elements,
            error_count: result.warnings.len() as i32,
        }))
    }

    // ----- ParseBuildScriptDependencies --------------------------------------

    async fn parse_build_script_dependencies(
        &self,
        req: Request<ParseBuildScriptDependenciesRequest>,
    ) -> Result<Response<ParseBuildScriptDependenciesResponse>, Status> {
        let content = req.get_ref().script_content.clone();
        let config_filter = req.get_ref().configuration_name.clone();

        let result = parse_script(&content, "build.gradle");
        let mut entries = Vec::new();

        for dep in &result.dependencies {
            // Filter by configuration if requested
            if !config_filter.is_empty() && dep.configuration != config_filter {
                continue;
            }

            let (group, artifact, version, is_project) = parse_dep_notation(&dep.notation);

            entries.push(DependencyEntry {
                group,
                artifact,
                version,
                configuration: dep.configuration.clone(),
                raw_text: dep.notation.clone(),
                is_project,
            });
        }

        Ok(Response::new(ParseBuildScriptDependenciesResponse {
            dependencies: entries,
        }))
    }

    // ----- ParseBuildScriptPlugins ------------------------------------------

    async fn parse_build_script_plugins(
        &self,
        req: Request<ParseBuildScriptPluginsRequest>,
    ) -> Result<Response<ParseBuildScriptPluginsResponse>, Status> {
        let content = req.get_ref().script_content.clone();

        let result = parse_script(&content, "build.gradle");
        let plugins: Vec<PluginEntry> = result
            .plugins
            .into_iter()
            .map(|p| PluginEntry {
                id: p.id,
                version: p.version.unwrap_or_default(),
                apply: p.apply,
                raw_text: String::new(),
                line: p.line.unwrap_or(0) as i32,
            })
            .collect();

        Ok(Response::new(ParseBuildScriptPluginsResponse { plugins }))
    }

    // ----- ParseBuildScriptRepositories --------------------------------------

    async fn parse_build_script_repositories(
        &self,
        req: Request<ParseBuildScriptRepositoriesRequest>,
    ) -> Result<Response<ParseBuildScriptRepositoriesResponse>, Status> {
        let content = req.get_ref().script_content.clone();

        let result = parse_script(&content, "build.gradle");
        let repos: Vec<RepositoryEntry> = result
            .repositories
            .into_iter()
            .map(|r| {
                // Determine if the name looks like a URL
                let url = if r.name.starts_with("http://") || r.name.starts_with("https://") {
                    r.name.clone()
                } else {
                    String::new()
                };
                RepositoryEntry {
                    name: r.name.clone(),
                    url,
                    r#type: r.repo_type.clone(),
                    raw_text: format!("{}({})", r.repo_type, r.name),
                }
            })
            .collect();

        Ok(Response::new(ParseBuildScriptRepositoriesResponse {
            repositories: repos,
        }))
    }

    // ----- ParseBuildScriptTasks --------------------------------------------

    async fn parse_build_script_tasks(
        &self,
        req: Request<ParseBuildScriptTasksRequest>,
    ) -> Result<Response<ParseBuildScriptTasksResponse>, Status> {
        let content = req.get_ref().script_content.clone();

        let result = parse_script(&content, "build.gradle");
        let tasks: Vec<TaskEntry> = result
            .task_configs
            .into_iter()
            .map(|t| {
                let mut props = HashMap::new();
                props.insert("enabled".to_string(), t.enabled.to_string());
                TaskEntry {
                    name: t.task_name,
                    r#type: String::new(), // build_script_parser does not classify task types yet
                    depends_on: t.depends_on,
                    properties: props,
                    raw_text: String::new(),
                    line: t.line.unwrap_or(0) as i32,
                }
            })
            .collect();

        Ok(Response::new(ParseBuildScriptTasksResponse { tasks }))
    }

    // ----- ParseBuildScriptSourceSets ---------------------------------------

    async fn parse_build_script_source_sets(
        &self,
        _req: Request<ParseBuildScriptSourceSetsRequest>,
    ) -> Result<Response<ParseBuildScriptSourceSetsResponse>, Status> {
        // build_script_parser does not parse source sets yet
        Ok(Response::new(ParseBuildScriptSourceSetsResponse {
            source_sets: Vec::new(),
        }))
    }

    // ----- ParseBuildScriptTyped --------------------------------------------

    async fn parse_build_script_typed(
        &self,
        req: Request<ParseBuildScriptTypedRequest>,
    ) -> Result<Response<ParseBuildScriptTypedResponse>, Status> {
        let content = req.get_ref().script_content.clone();
        let file_path = req.get_ref().file_path.clone();

        let result = parse_script(&content, &file_path);

        let script_type = match result.script_type {
            build_script_parser::ScriptType::KotlinDsl => crate::proto::ScriptType::KotlinDslScript,
            build_script_parser::ScriptType::Groovy => crate::proto::ScriptType::GroovyScript,
            build_script_parser::ScriptType::Unknown => crate::proto::ScriptType::UnknownScript,
        };

        let plugins: Vec<TypedPlugin> = result
            .plugins
            .into_iter()
            .map(|p| TypedPlugin {
                id: p.id,
                apply: p.apply,
                version: p.version.unwrap_or_default(),
                line: p.line.unwrap_or(0) as i32,
            })
            .collect();

        let dependencies: Vec<TypedDependency> = result
            .dependencies
            .into_iter()
            .map(|d| TypedDependency {
                configuration: d.configuration,
                notation: d.notation,
                line: d.line.unwrap_or(0) as i32,
            })
            .collect();

        let catalog_refs: Vec<TypedVersionCatalogRef> = result
            .catalog_refs
            .into_iter()
            .map(|c| TypedVersionCatalogRef {
                configuration: c.configuration,
                alias: c.alias,
            })
            .collect();

        let buildscript_deps: Vec<TypedBuildScriptDep> = result
            .buildscript_deps
            .into_iter()
            .map(|d| TypedBuildScriptDep { notation: d.notation })
            .collect();

        let task_configs: Vec<TypedTaskConfig> = result
            .task_configs
            .into_iter()
            .map(|t| TypedTaskConfig {
                task_name: t.task_name,
                depends_on: t.depends_on,
                should_run_after: t.should_run_after,
                enabled: t.enabled,
                line: t.line.unwrap_or(0) as i32,
            })
            .collect();

        let repositories: Vec<TypedRepository> = result
            .repositories
            .into_iter()
            .map(|r| TypedRepository {
                name: r.name,
                repo_type: r.repo_type,
            })
            .collect();

        let subprojects: Vec<TypedSubproject> = result
            .subprojects
            .into_iter()
            .map(|s| TypedSubproject { path: s.path })
            .collect();

        let plugin_management = result.plugin_management.map(|pm| TypedPluginManagement {
            repositories: pm
                .repositories
                .into_iter()
                .map(|r| TypedRepository {
                    name: r.name,
                    repo_type: r.repo_type,
                })
                .collect(),
        });

        let dependency_resolution_management =
            result
                .dependency_resolution_management
                .map(|drm| TypedDependencyResolutionManagement {
                    repositories_mode: drm.repositories_mode.unwrap_or_default(),
                    repositories: drm
                        .repositories
                        .into_iter()
                        .map(|r| TypedRepository {
                            name: r.name,
                            repo_type: r.repo_type,
                        })
                        .collect(),
                });

        Ok(Response::new(ParseBuildScriptTypedResponse {
            script_type: script_type as i32,
            plugins,
            dependencies,
            catalog_refs,
            buildscript_deps,
            task_configs,
            repositories,
            subprojects,
            plugin_management,
            dependency_resolution_management,
            source_compatibility: result.source_compatibility.unwrap_or_default(),
            target_compatibility: result.target_compatibility.unwrap_or_default(),
            group: result.group.unwrap_or_default(),
            version: result.version.unwrap_or_default(),
            warnings: result.warnings,
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dep_notation_full() {
        let (g, a, v, proj) = parse_dep_notation("com.example:lib:1.0");
        assert_eq!(g, "com.example");
        assert_eq!(a, "lib");
        assert_eq!(v, "1.0");
        assert!(!proj);
    }

    #[test]
    fn test_parse_dep_notation_no_version() {
        let (g, a, v, proj) = parse_dep_notation("com.example:lib");
        assert_eq!(g, "com.example");
        assert_eq!(a, "lib");
        assert_eq!(v, "");
        assert!(!proj);
    }

    #[test]
    fn test_parse_dep_notation_project() {
        let (g, a, v, proj) = parse_dep_notation("project(':other')");
        assert_eq!(g, "");
        assert_eq!(a, "project(':other')");
        assert_eq!(v, "");
        assert!(proj);
    }

    #[test]
    fn test_parse_dep_notation_single() {
        let (g, a, v, proj) = parse_dep_notation("somelib");
        assert_eq!(g, "");
        assert_eq!(a, "somelib");
        assert_eq!(v, "");
        assert!(!proj);
    }

    #[test]
    fn test_to_groovy_nodes_simple() {
        let source = "println 'hello'";
        let result = groovy_parser::parse(source);
        assert!(result.errors.is_empty(), "expected no parse errors");
        let nodes = to_groovy_nodes(&result.script);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_type, "Expr");
    }

    #[test]
    fn test_to_groovy_nodes_import() {
        let source = "import com.example.Foo";
        let result = groovy_parser::parse(source);
        assert!(result.errors.is_empty(), "expected no parse errors");
        let nodes = to_groovy_nodes(&result.script);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_type, "Import");
        assert!(nodes[0].text.contains("com.example.Foo"));
    }

    #[tokio::test]
    async fn test_parse_groovy_service() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseGroovyRequest {
            script_content: "println 'hello'".to_string(),
            file_path: String::new(),
            ..Default::default()
        });
        let resp = svc.parse_groovy(req).await.unwrap();
        assert_eq!(resp.get_ref().error_count, 0);
        assert_eq!(resp.get_ref().nodes.len(), 1);
    }

    #[tokio::test]
    async fn test_parse_build_script_service() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptRequest {
            script_content: r#"
                plugins {
                    id 'java'
                }
                repositories {
                    mavenCentral()
                }
                dependencies {
                    implementation 'com.example:lib:1.0'
                }
            "#
            .to_string(),
            file_path: "build.gradle".to_string(),
        });
        let resp = svc.parse_build_script(req).await.unwrap();
        let elements = &resp.get_ref().elements;
        assert!(!elements.is_empty());
        // Should contain at least one plugin, one dependency, one repository
        let types: Vec<&str> = elements.iter().map(|e| e.element_type.as_str()).collect();
        assert!(types.contains(&"plugin"));
        assert!(types.contains(&"dependency"));
        assert!(types.contains(&"repository"));
    }

    #[tokio::test]
    async fn test_parse_build_script_dependencies_filter() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptDependenciesRequest {
            script_content: "dependencies {\n    implementation 'com.example:lib:1.0'\n    testImplementation 'junit:junit:4.13'\n}\n".to_string(),
            configuration_name: "implementation".to_string(),
        });
        let resp = svc.parse_build_script_dependencies(req).await.unwrap();
        let deps = &resp.get_ref().dependencies;
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].configuration, "implementation");
        assert_eq!(deps[0].group, "com.example");
    }

    #[tokio::test]
    async fn test_parse_build_script_source_sets_empty() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptSourceSetsRequest {
            script_content: "plugins { id 'java' }".to_string(),
        });
        let resp = svc.parse_build_script_source_sets(req).await.unwrap();
        assert!(resp.get_ref().source_sets.is_empty());
    }

    #[tokio::test]
    async fn test_detect_kotlin_dialect_from_extension() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseGroovyRequest {
            script_content: "val x = 42".to_string(),
            file_path: "build.gradle.kts".to_string(),
            ..Default::default()
        });
        let resp = svc.parse_groovy(req).await.unwrap();
        assert_eq!(resp.get_ref().error_count, 0);
        // All nodes should be annotated with kotlin_dsl dialect
        for node in &resp.get_ref().nodes {
            assert_eq!(
                node.properties.get("dialect").unwrap_or(&String::new()),
                "kotlin_dsl"
            );
        }
    }

    #[tokio::test]
    async fn test_detect_kotlin_dialect_from_proto_field() {
        use crate::proto::Dialect;
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseGroovyRequest {
            script_content: "val x = 42".to_string(),
            file_path: "build.gradle".to_string(),
            dialect: Dialect::KotlinDsl as i32,
        });
        let resp = svc.parse_groovy(req).await.unwrap();
        assert_eq!(resp.get_ref().error_count, 0);
        for node in &resp.get_ref().nodes {
            assert_eq!(
                node.properties.get("dialect").unwrap_or(&String::new()),
                "kotlin_dsl"
            );
        }
    }

    #[tokio::test]
    async fn test_groovy_file_no_kotlin_dialect() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseGroovyRequest {
            script_content: "def x = 42".to_string(),
            file_path: "build.gradle".to_string(),
            ..Default::default()
        });
        let resp = svc.parse_groovy(req).await.unwrap();
        assert_eq!(resp.get_ref().error_count, 0);
        // Groovy files should NOT have kotlin_dsl annotation
        for node in &resp.get_ref().nodes {
            assert!(node.properties.get("dialect").is_none());
        }
    }

    // ----- Typed IR tests ---------------------------------------------------

    #[tokio::test]
    async fn test_parse_typed_groovy_with_plugins_and_deps() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptTypedRequest {
            script_content: r#"
                plugins {
                    id 'java'
                }
                dependencies {
                    implementation 'com.example:lib:1.0'
                    testImplementation 'junit:junit:4.13'
                }
            "#
            .to_string(),
            file_path: "build.gradle".to_string(),
        });
        let resp = svc.parse_build_script_typed(req).await.unwrap();
        let r = resp.get_ref();
        assert_eq!(r.script_type, crate::proto::ScriptType::GroovyScript as i32);
        assert!(!r.plugins.is_empty());
        assert_eq!(r.dependencies.len(), 2);
        assert_eq!(r.dependencies[0].configuration, "implementation");
        assert_eq!(r.dependencies[0].notation, "com.example:lib:1.0");
        assert_eq!(r.dependencies[1].configuration, "testImplementation");
    }

    #[tokio::test]
    async fn test_parse_typed_kotlin_dsl() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptTypedRequest {
            script_content: r#"
                plugins {
                    id("java")
                }
            "#
            .to_string(),
            file_path: "build.gradle.kts".to_string(),
        });
        let resp = svc.parse_build_script_typed(req).await.unwrap();
        assert_eq!(
            resp.get_ref().script_type,
            crate::proto::ScriptType::KotlinDslScript as i32
        );
        assert!(!resp.get_ref().plugins.is_empty());
        assert_eq!(resp.get_ref().plugins[0].id, "java");
    }

    #[tokio::test]
    async fn test_parse_typed_settings_with_subprojects() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptTypedRequest {
            script_content: "include ':app', ':lib'\ninclude ':shared'".to_string(),
            file_path: "settings.gradle".to_string(),
        });
        let resp = svc.parse_build_script_typed(req).await.unwrap();
        assert_eq!(resp.get_ref().subprojects.len(), 3);
        assert_eq!(resp.get_ref().subprojects[0].path, ":app");
    }

    #[tokio::test]
    async fn test_parse_typed_empty_script() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptTypedRequest {
            script_content: "".to_string(),
            file_path: "build.gradle".to_string(),
        });
        let resp = svc.parse_build_script_typed(req).await.unwrap();
        let r = resp.get_ref();
        assert!(r.plugins.is_empty());
        assert!(r.dependencies.is_empty());
        assert!(r.repositories.is_empty());
        assert!(r.task_configs.is_empty());
        assert!(r.subprojects.is_empty());
    }

    #[tokio::test]
    async fn test_parse_typed_task_config_with_depends_on() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptTypedRequest {
            script_content: "tasks.named('test') { dependsOn 'compileJava' }".to_string(),
            file_path: "build.gradle".to_string(),
        });
        let resp = svc.parse_build_script_typed(req).await.unwrap();
        // The parser may or may not extract this — check it doesn't crash
        // and returns valid structure
        assert_eq!(
            resp.get_ref().script_type,
            crate::proto::ScriptType::GroovyScript as i32
        );
    }

    #[tokio::test]
    async fn test_parse_typed_with_repositories() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptTypedRequest {
            script_content: r#"
                repositories {
                    mavenCentral()
                    maven { url 'https://repo.example.com' }
                }
            "#
            .to_string(),
            file_path: "build.gradle".to_string(),
        });
        let resp = svc.parse_build_script_typed(req).await.unwrap();
        assert!(!resp.get_ref().repositories.is_empty());
        // The string-based parser may extract "maven" or "mavenCentral"
        // depending on parsing; just verify repositories are populated
        assert!(!resp.get_ref().repositories[0].name.is_empty());
    }

    #[tokio::test]
    async fn test_parse_typed_catalog_refs() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptTypedRequest {
            script_content: r#"
                dependencies {
                    implementation(libs.commons.lang3)
                    testImplementation(libs.junit)
                }
            "#
            .to_string(),
            file_path: "build.gradle".to_string(),
        });
        let resp = svc.parse_build_script_typed(req).await.unwrap();
        // The string-based parser should extract catalog refs
        assert!(!resp.get_ref().catalog_refs.is_empty());
    }

    #[tokio::test]
    async fn test_parse_typed_unknown_file_type() {
        let svc = ParserServiceImpl::default();
        let req = Request::new(ParseBuildScriptTypedRequest {
            script_content: "x = 42".to_string(),
            file_path: "Makefile".to_string(),
        });
        let resp = svc.parse_build_script_typed(req).await.unwrap();
        assert_eq!(
            resp.get_ref().script_type,
            crate::proto::ScriptType::UnknownScript as i32
        );
    }
}
