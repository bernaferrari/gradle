use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::client::jvm_host_bridge::JvmHostBridge;
use crate::proto::{GetBuildEnvironmentResponse, GetBuildModelResponse};

use super::atomic_write::AtomicWriter;
use super::build_plan_ir::{
    fingerprint_normalized, from_proto, validate_schema_version, CanonicalBuildPlan,
    CanonicalBuildPlanDependency, CanonicalBuildPlanProject, CanonicalBuildPlanRepository,
    CanonicalBuildPlanTask, CanonicalBuildPlanTaskDiagnostic, CanonicalBuildPlanTaskInputSpec,
    CanonicalBuildPlanTaskOutputSpec, CanonicalBuildPlanToolchainRequest,
    BUILD_PLAN_SCHEMA_VERSION,
};
use super::build_script_parser::parse_build_script_file;
use super::build_script_types::BuildScriptParseResult;

const SHADOWED_CONFIGURATIONS: &[&str] = &[
    "classpath",
    "compileClasspath",
    "runtimeClasspath",
    "testCompileClasspath",
    "testRuntimeClasspath",
    "annotationProcessor",
    "kapt",
    "ksp",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuildPlanShadowArtifact {
    pub plan: CanonicalBuildPlan,
    pub fingerprint_sha256: String,
    pub stored_at_ms: i64,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct BuildPlanShadowStore {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlanShadowDiffReport {
    pub build_id: String,
    pub mismatches: Vec<String>,
}

#[derive(Debug, Clone)]
struct ParsedProjectBuildScript {
    project_path: String,
    parsed: BuildScriptParseResult,
}

impl BuildPlanShadowDiffReport {
    pub fn is_match(&self) -> bool {
        self.mismatches.is_empty()
    }
}

impl BuildPlanShadowStore {
    pub fn new(config_cache_dir: PathBuf) -> Self {
        let root = config_cache_dir.join("build-plan-shadow");
        std::fs::create_dir_all(&root).ok();
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn persist_plan(
        &self,
        plan: &CanonicalBuildPlan,
        source: &str,
    ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        let path = self.artifact_path(&plan.build_id);
        if !is_inline_shadow_source(source) && path.exists() {
            let bytes = std::fs::read(&path)?;
            let existing: BuildPlanShadowArtifact = serde_json::from_slice(&bytes)?;
            if is_inline_shadow_source(&existing.source)
                || existing
                    .plan
                    .metadata
                    .get("source")
                    .is_some_and(|source| is_inline_shadow_source(source))
            {
                return Ok(path);
            }
        }

        validate_schema_version(plan)
            .map_err(|e| format!("build plan schema validation failed: {}", e))?;

        // Clone once, normalize in-place, then fingerprint without further cloning.
        let mut normalized = plan.clone();
        normalized.normalize_mut();
        let fingerprint = fingerprint_normalized(&normalized)?;

        let artifact = BuildPlanShadowArtifact {
            plan: normalized,
            fingerprint_sha256: fingerprint,
            stored_at_ms: now_ms(),
            source: source.to_string(),
        };

        let payload = serde_json::to_vec_pretty(&artifact)?;
        let mut writer = AtomicWriter::new(path.clone());
        writer.write_all(&payload)?;
        writer.commit()?;
        Ok(path)
    }

    pub fn load_plan(
        &self,
        build_id: &str,
    ) -> Result<Option<BuildPlanShadowArtifact>, Box<dyn std::error::Error + Send + Sync>> {
        let path = self.artifact_path_for_build_id(build_id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let artifact: BuildPlanShadowArtifact = serde_json::from_slice(&bytes)?;
        self.validate_loaded_artifact(build_id, &path, &artifact)?;
        Ok(Some(artifact))
    }

    pub fn artifact_path_for_build_id(&self, build_id: &str) -> PathBuf {
        self.root.join(keyed_artifact_filename(build_id))
    }

    fn artifact_path(&self, build_id: &str) -> PathBuf {
        self.artifact_path_for_build_id(build_id)
    }

    fn validate_loaded_artifact(
        &self,
        build_id: &str,
        path: &Path,
        artifact: &BuildPlanShadowArtifact,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if artifact.plan.build_id != build_id {
            let reason = format!(
                "build plan shadow artifact build id mismatch: requested '{}' but artifact contains '{}'",
                build_id, artifact.plan.build_id
            );
            self.quarantine_artifact(path, &reason)?;
            return Err(reason.into());
        }
        if let Err(error) = validate_schema_version(&artifact.plan) {
            let reason = format!(
                "build plan shadow artifact schema validation failed: {}",
                error
            );
            self.quarantine_artifact(path, &reason)?;
            return Err(reason.into());
        }
        let mut normalized = artifact.plan.clone();
        normalized.normalize_mut();
        let actual_fingerprint = fingerprint_normalized(&normalized)?;
        if actual_fingerprint != artifact.fingerprint_sha256 {
            let reason = format!(
                "build plan shadow artifact fingerprint mismatch: computed '{}' but artifact stores '{}'",
                actual_fingerprint, artifact.fingerprint_sha256
            );
            self.quarantine_artifact(path, &reason)?;
            return Err(reason.into());
        }
        Ok(())
    }

    fn quarantine_artifact(
        &self,
        path: &Path,
        reason: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !path.exists() {
            return Ok(());
        }
        let quarantine_dir = self.root.join("quarantine");
        std::fs::create_dir_all(&quarantine_dir)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("shadow-artifact");
        let quarantine_path = quarantine_dir.join(format!("{}-{}.corrupt", now_ms(), file_name));
        std::fs::rename(path, &quarantine_path)?;
        let reason_path = quarantine_path.with_extension("reason.txt");
        std::fs::write(reason_path, reason)?;
        Ok(())
    }
}

pub async fn capture_and_persist_shadow_from_jvm(
    bridge: &JvmHostBridge,
    store: &BuildPlanShadowStore,
    build_id: &str,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(existing) = store.load_plan(build_id)? {
        if is_inline_shadow_source(&existing.source)
            || existing
                .plan
                .metadata
                .get("source")
                .is_some_and(|source| is_inline_shadow_source(source))
        {
            return Ok(Some(store.artifact_path_for_build_id(build_id)));
        }
    }

    let Some(model) = bridge.get_build_model(build_id).await? else {
        return Ok(None);
    };
    let env = bridge.get_build_environment().await?;
    let host_plan = get_successful_host_plan(bridge, build_id).await?;

    let plan =
        canonical_plan_from_jvm_bridge(bridge, build_id, &model, env.as_ref(), host_plan).await?;
    if let Some(existing) = store.load_plan(build_id)? {
        if is_inline_shadow_source(&existing.source)
            || existing
                .plan
                .metadata
                .get("source")
                .is_some_and(|source| is_inline_shadow_source(source))
        {
            return Ok(Some(store.artifact_path_for_build_id(build_id)));
        }
    }
    let path = store.persist_plan(&plan, "jvm-host-shadow")?;
    if let Some(artifact) = store.load_plan(build_id)? {
        let diff = diff_expected_vs_artifact(&plan, &artifact);
        if !diff.is_match() {
            return Err(format!(
                "shadow artifact persisted with mismatches: {}",
                diff.mismatches.join("; ")
            )
            .into());
        }
    }
    Ok(Some(path))
}

pub async fn verify_shadow_against_jvm(
    bridge: &JvmHostBridge,
    store: &BuildPlanShadowStore,
    build_id: &str,
) -> Result<BuildPlanShadowDiffReport, Box<dyn std::error::Error + Send + Sync>> {
    let Some(model) = bridge.get_build_model(build_id).await? else {
        return Ok(BuildPlanShadowDiffReport {
            build_id: build_id.to_string(),
            mismatches: vec!["JVM build model unavailable".to_string()],
        });
    };
    let env = bridge.get_build_environment().await?;
    let host_plan = get_successful_host_plan(bridge, build_id).await?;
    let expected =
        canonical_plan_from_jvm_bridge(bridge, build_id, &model, env.as_ref(), host_plan).await?;
    let Some(artifact) = store.load_plan(build_id)? else {
        return Ok(BuildPlanShadowDiffReport {
            build_id: build_id.to_string(),
            mismatches: vec!["missing shadow artifact".to_string()],
        });
    };

    Ok(diff_expected_vs_artifact(&expected, &artifact))
}

fn is_inline_shadow_source(source: &str) -> bool {
    matches!(
        source,
        "task-graph-listener-inline" | "finalized-execution-plan-inline" | "inline-build-plan"
    )
}

pub async fn canonical_plan_from_jvm_bridge(
    bridge: &JvmHostBridge,
    build_id: &str,
    model: &GetBuildModelResponse,
    env: Option<&GetBuildEnvironmentResponse>,
    host_plan: Option<CanonicalBuildPlan>,
) -> Result<CanonicalBuildPlan, Box<dyn std::error::Error + Send + Sync>> {
    let dependencies = collect_shadow_dependencies(bridge, build_id, model).await?;
    let parsed_scripts = collect_parsed_build_scripts(model);
    Ok(canonical_plan_from_jvm(
        build_id,
        model,
        env,
        dependencies,
        &parsed_scripts,
        host_plan,
    ))
}

async fn get_successful_host_plan(
    bridge: &JvmHostBridge,
    build_id: &str,
) -> Result<Option<CanonicalBuildPlan>, Box<dyn std::error::Error + Send + Sync>> {
    let Some(response) = bridge.get_build_plan(build_id).await? else {
        return Ok(None);
    };
    if !response.success {
        return Ok(None);
    }
    Ok(response.plan.map(|plan| from_proto(&plan)))
}

fn canonical_plan_from_jvm(
    build_id: &str,
    model: &GetBuildModelResponse,
    env: Option<&GetBuildEnvironmentResponse>,
    dependencies: Vec<CanonicalBuildPlanDependency>,
    parsed_scripts: &[ParsedProjectBuildScript],
    host_plan: Option<CanonicalBuildPlan>,
) -> CanonicalBuildPlan {
    let host_projects = host_plan
        .as_ref()
        .map(|plan| plan.projects.clone())
        .filter(|projects| !projects.is_empty());
    let host_tasks = host_plan.as_ref().map(|plan| plan.tasks.clone());

    let mut projects: Vec<CanonicalBuildPlanProject> = host_projects.unwrap_or_else(|| {
        model
            .projects
            .iter()
            .map(|p| CanonicalBuildPlanProject {
                path: p.path.clone(),
                name: p.name.clone(),
                project_dir: infer_project_dir(&p.build_file),
            })
            .collect()
    });

    if projects.is_empty() {
        projects.push(CanonicalBuildPlanProject {
            path: ":".to_string(),
            name: "root".to_string(),
            project_dir: String::new(),
        });
    }

    let mut tasks = host_tasks.unwrap_or_default();
    let task_source = if !tasks.is_empty() {
        "jvm-host-build-plan"
    } else {
        tasks = collect_script_declared_tasks(parsed_scripts);
        if tasks.is_empty() {
            "no-task-contracts"
        } else {
            "parsed-build-scripts"
        }
    };
    let task_dependency_edge_count = parsed_scripts
        .iter()
        .map(|script| {
            script
                .parsed
                .task_configs
                .iter()
                .map(|task| task.depends_on.len())
                .sum::<usize>()
        })
        .sum::<usize>();
    let task_soft_dependency_edge_count = parsed_scripts
        .iter()
        .map(|script| {
            script
                .parsed
                .task_configs
                .iter()
                .map(|task| task.should_run_after.len())
                .sum::<usize>()
        })
        .sum::<usize>();
    let task_must_run_after_edge_count = parsed_scripts
        .iter()
        .map(|script| {
            script
                .parsed
                .task_configs
                .iter()
                .map(|task| task.must_run_after.len())
                .sum::<usize>()
        })
        .sum::<usize>();
    let task_finalizer_edge_count = parsed_scripts
        .iter()
        .map(|script| {
            script
                .parsed
                .task_configs
                .iter()
                .map(|task| task.finalized_by.len())
                .sum::<usize>()
        })
        .sum::<usize>();
    let disabled_task_count = parsed_scripts
        .iter()
        .map(|script| {
            script
                .parsed
                .task_configs
                .iter()
                .filter(|task| !task.enabled)
                .count()
        })
        .sum::<usize>();
    let buildscript_classpath_dependency_count = parsed_scripts
        .iter()
        .map(|script| script.parsed.buildscript_deps.len())
        .sum::<usize>();
    let version_catalog_ref_count = parsed_scripts
        .iter()
        .map(|script| script.parsed.catalog_refs.len())
        .sum::<usize>();
    let subproject_include_count = parsed_scripts
        .iter()
        .map(|script| script.parsed.subprojects.len())
        .sum::<usize>();
    let group_assignment_count = parsed_scripts
        .iter()
        .filter(|script| script.parsed.group.is_some())
        .count();
    let version_assignment_count = parsed_scripts
        .iter()
        .filter(|script| script.parsed.version.is_some())
        .count();
    let dependency_configuration_count = parsed_scripts
        .iter()
        .flat_map(|script| {
            script
                .parsed
                .dependencies
                .iter()
                .map(|dependency| dependency.configuration.clone())
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>()
        .len();
    let typed_task_count = parsed_scripts
        .iter()
        .map(|script| {
            script
                .parsed
                .task_configs
                .iter()
                .filter(|task| task.task_type.is_some())
                .count()
        })
        .sum::<usize>();
    let declared_task_output_count = parsed_scripts
        .iter()
        .map(|script| {
            script
                .parsed
                .task_configs
                .iter()
                .map(|task| task.declared_outputs.len())
                .sum::<usize>()
        })
        .sum::<usize>();
    let task_input_spec_count = tasks
        .iter()
        .map(|task| task.input_specs.len())
        .sum::<usize>();
    let task_output_spec_count = tasks
        .iter()
        .map(|task| task.output_specs.len())
        .sum::<usize>();
    let task_diagnostic_count = tasks
        .iter()
        .map(|task| task.diagnostics.len())
        .sum::<usize>();

    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("source".to_string(), "jvm-host-shadow".to_string());
    if let Some(plan) = host_plan.as_ref() {
        for (key, value) in &plan.metadata {
            metadata.insert(format!("jvmHost.{key}"), value.clone());
        }
    }
    metadata.insert("taskSource".to_string(), task_source.to_string());
    metadata.insert("projectCount".to_string(), model.projects.len().to_string());
    metadata.insert(
        "dependencyCount".to_string(),
        dependencies.len().to_string(),
    );
    metadata.insert("taskCount".to_string(), tasks.len().to_string());
    metadata.insert(
        "parsedBuildScriptCount".to_string(),
        parsed_scripts.len().to_string(),
    );
    metadata.insert(
        "missingBuildScriptCount".to_string(),
        model
            .projects
            .len()
            .saturating_sub(parsed_scripts.len())
            .to_string(),
    );
    metadata.insert(
        "declaredDependencyCount".to_string(),
        parsed_scripts
            .iter()
            .map(|script| script.parsed.dependencies.len())
            .sum::<usize>()
            .to_string(),
    );
    metadata.insert(
        "pluginCount".to_string(),
        parsed_scripts
            .iter()
            .map(|script| script.parsed.plugins.len())
            .sum::<usize>()
            .to_string(),
    );
    metadata.insert(
        "repositoryCount".to_string(),
        parsed_scripts
            .iter()
            .map(repository_count)
            .sum::<usize>()
            .to_string(),
    );
    metadata.insert(
        "scriptWarningCount".to_string(),
        parsed_scripts
            .iter()
            .map(|script| script.parsed.warnings.len())
            .sum::<usize>()
            .to_string(),
    );
    metadata.insert(
        "taskDependencyEdgeCount".to_string(),
        task_dependency_edge_count.to_string(),
    );
    metadata.insert(
        "taskSoftDependencyEdgeCount".to_string(),
        task_soft_dependency_edge_count.to_string(),
    );
    metadata.insert(
        "taskMustRunAfterEdgeCount".to_string(),
        task_must_run_after_edge_count.to_string(),
    );
    metadata.insert(
        "taskFinalizerEdgeCount".to_string(),
        task_finalizer_edge_count.to_string(),
    );
    metadata.insert(
        "disabledTaskCount".to_string(),
        disabled_task_count.to_string(),
    );
    metadata.insert(
        "buildscriptClasspathDependencyCount".to_string(),
        buildscript_classpath_dependency_count.to_string(),
    );
    metadata.insert(
        "versionCatalogRefCount".to_string(),
        version_catalog_ref_count.to_string(),
    );
    metadata.insert(
        "subprojectIncludeCount".to_string(),
        subproject_include_count.to_string(),
    );
    metadata.insert(
        "groupAssignmentCount".to_string(),
        group_assignment_count.to_string(),
    );
    metadata.insert(
        "versionAssignmentCount".to_string(),
        version_assignment_count.to_string(),
    );
    metadata.insert(
        "dependencyConfigurationCount".to_string(),
        dependency_configuration_count.to_string(),
    );
    metadata.insert("typedTaskCount".to_string(), typed_task_count.to_string());
    metadata.insert(
        "declaredTaskOutputCount".to_string(),
        declared_task_output_count.to_string(),
    );
    metadata.insert(
        "taskInputSpecCount".to_string(),
        task_input_spec_count.to_string(),
    );
    metadata.insert(
        "taskOutputSpecCount".to_string(),
        task_output_spec_count.to_string(),
    );
    metadata.insert(
        "taskDiagnosticCount".to_string(),
        task_diagnostic_count.to_string(),
    );
    if let Some(env) = env {
        if !env.gradle_version.is_empty() {
            metadata.insert("gradleVersion".to_string(), env.gradle_version.clone());
        }
        if !env.java_version.is_empty() {
            metadata.insert("javaVersion".to_string(), env.java_version.clone());
        }
    }

    let toolchains = collect_shadow_toolchains(parsed_scripts, env);

    CanonicalBuildPlan {
        schema_version: BUILD_PLAN_SCHEMA_VERSION,
        build_id: build_id.to_string(),
        projects,
        tasks,
        dependencies,
        toolchains,
        metadata,
    }
    .normalized()
}

async fn collect_shadow_dependencies(
    bridge: &JvmHostBridge,
    build_id: &str,
    model: &GetBuildModelResponse,
) -> Result<Vec<CanonicalBuildPlanDependency>, Box<dyn std::error::Error + Send + Sync>> {
    let mut unique = BTreeMap::<
        (String, String, String, String),
        (Vec<CanonicalBuildPlanRepository>, Vec<String>),
    >::new();

    for project in &model.projects {
        for configuration in SHADOWED_CONFIGURATIONS {
            let Some(response) = bridge
                .resolve_configuration(build_id, configuration, &project.path)
                .await?
            else {
                continue;
            };

            if !response.success {
                continue;
            }

            for artifact in response.artifacts {
                let notation = dependency_notation(
                    &artifact.group,
                    &artifact.name,
                    &artifact.version,
                    &artifact.classifier,
                    &artifact.extension,
                );
                if notation.is_empty() {
                    continue;
                }
                let configuration_name = if artifact.configuration.is_empty() {
                    (*configuration).to_string()
                } else {
                    artifact.configuration
                };
                let kind = if artifact.kind.trim().is_empty() {
                    "dependency".to_string()
                } else {
                    artifact.kind
                };
                unique
                    .entry((project.path.clone(), configuration_name, notation, kind))
                    .or_insert_with(|| {
                        (
                            artifact
                                .repositories
                                .iter()
                                .map(|repository| CanonicalBuildPlanRepository {
                                    id: repository.id.clone(),
                                    url: repository.url.clone(),
                                    m2compatible: repository.m2compatible,
                                    allow_insecure_protocol: repository.allow_insecure_protocol,
                                    credentials: repository
                                        .credentials
                                        .iter()
                                        .map(|(k, v)| (k.clone(), v.clone()))
                                        .collect(),
                                    layout: repository.layout.clone(),
                                    ivy_pattern: repository.ivy_pattern.clone(),
                                    include_groups: repository.include_groups.clone(),
                                    exclude_groups: repository.exclude_groups.clone(),
                                    include_group_prefixes: repository
                                        .include_group_prefixes
                                        .clone(),
                                    exclude_group_prefixes: repository
                                        .exclude_group_prefixes
                                        .clone(),
                                    include_modules: repository.include_modules.clone(),
                                    exclude_modules: repository.exclude_modules.clone(),
                                    include_module_versions: repository
                                        .include_module_versions
                                        .clone(),
                                    exclude_module_versions: repository
                                        .exclude_module_versions
                                        .clone(),
                                })
                                .collect(),
                            artifact.unsupported_features.clone(),
                        )
                    });
            }
        }
    }

    Ok(unique
        .into_iter()
        .map(
            |(
                (project_path, configuration, notation, kind),
                (repositories, unsupported_features),
            )| {
                CanonicalBuildPlanDependency {
                    project_path,
                    configuration,
                    notation,
                    kind,
                    repositories,
                    unsupported_features,
                }
            },
        )
        .collect())
}

fn dependency_notation(
    group: &str,
    name: &str,
    version: &str,
    classifier: &str,
    extension: &str,
) -> String {
    let classifier = classifier.trim();
    let extension = extension.trim();
    let artifact_suffix = match (
        classifier.is_empty(),
        extension.is_empty() || extension == "jar",
    ) {
        (true, true) => String::new(),
        (true, false) => format!("@{extension}"),
        (false, true) => format!(":{classifier}"),
        (false, false) => format!(":{classifier}@{extension}"),
    };
    match (group.is_empty(), name.is_empty(), version.is_empty()) {
        (_, true, _) => String::new(),
        (false, false, false) => format!("{group}:{name}:{version}{artifact_suffix}"),
        (false, false, true) => format!("{group}:{name}"),
        (true, false, false) => format!("{name}:{version}"),
        (true, false, true) => name.to_string(),
    }
}

fn collect_parsed_build_scripts(model: &GetBuildModelResponse) -> Vec<ParsedProjectBuildScript> {
    model
        .projects
        .iter()
        .filter_map(|project| {
            if project.build_file.is_empty() {
                return None;
            }
            let path = Path::new(&project.build_file);
            let parsed = parse_build_script_file(path).ok()?;
            Some(ParsedProjectBuildScript {
                project_path: project.path.clone(),
                parsed,
            })
        })
        .collect()
}

fn collect_script_declared_tasks(
    parsed_scripts: &[ParsedProjectBuildScript],
) -> Vec<CanonicalBuildPlanTask> {
    parsed_scripts
        .iter()
        .flat_map(|script| {
            script
                .parsed
                .task_configs
                .iter()
                .map(|task| {
                    let path = qualify_task_path(&script.project_path, &task.task_name);
                    let implementation_id = task
                        .task_type
                        .as_deref()
                        .map(implementation_id_for_task_type)
                        .unwrap_or_else(|| "org.gradle.api.DefaultTask".to_string());
                    let outputs = task.declared_outputs.clone();
                    let inputs = declared_task_inputs(script, task);
                    CanonicalBuildPlanTask {
                        path,
                        project_path: script.project_path.clone(),
                        implementation_id,
                        depends_on: task
                            .depends_on
                            .iter()
                            .map(|dependency| qualify_task_path(&script.project_path, dependency))
                            .collect(),
                        input_specs: input_specs_from_map(&inputs),
                        output_specs: output_specs_from_paths(&outputs),
                        inputs,
                        outputs,
                        worker_isolation: worker_isolation_for_task_type(task.task_type.as_deref())
                            .to_string(),
                        should_run_after: task
                            .should_run_after
                            .iter()
                            .map(|dependency| qualify_task_path(&script.project_path, dependency))
                            .collect(),
                        must_run_after: task
                            .must_run_after
                            .iter()
                            .map(|dependency| qualify_task_path(&script.project_path, dependency))
                            .collect(),
                        finalized_by: task
                            .finalized_by
                            .iter()
                            .map(|dependency| qualify_task_path(&script.project_path, dependency))
                            .collect(),
                        cacheability: if task.declared_outputs.is_empty() {
                            "unknown".to_string()
                        } else {
                            "declared-outputs".to_string()
                        },
                        local_state: Vec::new(),
                        destroyables: Vec::new(),
                        action_kind: action_kind_for_task_type(task.task_type.as_deref())
                            .to_string(),
                        environment_inputs: Vec::new(),
                        system_property_inputs: Vec::new(),
                        diagnostics: vec![CanonicalBuildPlanTaskDiagnostic {
                            severity: "info".to_string(),
                            code: "declared-task-contract".to_string(),
                            message:
                                "Task contract was derived from build-script declarations only"
                                    .to_string(),
                            source: "build-script-parser".to_string(),
                        }],
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn declared_task_inputs(
    script: &ParsedProjectBuildScript,
    task: &super::build_script_types::ParsedTaskConfig,
) -> std::collections::BTreeMap<String, String> {
    let mut inputs = std::collections::BTreeMap::new();
    inputs.insert("source".to_string(), "build-script-parser".to_string());
    inputs.insert("enabled".to_string(), task.enabled.to_string());
    if let Some(task_type) = task.task_type.as_deref() {
        inputs.insert("taskType".to_string(), task_type.to_string());
    }
    if let Some(line) = task.line {
        inputs.insert("declarationLine".to_string(), line.to_string());
    }
    if let Some(group) = script.parsed.group.as_deref() {
        inputs.insert("projectGroup".to_string(), group.to_string());
    }
    if let Some(version) = script.parsed.version.as_deref() {
        inputs.insert("projectVersion".to_string(), version.to_string());
    }
    if let Some(source) = script.parsed.source_compatibility.as_deref() {
        inputs.insert(
            "sourceCompatibility".to_string(),
            normalize_java_version(source),
        );
    }
    if let Some(target) = script.parsed.target_compatibility.as_deref() {
        inputs.insert(
            "targetCompatibility".to_string(),
            normalize_java_version(target),
        );
    }
    inputs
}

fn input_specs_from_map(
    inputs: &std::collections::BTreeMap<String, String>,
) -> Vec<CanonicalBuildPlanTaskInputSpec> {
    inputs
        .iter()
        .map(|(name, value)| CanonicalBuildPlanTaskInputSpec {
            name: name.clone(),
            kind: "value".to_string(),
            value: value.clone(),
            normalization: "scalar".to_string(),
            optional: false,
        })
        .collect()
}

fn output_specs_from_paths(paths: &[String]) -> Vec<CanonicalBuildPlanTaskOutputSpec> {
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| CanonicalBuildPlanTaskOutputSpec {
            name: format!("output{index}"),
            kind: "path".to_string(),
            path: path.clone(),
        })
        .collect()
}

fn qualify_task_path(project_path: &str, task_ref: &str) -> String {
    let trimmed = task_ref.trim();
    if trimmed.starts_with(':') {
        return trimmed.to_string();
    }
    if project_path == ":" || project_path.is_empty() {
        format!(":{trimmed}")
    } else {
        format!("{project_path}:{trimmed}")
    }
}

fn implementation_id_for_task_type(task_type: &str) -> String {
    match task_type {
        "JavaCompile" => "org.gradle.api.tasks.compile.JavaCompile",
        "Test" => "org.gradle.api.tasks.testing.Test",
        "Copy" => "org.gradle.api.tasks.Copy",
        "Sync" => "org.gradle.api.tasks.Sync",
        "Delete" => "org.gradle.api.tasks.Delete",
        "Jar" => "org.gradle.jvm.tasks.Jar",
        "Zip" => "org.gradle.api.tasks.bundling.Zip",
        "Tar" => "org.gradle.api.tasks.bundling.Tar",
        other if other.contains('.') => other,
        other => other,
    }
    .to_string()
}

fn worker_isolation_for_task_type(task_type: Option<&str>) -> &'static str {
    match task_type.unwrap_or_default() {
        "JavaCompile" | "GroovyCompile" | "ScalaCompile" | "KotlinCompile" | "Test" | "Exec"
        | "JavaExec" | "Javadoc" | "Groovydoc" | "Scaladoc" => "process",
        "Copy" | "Sync" | "Delete" | "Jar" | "War" | "Ear" | "Zip" | "Tar" => "in-process",
        _ => "compat-jvm",
    }
}

fn action_kind_for_task_type(task_type: Option<&str>) -> &'static str {
    match task_type.unwrap_or_default() {
        "JavaCompile" | "GroovyCompile" | "ScalaCompile" | "KotlinCompile" => "compile",
        "Test" => "test",
        "Copy" | "Sync" => "file-transform",
        "Delete" => "delete",
        "Jar" | "War" | "Ear" | "Zip" | "Tar" => "archive",
        "Exec" | "JavaExec" => "external-process",
        "" => "default-task",
        _ => "jvm-task",
    }
}

fn collect_shadow_toolchains(
    parsed_scripts: &[ParsedProjectBuildScript],
    env: Option<&GetBuildEnvironmentResponse>,
) -> Vec<CanonicalBuildPlanToolchainRequest> {
    let mut versions = BTreeSet::<String>::new();
    for script in parsed_scripts {
        if let Some(version) = script.parsed.source_compatibility.as_deref() {
            versions.insert(normalize_java_version(version));
        }
        if let Some(version) = script.parsed.target_compatibility.as_deref() {
            versions.insert(normalize_java_version(version));
        }
    }

    if versions.is_empty() {
        if let Some(env) = env {
            if !env.java_version.is_empty() {
                versions.insert(normalize_java_version(&env.java_version));
            }
        }
    }

    versions
        .into_iter()
        .filter(|version| !version.is_empty())
        .map(|version| CanonicalBuildPlanToolchainRequest {
            language: "java".to_string(),
            version,
            vendor: if parsed_scripts.is_empty() {
                "jvm-host"
            } else {
                "declared-build-script"
            }
            .to_string(),
            implementation: "jvm".to_string(),
        })
        .collect()
}

fn repository_count(script: &ParsedProjectBuildScript) -> usize {
    let plugin_repos = script
        .parsed
        .plugin_management
        .as_ref()
        .map(|pm| pm.repositories.len())
        .unwrap_or(0);
    let dependency_resolution_repos = script
        .parsed
        .dependency_resolution_management
        .as_ref()
        .map(|drm| drm.repositories.len())
        .unwrap_or(0);
    script.parsed.repositories.len() + plugin_repos + dependency_resolution_repos
}

fn infer_project_dir(build_file: &str) -> String {
    if build_file.is_empty() {
        return String::new();
    }
    let path = Path::new(build_file);
    path.parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn normalize_java_version(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("JavaVersion.VERSION_") {
        return rest.to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("VERSION_") {
        return rest.to_string();
    }
    if let Some(captured) = trimmed
        .split(|c: char| !c.is_ascii_digit())
        .find(|segment| !segment.is_empty())
    {
        return captured.to_string();
    }
    trimmed.to_string()
}

fn sanitize_key(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn keyed_artifact_filename(build_id: &str) -> String {
    format!(
        "{}-{}.json",
        sanitize_key(build_id),
        stable_short_hash(build_id)
    )
}

fn stable_short_hash(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let digest = hasher.finalize();
    super::cache::hex::encode(&digest[..8])
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn diff_expected_vs_artifact(
    expected: &CanonicalBuildPlan,
    artifact: &BuildPlanShadowArtifact,
) -> BuildPlanShadowDiffReport {
    let expected = expected.clone().normalized();
    let actual = artifact.plan.clone().normalized();
    let mut mismatches = Vec::new();

    if expected.build_id != actual.build_id {
        mismatches.push(format!(
            "build_id mismatch: expected '{}' got '{}'",
            expected.build_id, actual.build_id
        ));
    }
    if expected.schema_version != actual.schema_version {
        mismatches.push(format!(
            "schema_version mismatch: expected {} got {}",
            expected.schema_version, actual.schema_version
        ));
    }
    if expected.projects != actual.projects {
        // Field-level diff: identify specific project differences
        let expected_paths: std::collections::HashSet<&str> =
            expected.projects.iter().map(|p| p.path.as_str()).collect();
        let actual_paths: std::collections::HashSet<&str> =
            actual.projects.iter().map(|p| p.path.as_str()).collect();

        let only_expected: Vec<&str> = expected_paths.difference(&actual_paths).copied().collect();
        let only_actual: Vec<&str> = actual_paths.difference(&expected_paths).copied().collect();

        if !only_expected.is_empty() {
            mismatches.push(format!(
                "projects only in expected: [{}]",
                only_expected.join(", ")
            ));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!(
                "projects only in actual: [{}]",
                only_actual.join(", ")
            ));
        }

        // Check for field-level differences in common projects
        for p in &expected.projects {
            if let Some(a) = actual.projects.iter().find(|a| a.path == p.path) {
                if p.name != a.name {
                    mismatches.push(format!(
                        "project '{}' name mismatch: expected '{}' got '{}'",
                        p.path, p.name, a.name
                    ));
                }
                if p.project_dir != a.project_dir {
                    mismatches.push(format!(
                        "project '{}' project_dir mismatch: expected '{}' got '{}'",
                        p.path, p.project_dir, a.project_dir
                    ));
                }
            }
        }

        if only_expected.is_empty()
            && only_actual.is_empty()
            && mismatches.iter().all(|m| !m.starts_with("project"))
        {
            mismatches.push(format!(
                "projects mismatch: expected {} entries got {}",
                expected.projects.len(),
                actual.projects.len()
            ));
        }
    }
    if expected.tasks != actual.tasks {
        let expected_tasks: std::collections::HashSet<&str> =
            expected.tasks.iter().map(|t| t.path.as_str()).collect();
        let actual_tasks: std::collections::HashSet<&str> =
            actual.tasks.iter().map(|t| t.path.as_str()).collect();

        let only_expected: Vec<&str> = expected_tasks.difference(&actual_tasks).copied().collect();
        let only_actual: Vec<&str> = actual_tasks.difference(&expected_tasks).copied().collect();

        if !only_expected.is_empty() {
            mismatches.push(format!(
                "tasks only in expected: [{}]",
                only_expected.join(", ")
            ));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!(
                "tasks only in actual: [{}]",
                only_actual.join(", ")
            ));
        }
        if only_expected.is_empty() && only_actual.is_empty() {
            mismatches.push(format!(
                "tasks field-level mismatch: expected {} entries got {}",
                expected.tasks.len(),
                actual.tasks.len()
            ));
        }
    }
    if expected.dependencies != actual.dependencies {
        let expected_deps: std::collections::HashSet<String> = expected
            .dependencies
            .iter()
            .map(|d| format!("{}|{}|{}", d.project_path, d.configuration, d.notation))
            .collect();
        let actual_deps: std::collections::HashSet<String> = actual
            .dependencies
            .iter()
            .map(|d| format!("{}|{}|{}", d.project_path, d.configuration, d.notation))
            .collect();

        let only_expected: Vec<String> = expected_deps
            .iter()
            .filter(|s| !actual_deps.contains(*s))
            .cloned()
            .collect();
        let only_actual: Vec<String> = actual_deps
            .iter()
            .filter(|s| !expected_deps.contains(*s))
            .cloned()
            .collect();

        if !only_expected.is_empty() {
            mismatches.push(format!(
                "dependencies only in expected: [{}]",
                only_expected.join(", ")
            ));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!(
                "dependencies only in actual: [{}]",
                only_actual.join(", ")
            ));
        }
        if only_expected.is_empty() && only_actual.is_empty() {
            mismatches.push(format!(
                "dependencies mismatch: expected {} entries got {}",
                expected.dependencies.len(),
                actual.dependencies.len()
            ));
        }
    }
    if expected.toolchains != actual.toolchains {
        let expected_tc: std::collections::HashSet<&str> = expected
            .toolchains
            .iter()
            .map(|t| t.language.as_str())
            .collect();
        let actual_tc: std::collections::HashSet<&str> = actual
            .toolchains
            .iter()
            .map(|t| t.language.as_str())
            .collect();

        let only_expected: Vec<&str> = expected_tc.difference(&actual_tc).copied().collect();
        let only_actual: Vec<&str> = actual_tc.difference(&expected_tc).copied().collect();

        if !only_expected.is_empty() {
            mismatches.push(format!(
                "toolchains only in expected: [{}]",
                only_expected.join(", ")
            ));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!(
                "toolchains only in actual: [{}]",
                only_actual.join(", ")
            ));
        }
        if only_expected.is_empty() && only_actual.is_empty() {
            mismatches.push(format!(
                "toolchains field-level mismatch: expected {} entries got {}",
                expected.toolchains.len(),
                actual.toolchains.len()
            ));
        }
    }
    if expected.metadata != actual.metadata {
        let only_expected: Vec<String> = expected
            .metadata
            .keys()
            .filter(|k| !actual.metadata.contains_key(*k))
            .map(|k| k.to_string())
            .collect();
        let only_actual: Vec<String> = actual
            .metadata
            .keys()
            .filter(|k| !expected.metadata.contains_key(*k))
            .map(|k| k.to_string())
            .collect();
        let value_diffs: Vec<String> = expected
            .metadata
            .iter()
            .filter(|(k, v)| actual.metadata.get(*k) != Some(v))
            .map(|(k, v)| {
                format!(
                    "{}: expected='{}' got='{}'",
                    k,
                    v,
                    actual.metadata.get(k).unwrap_or(&String::new())
                )
            })
            .collect();

        if !only_expected.is_empty() {
            mismatches.push(format!(
                "metadata keys only in expected: [{}]",
                only_expected.join(", ")
            ));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!(
                "metadata keys only in actual: [{}]",
                only_actual.join(", ")
            ));
        }
        for d in &value_diffs {
            mismatches.push(format!("metadata {}", d));
        }
        if only_expected.is_empty() && only_actual.is_empty() && value_diffs.is_empty() {
            mismatches.push("metadata mismatch".to_string());
        }
    }

    match fingerprint_normalized(&actual) {
        Ok(fp) if fp != artifact.fingerprint_sha256 => mismatches.push(format!(
            "fingerprint mismatch: expected '{}' got '{}'",
            fp, artifact.fingerprint_sha256
        )),
        Err(err) => mismatches.push(format!("fingerprint computation failed: {}", err)),
        _ => {}
    }

    BuildPlanShadowDiffReport {
        build_id: expected.build_id,
        mismatches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::build_plan_ir::fingerprint_sha256_hex; // used in test assertions

    #[test]
    fn canonical_plan_from_jvm_extracts_projects_and_toolchain() {
        let model = GetBuildModelResponse {
            projects: vec![crate::proto::ProjectModel {
                path: ":app".to_string(),
                name: "app".to_string(),
                build_file: "/repo/app/build.gradle.kts".to_string(),
                subprojects: vec![],
            }],
        };
        let env = GetBuildEnvironmentResponse {
            java_version: "21.0.4".to_string(),
            java_home: String::new(),
            gradle_version: "9.0.0".to_string(),
            os_name: String::new(),
            os_arch: String::new(),
            available_processors: 0,
            max_memory_bytes: 0,
            system_properties: Default::default(),
        };

        let plan = canonical_plan_from_jvm("build-abc", &model, Some(&env), Vec::new(), &[], None);
        assert_eq!(plan.schema_version, BUILD_PLAN_SCHEMA_VERSION);
        assert_eq!(plan.build_id, "build-abc");
        assert_eq!(plan.projects.len(), 1);
        assert_eq!(plan.projects[0].project_dir, "/repo/app");
        assert_eq!(plan.toolchains.len(), 1);
        assert_eq!(plan.toolchains[0].version, "21");
        assert_eq!(
            plan.metadata.get("dependencyCount").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn canonical_plan_from_jvm_uses_scripts_for_metadata_not_task_fallback() {
        let repo = tempfile::tempdir().unwrap();
        let app_dir = repo.path().join("app");
        std::fs::create_dir_all(&app_dir).unwrap();

        let root_build = repo.path().join("build.gradle.kts");
        std::fs::write(
            &root_build,
            r#"
                plugins {
                    java
                }

                repositories {
                    mavenCentral()
                }

                group = "org.example"
                version = "1.0.0"

                java {
                    sourceCompatibility = JavaVersion.VERSION_17
                    targetCompatibility = JavaVersion.VERSION_17
                }

                tasks.register("lint") {
                    dependsOn("check")
                    shouldRunAfter("test")
                }
            "#,
        )
        .unwrap();

        let app_build = app_dir.join("build.gradle.kts");
        std::fs::write(
            &app_build,
            r#"
                tasks.register<JavaCompile>("compileJava") {
                    dependsOn("generateSources")
                    mustRunAfter("processResources")
                    finalizedBy("check")
                    destinationDirectory = layout.buildDirectory.dir("classes/java/main")
                }

                tasks.register<Test>("integrationTest") {
                    dependsOn("test")
                    shouldRunAfter("compileJava")
                    outputs.dir("build/test-results/integrationTest")
                    enabled = false
                }
            "#,
        )
        .unwrap();

        let model = GetBuildModelResponse {
            projects: vec![
                crate::proto::ProjectModel {
                    path: ":".to_string(),
                    name: "root".to_string(),
                    build_file: root_build.to_string_lossy().into_owned(),
                    subprojects: vec![":app".to_string()],
                },
                crate::proto::ProjectModel {
                    path: ":app".to_string(),
                    name: "app".to_string(),
                    build_file: app_build.to_string_lossy().into_owned(),
                    subprojects: vec![],
                },
            ],
        };

        let parsed_scripts = collect_parsed_build_scripts(&model);
        let plan = canonical_plan_from_jvm(
            "build-script-shadow",
            &model,
            None,
            Vec::new(),
            &parsed_scripts,
            None,
        );

        assert_eq!(
            plan.metadata
                .get("parsedBuildScriptCount")
                .map(String::as_str),
            Some("2")
        );
        assert_eq!(
            plan.metadata.get("taskSource").map(String::as_str),
            Some("parsed-build-scripts")
        );
        assert_eq!(
            plan.metadata.get("taskCount").map(String::as_str),
            Some("3")
        );
        assert_eq!(
            plan.metadata
                .get("taskDependencyEdgeCount")
                .map(String::as_str),
            Some("3")
        );
        assert_eq!(
            plan.metadata
                .get("taskSoftDependencyEdgeCount")
                .map(String::as_str),
            Some("2")
        );
        assert_eq!(
            plan.metadata
                .get("taskMustRunAfterEdgeCount")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            plan.metadata
                .get("taskFinalizerEdgeCount")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            plan.metadata.get("disabledTaskCount").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            plan.metadata
                .get("dependencyConfigurationCount")
                .map(String::as_str),
            Some("0")
        );
        assert_eq!(
            plan.metadata.get("typedTaskCount").map(String::as_str),
            Some("2")
        );
        assert_eq!(
            plan.metadata
                .get("declaredTaskOutputCount")
                .map(String::as_str),
            Some("2")
        );
        assert_eq!(plan.toolchains.len(), 1);
        assert_eq!(plan.toolchains[0].version, "17");
        let compile_java = plan
            .tasks
            .iter()
            .find(|task| task.path == ":app:compileJava")
            .expect("expected script-declared compileJava task");
        assert_eq!(
            compile_java.implementation_id,
            "org.gradle.api.tasks.compile.JavaCompile"
        );
        assert_eq!(compile_java.action_kind, "compile");
        assert_eq!(compile_java.worker_isolation, "process");
        assert_eq!(compile_java.depends_on, vec![":app:generateSources"]);
        assert_eq!(compile_java.must_run_after, vec![":app:processResources"]);
        assert_eq!(compile_java.finalized_by, vec![":app:check"]);
        assert_eq!(compile_java.outputs, vec!["classes/java/main"]);
        assert!(compile_java
            .input_specs
            .iter()
            .any(|input| input.name == "taskType" && input.value == "JavaCompile"));
        assert_eq!(compile_java.output_specs.len(), 1);
        assert_eq!(compile_java.diagnostics.len(), 1);
    }

    #[test]
    fn persist_and_load_shadow_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let store = BuildPlanShadowStore::new(temp.path().to_path_buf());
        let plan = CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "build:1".to_string(),
            projects: vec![CanonicalBuildPlanProject {
                path: ":".to_string(),
                name: "root".to_string(),
                project_dir: "/repo".to_string(),
            }],
            tasks: Vec::new(),
            dependencies: Vec::new(),
            toolchains: Vec::new(),
            metadata: std::collections::BTreeMap::new(),
        };

        let path = store.persist_plan(&plan, "test").unwrap();
        assert!(path.exists());

        let loaded = store.load_plan("build:1").unwrap().unwrap();
        assert_eq!(loaded.plan.build_id, "build:1");
        assert_eq!(loaded.source, "test");
        assert!(!loaded.fingerprint_sha256.is_empty());
    }

    #[test]
    fn load_plan_quarantines_corrupt_shadow_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let store = BuildPlanShadowStore::new(temp.path().to_path_buf());
        let mut plan = CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "build:corrupt".to_string(),
            projects: vec![CanonicalBuildPlanProject {
                path: ":".to_string(),
                name: "root".to_string(),
                project_dir: "/repo".to_string(),
            }],
            tasks: Vec::new(),
            dependencies: Vec::new(),
            toolchains: Vec::new(),
            metadata: std::collections::BTreeMap::new(),
        };

        let path = store.persist_plan(&plan, "test").unwrap();
        let mut artifact: BuildPlanShadowArtifact =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        plan.projects[0].name = "mutated".to_string();
        artifact.plan = plan;
        std::fs::write(&path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

        let error = store.load_plan("build:corrupt").unwrap_err();

        assert!(
            error.to_string().contains("fingerprint mismatch"),
            "unexpected error: {}",
            error
        );
        assert!(!path.exists(), "corrupt artifact should be moved away");
        let quarantine_dir = store.root().join("quarantine");
        assert!(quarantine_dir.exists());
        assert_eq!(std::fs::read_dir(quarantine_dir).unwrap().count(), 2);
    }

    #[test]
    fn load_plan_quarantines_wrong_build_id_shadow_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let store = BuildPlanShadowStore::new(temp.path().to_path_buf());
        let plan = CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "build:a".to_string(),
            projects: Vec::new(),
            tasks: Vec::new(),
            dependencies: Vec::new(),
            toolchains: Vec::new(),
            metadata: std::collections::BTreeMap::new(),
        };
        let path = store.persist_plan(&plan, "test").unwrap();
        let wrong_path = store.artifact_path_for_build_id("build:b");
        std::fs::rename(&path, &wrong_path).unwrap();

        let error = store.load_plan("build:b").unwrap_err();

        assert!(error.to_string().contains("build id mismatch"));
        assert!(!wrong_path.exists());
    }

    #[test]
    fn load_plan_quarantines_schema_version_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let store = BuildPlanShadowStore::new(temp.path().to_path_buf());
        let plan = CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "build:schema".to_string(),
            projects: Vec::new(),
            tasks: Vec::new(),
            dependencies: Vec::new(),
            toolchains: Vec::new(),
            metadata: std::collections::BTreeMap::new(),
        };
        let path = store.persist_plan(&plan, "test").unwrap();
        let mut artifact: BuildPlanShadowArtifact =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        artifact.plan.schema_version = BUILD_PLAN_SCHEMA_VERSION + 1;
        std::fs::write(&path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

        let error = store.load_plan("build:schema").unwrap_err();

        assert!(
            error.to_string().contains("schema validation failed"),
            "unexpected error: {}",
            error
        );
        assert!(!path.exists(), "schema-mismatched artifact should be quarantined");
    }

    #[test]
    fn dependency_notation_preserves_classifier_and_extension() {
        assert_eq!(
            dependency_notation("org.example", "demo", "1.2.3", "", "jar"),
            "org.example:demo:1.2.3"
        );
        assert_eq!(
            dependency_notation("org.example", "demo", "1.2.3", "sources", "jar"),
            "org.example:demo:1.2.3:sources"
        );
        assert_eq!(
            dependency_notation("org.example", "demo", "1.2.3", "", "aar"),
            "org.example:demo:1.2.3@aar"
        );
        assert_eq!(
            dependency_notation("org.example", "demo", "1.2.3", "debug", "aar"),
            "org.example:demo:1.2.3:debug@aar"
        );
    }

    #[test]
    fn diff_report_detects_modified_artifact() {
        let expected = CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "build-x".to_string(),
            projects: vec![CanonicalBuildPlanProject {
                path: ":".to_string(),
                name: "root".to_string(),
                project_dir: "/repo".to_string(),
            }],
            tasks: Vec::new(),
            dependencies: Vec::new(),
            toolchains: Vec::new(),
            metadata: std::collections::BTreeMap::new(),
        };

        let mut artifact = BuildPlanShadowArtifact {
            plan: expected.clone(),
            fingerprint_sha256: fingerprint_sha256_hex(&expected).unwrap(),
            stored_at_ms: 0,
            source: "test".to_string(),
        };
        artifact.plan.projects[0].name = "mutated".to_string();

        let report = diff_expected_vs_artifact(&expected, &artifact);
        assert!(!report.is_match());
        assert!(!report.mismatches.is_empty());
    }

    #[test]
    fn artifact_filename_is_collision_safe_for_similar_sanitized_keys() {
        let temp = tempfile::tempdir().unwrap();
        let store = BuildPlanShadowStore::new(temp.path().to_path_buf());
        let a = store.artifact_path_for_build_id("build/a");
        let b = store.artifact_path_for_build_id("build:a");
        assert_ne!(a, b);
    }
}
