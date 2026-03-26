//! Shared IR types for build script parsing results.
//!
//! These types are produced by both the AST-based extractor (`ast_extractor`)
//! and the string-based extractor (`build_script_parser`).

use serde::{Deserialize, Serialize};

/// A parsed dependency declaration from a build script.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ParsedDependency {
    /// The dependency configuration (e.g. "implementation", "api", "testImplementation").
    pub configuration: String,
    /// The dependency notation (e.g. "com.example:lib:1.0", "project(:other)").
    pub notation: String,
    /// Line number in the source file (from AST, None from string-based extraction).
    pub line: Option<u32>,
}

/// A parsed plugin application from a build script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedPlugin {
    /// The plugin ID or fully-qualified class name.
    pub id: String,
    /// Whether `apply false` was used (deferred application).
    pub apply: bool,
    /// Optional version string.
    pub version: Option<String>,
    /// Line number in the source file (from AST, None from string-based extraction).
    pub line: Option<u32>,
}

impl Default for ParsedPlugin {
    fn default() -> Self {
        Self {
            id: String::new(),
            apply: true,
            version: None,
            line: None,
        }
    }
}

/// A parsed task dependency declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedTaskDependency {
    /// The task path.
    pub path: String,
    /// Whether the dependency is "shouldRunAfter" (soft) vs dependsOn (hard).
    pub soft: bool,
}

/// A parsed task configuration block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedTaskConfig {
    /// The task name.
    pub task_name: String,
    /// Dependencies declared on this task.
    pub depends_on: Vec<String>,
    /// shouldRunAfter dependencies.
    pub should_run_after: Vec<String>,
    /// Whether the task is enabled (default: true).
    pub enabled: bool,
    /// Line number in the source file (from AST, None from string-based extraction).
    pub line: Option<u32>,
}

impl Default for ParsedTaskConfig {
    fn default() -> Self {
        Self {
            task_name: String::new(),
            depends_on: Vec::new(),
            should_run_after: Vec::new(),
            enabled: true,
            line: None,
        }
    }
}

/// A parsed repositories declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedRepository {
    /// Repository name or URL.
    pub name: String,
    /// Repository type (maven, mavenCentral, gradlePluginPortal, etc.).
    pub repo_type: String,
}

/// A parsed subprojects declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedSubproject {
    /// The subproject path (e.g. ":app", ":lib").
    pub path: String,
}

/// A parsed version catalog alias reference (e.g. `libs.commons.lang3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedVersionCatalogRef {
    /// The configuration (e.g. "implementation", "api").
    pub configuration: String,
    /// The catalog alias (e.g. "libs.commons.lang3", "libs.versions.java").
    pub alias: String,
}

/// A parsed buildscript classpath dependency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedBuildScriptDep {
    /// The dependency notation.
    pub notation: String,
}

/// A parsed plugin management repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedPluginRepository {
    /// Repository name or URL.
    pub name: String,
    /// Repository type (gradlePluginPortal, maven, mavenLocal, etc.).
    pub repo_type: String,
}

/// Parsed pluginManagement block.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ParsedPluginManagement {
    /// Plugin repositories.
    pub repositories: Vec<ParsedPluginRepository>,
}

/// Parsed dependencyResolutionManagement block.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ParsedDependencyResolutionManagement {
    /// Repositories mode (e.g. "PREFER_SETTINGS", "FAIL_ON_PROJECT_REPOS").
    pub repositories_mode: Option<String>,
    /// Repositories declared in settings.
    pub repositories: Vec<ParsedRepository>,
}

/// The result of parsing a build script.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildScriptParseResult {
    /// Applied plugins.
    pub plugins: Vec<ParsedPlugin>,
    /// Dependencies (project and external).
    pub dependencies: Vec<ParsedDependency>,
    /// Version catalog alias references.
    pub catalog_refs: Vec<ParsedVersionCatalogRef>,
    /// Buildscript classpath dependencies.
    pub buildscript_deps: Vec<ParsedBuildScriptDep>,
    /// Task configurations.
    pub task_configs: Vec<ParsedTaskConfig>,
    /// Repositories.
    pub repositories: Vec<ParsedRepository>,
    /// Subprojects (from settings.gradle or include statements).
    pub subprojects: Vec<ParsedSubproject>,
    /// Plugin management block (settings.gradle(.kts)).
    pub plugin_management: Option<ParsedPluginManagement>,
    /// Dependency resolution management block (settings.gradle(.kts)).
    pub dependency_resolution_management: Option<ParsedDependencyResolutionManagement>,
    /// Source compatibility (java, kotlin).
    pub source_compatibility: Option<String>,
    /// Target compatibility.
    pub target_compatibility: Option<String>,
    /// Group ID.
    pub group: Option<String>,
    /// Version.
    pub version: Option<String>,
    /// The build script type detected.
    pub script_type: ScriptType,
    /// Parse errors or warnings.
    pub warnings: Vec<String>,
}

/// Detected build script type.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum ScriptType {
    #[default]
    Unknown,
    KotlinDsl,
    Groovy,
}
