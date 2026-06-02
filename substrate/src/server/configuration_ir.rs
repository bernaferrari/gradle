use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::proto::{GetBuildEnvironmentResponse, GetBuildModelResponse};

use super::build_plan_ir::{
    CanonicalBuildPlan, CanonicalBuildPlanDependency, CanonicalBuildPlanRepository,
    CanonicalBuildPlanToolchainRequest,
};
use super::build_script_types::{
    BuildScriptParseResult, ParsedPlugin, ParsedRepository, ParsedVersionCatalogRef,
};

pub const CONFIGURATION_GRAPH_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalConfigurationGraph {
    pub schema_version: u32,
    pub build_id: String,
    #[serde(default)]
    pub settings: Option<CanonicalSettingsModel>,
    pub projects: Vec<CanonicalProjectConfiguration>,
    pub source_sets: Vec<CanonicalSourceSet>,
    pub tasks: Vec<CanonicalTaskConfiguration>,
    pub plugins: Vec<CanonicalPluginModel>,
    pub dependency_configurations: Vec<CanonicalDependencyConfiguration>,
    pub toolchains: Vec<CanonicalBuildPlanToolchainRequest>,
    pub invalidation_inputs: Vec<CanonicalConfigurationInput>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalSettingsModel {
    pub settings_file: String,
    pub root_project_name: String,
    pub included_projects: Vec<String>,
    pub plugin_repositories: Vec<CanonicalParsedRepository>,
    pub dependency_repositories: Vec<CanonicalParsedRepository>,
    #[serde(default)]
    pub repositories_mode: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalProjectConfiguration {
    pub path: String,
    pub name: String,
    pub project_dir: String,
    pub build_file: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub version: String,
    pub plugins: Vec<CanonicalPluginApplication>,
    pub repositories: Vec<CanonicalParsedRepository>,
    pub version_catalog_refs: Vec<CanonicalVersionCatalogRef>,
    #[serde(default)]
    pub source_compatibility: String,
    #[serde(default)]
    pub target_compatibility: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalPluginApplication {
    pub id: String,
    #[serde(default)]
    pub version: String,
    pub apply: bool,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalPluginModel {
    pub project_path: String,
    pub id: String,
    #[serde(default)]
    pub version: String,
    pub apply: bool,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalParsedRepository {
    pub name: String,
    pub repo_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalVersionCatalogRef {
    pub configuration: String,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalSourceSet {
    pub project_path: String,
    pub name: String,
    pub java_source_dirs: Vec<String>,
    pub kotlin_source_dirs: Vec<String>,
    pub resource_dirs: Vec<String>,
    pub compile_classpath_configuration: String,
    pub runtime_classpath_configuration: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalTaskConfiguration {
    pub path: String,
    pub project_path: String,
    pub implementation_id: String,
    pub action_kind: String,
    pub cacheability: String,
    pub depends_on: Vec<String>,
    pub input_spec_count: usize,
    pub output_spec_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalDependencyConfiguration {
    pub project_path: String,
    pub name: String,
    pub dependencies: Vec<String>,
    pub constraints: Vec<String>,
    pub repositories: Vec<CanonicalBuildPlanRepository>,
    pub unsupported_features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalConfigurationInput {
    pub path: String,
    pub kind: String,
    pub exists: bool,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationScriptKind {
    Settings,
    Project,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigurationScript<'a> {
    pub kind: ConfigurationScriptKind,
    pub project_path: &'a str,
    pub path: &'a str,
    pub parsed: &'a BuildScriptParseResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationReplayAdmission {
    Accepted {
        project_count: usize,
        plugin_count: usize,
    },
    Rejected(ConfigurationReplayRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationReplayRejection {
    pub build_id: String,
    pub reasons: Vec<String>,
}

impl ConfigurationReplayRejection {
    pub fn message(&self) -> String {
        format!(
            "Rust configuration replay rejected build '{}' before execution: {}",
            self.build_id,
            self.reasons.join("; ")
        )
    }
}

impl CanonicalConfigurationGraph {
    pub fn normalized(mut self) -> Self {
        self.normalize_mut();
        self
    }

    pub fn normalize_mut(&mut self) {
        if let Some(settings) = &mut self.settings {
            settings.included_projects.sort_unstable();
            settings.included_projects.dedup();
            settings
                .plugin_repositories
                .sort_unstable_by(|a, b| (&a.repo_type, &a.name).cmp(&(&b.repo_type, &b.name)));
            settings.plugin_repositories.dedup();
            settings
                .dependency_repositories
                .sort_unstable_by(|a, b| (&a.repo_type, &a.name).cmp(&(&b.repo_type, &b.name)));
            settings.dependency_repositories.dedup();
            settings.warnings.sort_unstable();
            settings.warnings.dedup();
        }

        for project in &mut self.projects {
            project.plugins.sort_unstable_by(|a, b| {
                (&a.id, &a.version, a.apply).cmp(&(&b.id, &b.version, b.apply))
            });
            project.plugins.dedup();
            project
                .repositories
                .sort_unstable_by(|a, b| (&a.repo_type, &a.name).cmp(&(&b.repo_type, &b.name)));
            project.repositories.dedup();
            project.version_catalog_refs.sort_unstable_by(|a, b| {
                (&a.configuration, &a.alias).cmp(&(&b.configuration, &b.alias))
            });
            project.version_catalog_refs.dedup();
            project.warnings.sort_unstable();
            project.warnings.dedup();
        }
        self.projects.sort_unstable_by(|a, b| {
            (&a.path, &a.name, &a.project_dir).cmp(&(&b.path, &b.name, &b.project_dir))
        });

        for source_set in &mut self.source_sets {
            source_set.java_source_dirs.sort_unstable();
            source_set.java_source_dirs.dedup();
            source_set.kotlin_source_dirs.sort_unstable();
            source_set.kotlin_source_dirs.dedup();
            source_set.resource_dirs.sort_unstable();
            source_set.resource_dirs.dedup();
        }
        self.source_sets
            .sort_unstable_by(|a, b| (&a.project_path, &a.name).cmp(&(&b.project_path, &b.name)));
        self.source_sets.dedup();

        self.tasks.sort_unstable_by(|a, b| {
            (&a.path, &a.project_path, &a.implementation_id).cmp(&(
                &b.path,
                &b.project_path,
                &b.implementation_id,
            ))
        });

        self.plugins.sort_unstable_by(|a, b| {
            (&a.project_path, &a.id, &a.version, a.apply).cmp(&(
                &b.project_path,
                &b.id,
                &b.version,
                b.apply,
            ))
        });
        self.plugins.dedup();

        for configuration in &mut self.dependency_configurations {
            configuration.dependencies.sort_unstable();
            configuration.dependencies.dedup();
            configuration.constraints.sort_unstable();
            configuration.constraints.dedup();
            configuration.repositories.sort_unstable_by(|a, b| {
                (&a.id, &a.url, &a.layout).cmp(&(&b.id, &b.url, &b.layout))
            });
            configuration.repositories.dedup();
            configuration.unsupported_features.sort_unstable();
            configuration.unsupported_features.dedup();
        }
        self.dependency_configurations
            .sort_unstable_by(|a, b| (&a.project_path, &a.name).cmp(&(&b.project_path, &b.name)));

        self.toolchains.sort_unstable_by(|a, b| {
            (&a.language, &a.version, &a.vendor, &a.implementation).cmp(&(
                &b.language,
                &b.version,
                &b.vendor,
                &b.implementation,
            ))
        });
        self.toolchains.dedup();

        self.invalidation_inputs.sort_unstable_by(|a, b| {
            (&a.kind, &a.path, a.exists, &a.sha256).cmp(&(&b.kind, &b.path, b.exists, &b.sha256))
        });
        self.invalidation_inputs.dedup();
    }
}

pub fn validate_schema_version(graph: &CanonicalConfigurationGraph) -> Result<(), String> {
    if graph.schema_version != CONFIGURATION_GRAPH_SCHEMA_VERSION {
        return Err(format!(
            "unsupported configuration graph schema version: {} (expected {})",
            graph.schema_version, CONFIGURATION_GRAPH_SCHEMA_VERSION
        ));
    }
    Ok(())
}

pub fn fingerprint_normalized(
    graph: &CanonicalConfigurationGraph,
) -> Result<String, serde_json::Error> {
    let canonical = serde_json::to_string(graph)?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Ok(super::cache::hex::encode(hasher.finalize().as_ref()))
}

pub fn fingerprint_sha256_hex(
    graph: &CanonicalConfigurationGraph,
) -> Result<String, serde_json::Error> {
    let mut normalized = graph.clone();
    normalized.normalize_mut();
    fingerprint_normalized(&normalized)
}

pub fn admit_native_replay(graph: &CanonicalConfigurationGraph) -> ConfigurationReplayAdmission {
    let reasons = native_replay_rejection_reasons(graph);
    if reasons.is_empty() {
        ConfigurationReplayAdmission::Accepted {
            project_count: graph.projects.len(),
            plugin_count: graph.plugins.len(),
        }
    } else {
        ConfigurationReplayAdmission::Rejected(ConfigurationReplayRejection {
            build_id: graph.build_id.clone(),
            reasons,
        })
    }
}

pub fn from_build_plan(plan: &CanonicalBuildPlan) -> CanonicalConfigurationGraph {
    let mut graph = CanonicalConfigurationGraph {
        schema_version: CONFIGURATION_GRAPH_SCHEMA_VERSION,
        build_id: plan.build_id.clone(),
        settings: None,
        projects: plan
            .projects
            .iter()
            .map(|project| CanonicalProjectConfiguration {
                path: project.path.clone(),
                name: project.name.clone(),
                project_dir: project.project_dir.clone(),
                build_file: String::new(),
                group: String::new(),
                version: String::new(),
                plugins: Vec::new(),
                repositories: Vec::new(),
                version_catalog_refs: Vec::new(),
                source_compatibility: String::new(),
                target_compatibility: String::new(),
                warnings: Vec::new(),
            })
            .collect(),
        source_sets: Vec::new(),
        tasks: task_configurations_from_plan(plan),
        plugins: Vec::new(),
        dependency_configurations: dependency_configurations_from_plan(&plan.dependencies),
        toolchains: plan.toolchains.clone(),
        invalidation_inputs: Vec::new(),
        metadata: BTreeMap::from([
            ("source".to_string(), "build-plan-only".to_string()),
            ("projectCount".to_string(), plan.projects.len().to_string()),
            ("taskCount".to_string(), plan.tasks.len().to_string()),
            (
                "dependencyConfigurationCount".to_string(),
                plan.dependencies
                    .iter()
                    .map(|dependency| {
                        (
                            dependency.project_path.as_str(),
                            dependency.configuration.as_str(),
                        )
                    })
                    .collect::<BTreeSet<_>>()
                    .len()
                    .to_string(),
            ),
        ]),
    };
    graph.normalize_mut();
    graph
}

pub fn from_jvm_capture(
    build_id: &str,
    model: &GetBuildModelResponse,
    env: Option<&GetBuildEnvironmentResponse>,
    plan: &CanonicalBuildPlan,
    scripts: &[ConfigurationScript<'_>],
) -> CanonicalConfigurationGraph {
    let settings_script = scripts
        .iter()
        .find(|script| script.kind == ConfigurationScriptKind::Settings);
    let project_scripts = scripts
        .iter()
        .filter(|script| script.kind == ConfigurationScriptKind::Project)
        .collect::<Vec<_>>();

    let projects = project_configurations_from_model(model, &project_scripts);
    let plugins = plugin_models(&projects);
    let source_sets = source_sets_from_projects(&projects);
    let mut invalidation_inputs = scripts
        .iter()
        .map(|script| {
            let kind = match script.kind {
                ConfigurationScriptKind::Settings => "settings-script",
                ConfigurationScriptKind::Project => "build-script",
            };
            configuration_input(script.path, kind)
        })
        .collect::<Vec<_>>();
    invalidation_inputs.extend(version_catalog_inputs(
        model,
        settings_script.map(|s| s.path),
    ));
    invalidation_inputs.extend(init_script_inputs());

    let mut metadata = BTreeMap::from([
        (
            "source".to_string(),
            "jvm-configuration-capture".to_string(),
        ),
        ("projectCount".to_string(), projects.len().to_string()),
        ("taskCount".to_string(), plan.tasks.len().to_string()),
        ("pluginCount".to_string(), plugins.len().to_string()),
        ("sourceSetCount".to_string(), source_sets.len().to_string()),
        (
            "invalidationInputCount".to_string(),
            invalidation_inputs.len().to_string(),
        ),
    ]);
    if let Some(env) = env {
        if !env.gradle_version.is_empty() {
            metadata.insert("gradleVersion".to_string(), env.gradle_version.clone());
        }
        if !env.java_version.is_empty() {
            metadata.insert("javaVersion".to_string(), env.java_version.clone());
        }
    }

    let mut graph = CanonicalConfigurationGraph {
        schema_version: CONFIGURATION_GRAPH_SCHEMA_VERSION,
        build_id: build_id.to_string(),
        settings: settings_script.map(settings_model),
        projects,
        source_sets,
        tasks: task_configurations_from_plan(plan),
        plugins,
        dependency_configurations: dependency_configurations_from_plan(&plan.dependencies),
        toolchains: plan.toolchains.clone(),
        invalidation_inputs,
        metadata,
    };
    graph.normalize_mut();
    graph
}

fn native_replay_rejection_reasons(graph: &CanonicalConfigurationGraph) -> Vec<String> {
    let mut reasons = Vec::new();
    if let Err(error) = validate_schema_version(graph) {
        reasons.push(error);
    }
    if let Some(settings) = &graph.settings {
        for warning in &settings.warnings {
            reasons.push(format!(
                "settings script warning requires JVM replay: {warning}"
            ));
        }
        for repository in settings
            .plugin_repositories
            .iter()
            .chain(settings.dependency_repositories.iter())
        {
            if !native_replay_repository_type_supported(&repository.repo_type) {
                reasons.push(format!(
                    "settings repository '{}' uses unsupported repository type '{}'",
                    repository.name, repository.repo_type
                ));
            }
        }
    }
    for project in &graph.projects {
        for warning in &project.warnings {
            reasons.push(format!(
                "project '{}' script warning requires JVM replay: {}",
                project.path, warning
            ));
        }
        for repository in &project.repositories {
            if !native_replay_repository_type_supported(&repository.repo_type) {
                reasons.push(format!(
                    "project '{}' repository '{}' uses unsupported repository type '{}'",
                    project.path, repository.name, repository.repo_type
                ));
            }
        }
    }
    for plugin in &graph.plugins {
        if plugin.apply && !native_replay_plugin_supported(&plugin.id) {
            reasons.push(format!(
                "applied plugin '{}' on project '{}' is not supported by Rust configuration replay",
                plugin.id, plugin.project_path
            ));
        }
    }
    if graph
        .projects
        .iter()
        .any(|project| !project.version_catalog_refs.is_empty())
        && !graph
            .invalidation_inputs
            .iter()
            .any(|input| input.kind == "version-catalog")
    {
        reasons.push(
            "version catalog references require a captured version-catalog invalidation input"
                .to_string(),
        );
    }
    for input in &graph.invalidation_inputs {
        if input.exists && input.sha256.is_empty() {
            reasons.push(format!(
                "configuration input '{}' for {} is missing a content fingerprint",
                input.path, input.kind
            ));
        }
    }

    reasons.sort_unstable();
    reasons.dedup();
    reasons
}

fn native_replay_plugin_supported(plugin_id: &str) -> bool {
    matches!(
        plugin_id,
        "base"
            | "org.gradle.base"
            | "java"
            | "org.gradle.java"
            | "java-library"
            | "org.gradle.java-library"
            | "application"
            | "org.gradle.application"
    )
}

fn native_replay_repository_type_supported(repo_type: &str) -> bool {
    matches!(
        repo_type,
        "" | "maven" | "mavenCentral" | "mavenLocal" | "google" | "gradlePluginPortal" | "ivy"
    )
}

fn settings_model(script: &ConfigurationScript<'_>) -> CanonicalSettingsModel {
    let plugin_repositories = script
        .parsed
        .plugin_management
        .as_ref()
        .map(|plugin_management| {
            plugin_management
                .repositories
                .iter()
                .map(|repo| CanonicalParsedRepository {
                    name: repo.name.clone(),
                    repo_type: repo.repo_type.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let (repositories_mode, dependency_repositories) = script
        .parsed
        .dependency_resolution_management
        .as_ref()
        .map(|management| {
            (
                management.repositories_mode.clone().unwrap_or_default(),
                management
                    .repositories
                    .iter()
                    .map(repository_from_parsed)
                    .collect(),
            )
        })
        .unwrap_or_default();

    CanonicalSettingsModel {
        settings_file: script.path.to_string(),
        root_project_name: infer_root_project_name(script.path),
        included_projects: script
            .parsed
            .subprojects
            .iter()
            .map(|project| project.path.clone())
            .collect(),
        plugin_repositories,
        dependency_repositories,
        repositories_mode,
        warnings: script.parsed.warnings.clone(),
    }
}

fn project_configurations_from_model(
    model: &GetBuildModelResponse,
    scripts: &[&ConfigurationScript<'_>],
) -> Vec<CanonicalProjectConfiguration> {
    model
        .projects
        .iter()
        .map(|project| {
            let parsed = scripts
                .iter()
                .find(|script| script.project_path == project.path)
                .map(|script| script.parsed);
            let plugins = parsed
                .map(|parsed| {
                    parsed
                        .plugins
                        .iter()
                        .map(|plugin| plugin_application(plugin, "project-script"))
                        .collect()
                })
                .unwrap_or_default();
            CanonicalProjectConfiguration {
                path: project.path.clone(),
                name: project.name.clone(),
                project_dir: infer_project_dir(&project.build_file),
                build_file: project.build_file.clone(),
                group: parsed
                    .and_then(|parsed| parsed.group.clone())
                    .unwrap_or_default(),
                version: parsed
                    .and_then(|parsed| parsed.version.clone())
                    .unwrap_or_default(),
                plugins,
                repositories: parsed
                    .map(|parsed| {
                        parsed
                            .repositories
                            .iter()
                            .map(repository_from_parsed)
                            .collect()
                    })
                    .unwrap_or_default(),
                version_catalog_refs: parsed
                    .map(|parsed| {
                        parsed
                            .catalog_refs
                            .iter()
                            .map(version_catalog_ref)
                            .collect()
                    })
                    .unwrap_or_default(),
                source_compatibility: parsed
                    .and_then(|parsed| parsed.source_compatibility.clone())
                    .unwrap_or_default(),
                target_compatibility: parsed
                    .and_then(|parsed| parsed.target_compatibility.clone())
                    .unwrap_or_default(),
                warnings: parsed
                    .map(|parsed| parsed.warnings.clone())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

fn plugin_application(plugin: &ParsedPlugin, source: &str) -> CanonicalPluginApplication {
    CanonicalPluginApplication {
        id: plugin.id.clone(),
        version: plugin.version.clone().unwrap_or_default(),
        apply: plugin.apply,
        source: source.to_string(),
    }
}

fn plugin_models(projects: &[CanonicalProjectConfiguration]) -> Vec<CanonicalPluginModel> {
    projects
        .iter()
        .flat_map(|project| {
            project.plugins.iter().map(|plugin| CanonicalPluginModel {
                project_path: project.path.clone(),
                id: plugin.id.clone(),
                version: plugin.version.clone(),
                apply: plugin.apply,
                source: plugin.source.clone(),
            })
        })
        .collect()
}

fn source_sets_from_projects(
    projects: &[CanonicalProjectConfiguration],
) -> Vec<CanonicalSourceSet> {
    let mut source_sets = Vec::new();
    for project in projects {
        let has_java = project.plugins.iter().any(|plugin| {
            matches!(
                plugin.id.as_str(),
                "java" | "java-library" | "application" | "org.gradle.java"
            )
        });
        let has_kotlin = project
            .plugins
            .iter()
            .any(|plugin| plugin.id == "org.jetbrains.kotlin.jvm" || plugin.id == "kotlin");
        if !has_java && !has_kotlin {
            continue;
        }
        source_sets.push(default_source_set(project, "main", has_java, has_kotlin));
        source_sets.push(default_source_set(project, "test", has_java, has_kotlin));
    }
    source_sets
}

fn default_source_set(
    project: &CanonicalProjectConfiguration,
    name: &str,
    has_java: bool,
    has_kotlin: bool,
) -> CanonicalSourceSet {
    let base = PathBuf::from(&project.project_dir).join("src").join(name);
    let java_source_dirs = if has_java {
        vec![base.join("java").to_string_lossy().into_owned()]
    } else {
        Vec::new()
    };
    let kotlin_source_dirs = if has_kotlin {
        vec![base.join("kotlin").to_string_lossy().into_owned()]
    } else {
        Vec::new()
    };
    let resource_dirs = vec![base.join("resources").to_string_lossy().into_owned()];
    let (compile_classpath_configuration, runtime_classpath_configuration) = if name == "main" {
        (
            "compileClasspath".to_string(),
            "runtimeClasspath".to_string(),
        )
    } else {
        (
            format!("{name}CompileClasspath"),
            format!("{name}RuntimeClasspath"),
        )
    };

    CanonicalSourceSet {
        project_path: project.path.clone(),
        name: name.to_string(),
        java_source_dirs,
        kotlin_source_dirs,
        resource_dirs,
        compile_classpath_configuration,
        runtime_classpath_configuration,
    }
}

fn task_configurations_from_plan(plan: &CanonicalBuildPlan) -> Vec<CanonicalTaskConfiguration> {
    plan.tasks
        .iter()
        .map(|task| CanonicalTaskConfiguration {
            path: task.path.clone(),
            project_path: task.project_path.clone(),
            implementation_id: task.implementation_id.clone(),
            action_kind: task.action_kind.clone(),
            cacheability: task.cacheability.clone(),
            depends_on: task.depends_on.clone(),
            input_spec_count: task.input_specs.len(),
            output_spec_count: task.output_specs.len(),
        })
        .collect()
}

fn dependency_configurations_from_plan(
    dependencies: &[CanonicalBuildPlanDependency],
) -> Vec<CanonicalDependencyConfiguration> {
    let mut grouped = BTreeMap::<(String, String), CanonicalDependencyConfiguration>::new();
    for dependency in dependencies {
        let key = (
            dependency.project_path.clone(),
            dependency.configuration.clone(),
        );
        let configuration =
            grouped
                .entry(key)
                .or_insert_with(|| CanonicalDependencyConfiguration {
                    project_path: dependency.project_path.clone(),
                    name: dependency.configuration.clone(),
                    dependencies: Vec::new(),
                    constraints: Vec::new(),
                    repositories: Vec::new(),
                    unsupported_features: Vec::new(),
                });
        if dependency.kind == "constraint" {
            configuration.constraints.push(dependency.notation.clone());
        } else {
            configuration.dependencies.push(dependency.notation.clone());
        }
        configuration
            .repositories
            .extend(dependency.repositories.iter().cloned());
        configuration
            .unsupported_features
            .extend(dependency.unsupported_features.iter().cloned());
    }
    grouped.into_values().collect()
}

fn version_catalog_inputs(
    model: &GetBuildModelResponse,
    settings_file: Option<&str>,
) -> Vec<CanonicalConfigurationInput> {
    let root_dir = settings_file
        .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
        .or_else(|| {
            model
                .projects
                .iter()
                .find(|project| project.path == ":")
                .and_then(|project| {
                    Path::new(&project.build_file)
                        .parent()
                        .map(Path::to_path_buf)
                })
        });
    let Some(root_dir) = root_dir else {
        return Vec::new();
    };
    let catalog = root_dir.join("gradle").join("libs.versions.toml");
    if catalog.exists() {
        vec![configuration_input(
            &catalog.to_string_lossy(),
            "version-catalog",
        )]
    } else {
        Vec::new()
    }
}

fn configuration_input(path: &str, kind: &str) -> CanonicalConfigurationInput {
    let path_ref = Path::new(path);
    let exists = path_ref.exists();
    let sha256 = if exists {
        fingerprint_configuration_input_path(path_ref).unwrap_or_default()
    } else {
        String::new()
    };
    CanonicalConfigurationInput {
        path: path.to_string(),
        kind: kind.to_string(),
        exists,
        sha256,
    }
}

fn init_script_inputs() -> Vec<CanonicalConfigurationInput> {
    let Some(gradle_user_home) = gradle_user_home() else {
        return Vec::new();
    };
    init_script_inputs_from_gradle_user_home(&gradle_user_home)
}

fn init_script_inputs_from_gradle_user_home(
    gradle_user_home: &Path,
) -> Vec<CanonicalConfigurationInput> {
    [
        (gradle_user_home.join("init.gradle"), "init-script"),
        (gradle_user_home.join("init.gradle.kts"), "init-script"),
        (gradle_user_home.join("init.d"), "init-script-directory"),
    ]
    .into_iter()
    .map(|(path, kind)| configuration_input(&path.to_string_lossy(), kind))
    .collect()
}

fn gradle_user_home() -> Option<PathBuf> {
    std::env::var_os("GRADLE_USER_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".gradle"))
        })
}

pub fn fingerprint_configuration_input_path(
    path: &Path,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    if path.is_file() {
        let bytes = std::fs::read(path)?;
        return Ok(format!("{:x}", Sha256::digest(bytes)));
    }
    if path.is_dir() {
        return fingerprint_configuration_input_directory(path);
    }
    Ok(String::new())
}

fn fingerprint_configuration_input_directory(
    path: &Path,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut files = Vec::new();
    collect_configuration_input_directory_files(path, path, &mut files)?;
    let mut hasher = Sha256::new();
    for (relative_path, file_path) in files {
        hasher.update(relative_path.as_bytes());
        hasher.update([0]);
        hasher.update(fingerprint_configuration_input_path(&file_path)?.as_bytes());
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_configuration_input_directory_files(
    root: &Path,
    path: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for entry in std::fs::read_dir(path)? {
        let child = entry?.path();
        let metadata = std::fs::metadata(&child)?;
        if metadata.is_dir() {
            collect_configuration_input_directory_files(root, &child, files)?;
        } else if metadata.is_file() {
            let relative_path = child
                .strip_prefix(root)?
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            files.push((relative_path, child));
        }
    }
    files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(())
}

fn repository_from_parsed(repository: &ParsedRepository) -> CanonicalParsedRepository {
    CanonicalParsedRepository {
        name: repository.name.clone(),
        repo_type: repository.repo_type.clone(),
    }
}

fn version_catalog_ref(reference: &ParsedVersionCatalogRef) -> CanonicalVersionCatalogRef {
    CanonicalVersionCatalogRef {
        configuration: reference.configuration.clone(),
        alias: reference.alias.clone(),
    }
}

fn infer_project_dir(build_file: &str) -> String {
    if build_file.is_empty() {
        return String::new();
    }
    Path::new(build_file)
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn infer_root_project_name(settings_file: &str) -> String {
    Path::new(settings_file)
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("root")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::build_plan_ir::{
        CanonicalBuildPlanProject, CanonicalBuildPlanTask, BUILD_PLAN_SCHEMA_VERSION,
    };
    use crate::server::build_script_parser::parse_build_script_file;

    fn sample_plan(project_dir: &Path) -> CanonicalBuildPlan {
        CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "build-config".to_string(),
            projects: vec![CanonicalBuildPlanProject {
                path: ":".to_string(),
                name: "root".to_string(),
                project_dir: project_dir.to_string_lossy().into_owned(),
            }],
            tasks: vec![CanonicalBuildPlanTask {
                path: ":compileJava".to_string(),
                project_path: ":".to_string(),
                implementation_id: "org.gradle.api.tasks.compile.JavaCompile".to_string(),
                depends_on: vec![":classes".to_string()],
                inputs: BTreeMap::new(),
                outputs: Vec::new(),
                worker_isolation: "process".to_string(),
                should_run_after: Vec::new(),
                must_run_after: Vec::new(),
                finalized_by: Vec::new(),
                cacheability: "cacheable".to_string(),
                local_state: Vec::new(),
                destroyables: Vec::new(),
                action_kind: "compile".to_string(),
                input_specs: Vec::new(),
                output_specs: Vec::new(),
                environment_inputs: Vec::new(),
                system_property_inputs: Vec::new(),
                diagnostics: Vec::new(),
            }],
            dependencies: vec![CanonicalBuildPlanDependency {
                project_path: ":".to_string(),
                configuration: "implementation".to_string(),
                notation: "org.example:lib:1.0".to_string(),
                kind: "dependency".to_string(),
                repositories: Vec::new(),
                unsupported_features: Vec::new(),
            }],
            toolchains: vec![CanonicalBuildPlanToolchainRequest {
                language: "java".to_string(),
                version: "17".to_string(),
                vendor: "test".to_string(),
                implementation: "jvm".to_string(),
            }],
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn configuration_graph_from_capture_includes_settings_plugins_source_sets_and_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let settings = temp.path().join("settings.gradle.kts");
        let build = temp.path().join("build.gradle.kts");
        let catalog_dir = temp.path().join("gradle");
        std::fs::create_dir_all(&catalog_dir).unwrap();
        std::fs::write(
            catalog_dir.join("libs.versions.toml"),
            "[versions]\njava = \"17\"\n",
        )
        .unwrap();
        std::fs::write(
            &settings,
            r#"
                pluginManagement { repositories { gradlePluginPortal() } }
                dependencyResolutionManagement {
                    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
                    repositories { mavenCentral() }
                }
                include(":app")
            "#,
        )
        .unwrap();
        std::fs::write(
            &build,
            r#"
                plugins { java }
                repositories { mavenCentral() }
                dependencies { implementation(libs.example.lib) }
                group = "org.example"
                version = "1.0"
            "#,
        )
        .unwrap();

        let model = GetBuildModelResponse {
            projects: vec![crate::proto::ProjectModel {
                path: ":".to_string(),
                name: "root".to_string(),
                build_file: build.to_string_lossy().into_owned(),
                subprojects: vec![":app".to_string()],
            }],
        };
        let env = GetBuildEnvironmentResponse {
            java_version: "17.0.10".to_string(),
            gradle_version: "9.6".to_string(),
            ..Default::default()
        };
        let settings_parsed = parse_build_script_file(&settings).unwrap();
        let build_parsed = parse_build_script_file(&build).unwrap();
        let settings_path = settings.to_string_lossy();
        let build_path = build.to_string_lossy();
        let scripts = vec![
            ConfigurationScript {
                kind: ConfigurationScriptKind::Settings,
                project_path: ":",
                path: &settings_path,
                parsed: &settings_parsed,
            },
            ConfigurationScript {
                kind: ConfigurationScriptKind::Project,
                project_path: ":",
                path: &build_path,
                parsed: &build_parsed,
            },
        ];

        let graph = from_jvm_capture(
            "build-config",
            &model,
            Some(&env),
            &sample_plan(temp.path()),
            &scripts,
        );

        assert_eq!(graph.schema_version, CONFIGURATION_GRAPH_SCHEMA_VERSION);
        assert_eq!(
            graph.settings.as_ref().unwrap().included_projects,
            vec![":app"]
        );
        assert_eq!(graph.projects[0].group, "org.example");
        assert!(graph.plugins.iter().any(|plugin| plugin.id == "java"));
        assert!(graph
            .source_sets
            .iter()
            .any(|source_set| source_set.name == "main"
                && source_set.java_source_dirs[0].ends_with("src/main/java")));
        assert!(graph
            .dependency_configurations
            .iter()
            .any(|configuration| configuration.name == "implementation"
                && configuration.dependencies == vec!["org.example:lib:1.0"]));
        assert!(graph
            .invalidation_inputs
            .iter()
            .any(|input| input.kind == "settings-script"
                && input.exists
                && !input.sha256.is_empty()));
        assert!(graph
            .invalidation_inputs
            .iter()
            .any(|input| input.kind == "version-catalog"
                && input.exists
                && !input.sha256.is_empty()));
        assert_eq!(
            admit_native_replay(&graph),
            ConfigurationReplayAdmission::Accepted {
                project_count: 1,
                plugin_count: 1
            }
        );
    }

    #[test]
    fn native_replay_admission_rejects_applied_external_plugin() {
        let temp = tempfile::tempdir().unwrap();
        let mut graph = from_build_plan(&sample_plan(temp.path()));
        graph.plugins.push(CanonicalPluginModel {
            project_path: ":".to_string(),
            id: "com.example.custom".to_string(),
            version: "1.0".to_string(),
            apply: true,
            source: "project-script".to_string(),
        });

        let ConfigurationReplayAdmission::Rejected(rejection) = admit_native_replay(&graph) else {
            panic!("expected external plugin to reject native configuration replay");
        };

        assert!(rejection
            .message()
            .contains("applied plugin 'com.example.custom'"));
    }

    #[test]
    fn init_script_inputs_track_missing_files_and_init_directory() {
        let temp = tempfile::tempdir().unwrap();
        let init_dir = temp.path().join("init.d");
        std::fs::create_dir_all(&init_dir).unwrap();
        std::fs::write(init_dir.join("tooling.gradle.kts"), "println(\"init\")\n").unwrap();

        let inputs = init_script_inputs_from_gradle_user_home(temp.path());

        assert!(inputs.iter().any(|input| input.kind == "init-script"
            && input.path.ends_with("init.gradle")
            && !input.exists));
        let init_directory = inputs
            .iter()
            .find(|input| input.kind == "init-script-directory")
            .expect("expected init.d sentinel");
        assert!(init_directory.exists);
        assert!(!init_directory.sha256.is_empty());
    }

    #[test]
    fn configuration_directory_fingerprint_changes_when_child_file_appears() {
        let temp = tempfile::tempdir().unwrap();
        let init_dir = temp.path().join("init.d");
        std::fs::create_dir_all(&init_dir).unwrap();
        std::fs::write(init_dir.join("a.gradle"), "println(\"a\")\n").unwrap();
        let before = fingerprint_configuration_input_path(&init_dir).unwrap();

        std::fs::write(init_dir.join("b.gradle"), "println(\"b\")\n").unwrap();
        let after = fingerprint_configuration_input_path(&init_dir).unwrap();

        assert_ne!(before, after);
    }

    #[test]
    fn configuration_graph_fingerprint_is_order_insensitive() {
        let temp = tempfile::tempdir().unwrap();
        let mut graph = from_build_plan(&sample_plan(temp.path()));
        graph.projects.push(CanonicalProjectConfiguration {
            path: ":app".to_string(),
            name: "app".to_string(),
            project_dir: temp.path().join("app").to_string_lossy().into_owned(),
            build_file: String::new(),
            group: String::new(),
            version: String::new(),
            plugins: Vec::new(),
            repositories: Vec::new(),
            version_catalog_refs: Vec::new(),
            source_compatibility: String::new(),
            target_compatibility: String::new(),
            warnings: Vec::new(),
        });
        let mut reordered = graph.clone();
        reordered.projects.reverse();

        assert_eq!(
            fingerprint_sha256_hex(&graph).unwrap(),
            fingerprint_sha256_hex(&reordered).unwrap()
        );
    }

    #[test]
    fn schema_version_validation_rejects_unknown_configuration_graph_version() {
        let temp = tempfile::tempdir().unwrap();
        let mut graph = from_build_plan(&sample_plan(temp.path()));
        graph.schema_version = CONFIGURATION_GRAPH_SCHEMA_VERSION + 1;

        assert!(validate_schema_version(&graph)
            .unwrap_err()
            .contains("unsupported configuration graph schema version"));
    }
}
