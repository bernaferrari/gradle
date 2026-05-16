//! JvmHostService — Rust implementation of the JVM compatibility host.
//!
//! The JvmHostService exposes build environment, configuration resolution,
//! build model, and task execution APIs that originally lived in the JVM
//! Gradle daemon. This module progressively ports those RPCs to pure Rust.
//!
//! Currently implemented: GetBuildEnvironment, GetBuildModel, GetBuildPlan,
//! ResolveConfiguration, ExecuteTask. EvaluateScript is explicit fail-closed
//! because Groovy/Kotlin DSL execution remains JVM-owned.

use std::collections::HashMap;
use std::env;
use std::path::{Path, MAIN_SEPARATOR};
use std::process::Command;
use std::sync::Arc;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::jvm_host_service_server::JvmHostService;
use crate::proto::{
    EvaluateScriptRequest, EvaluateScriptResponse, ExecuteTaskRequest, ExecuteTaskResponse,
    GetBuildEnvironmentRequest, GetBuildEnvironmentResponse, GetBuildModelRequest,
    GetBuildModelResponse, GetBuildPlanRequest, GetBuildPlanResponse, ProjectModel,
    RepositoryDescriptor, ResolveConfigRequest, ResolveConfigResponse, ResolvedArtifact,
};

use crate::server::build_init::BuildInitServiceImpl;
use crate::server::build_plan_ir::{to_proto, CanonicalBuildPlanRepository};
use crate::server::build_plan_shadow::BuildPlanShadowStore;
use crate::server::platform::{CurrentPlatform, PlatformOps};
use crate::server::scopes::BuildId;
use crate::server::task_executor::{TaskExecutorRegistry, TaskInput};

/// JvmHostService implementation.
#[derive(Clone)]
pub struct JvmHostServiceImpl {
    build_registry: Arc<DashMap<BuildId, String>>,
    build_plan_shadow_store: Option<Arc<BuildPlanShadowStore>>,
    task_executors: Arc<TaskExecutorRegistry>,
}

impl JvmHostServiceImpl {
    /// Create a new JvmHostService with access to the shared build registry.
    pub fn new(build_registry: Arc<DashMap<BuildId, String>>) -> Self {
        Self {
            build_registry,
            build_plan_shadow_store: None,
            task_executors: Arc::new(TaskExecutorRegistry::new()),
        }
    }

    /// Set the build-plan shadow store used by GetBuildPlan.
    pub fn with_build_plan_shadow_store(
        mut self,
        build_plan_shadow_store: Arc<BuildPlanShadowStore>,
    ) -> Self {
        self.build_plan_shadow_store = Some(build_plan_shadow_store);
        self
    }

    /// Set the task executor registry used by ExecuteTask.
    pub fn with_task_executors(mut self, task_executors: Arc<TaskExecutorRegistry>) -> Self {
        self.task_executors = task_executors;
        self
    }
}

impl Default for JvmHostServiceImpl {
    fn default() -> Self {
        // In production, JvmHostServiceImpl is always constructed with a real
        // build_registry from BootstrapService. This default exists only for
        // tests that don't need registry lookups.
        Self {
            build_registry: Arc::new(DashMap::new()),
            build_plan_shadow_store: None,
            task_executors: Arc::new(TaskExecutorRegistry::new()),
        }
    }
}

#[tonic::async_trait]
impl JvmHostService for JvmHostServiceImpl {
    async fn get_build_environment(
        &self,
        _request: Request<GetBuildEnvironmentRequest>,
    ) -> Result<Response<GetBuildEnvironmentResponse>, Status> {
        let (java_home, java_version) = match Self::detect_java() {
            Some((home, version)) => (home, version),
            None => (String::new(), String::new()),
        };

        // Gradle version: try read from wrapper properties or env, else default
        let gradle_version = Self::detect_gradle_version();

        // OS and architecture
        let os_name = env::consts::OS.to_string();
        let os_arch = env::consts::ARCH.to_string();

        // CPU count
        let available_processors = num_cpus::get() as i32;

        // Total system memory
        let max_memory_bytes = CurrentPlatform::total_memory_bytes() as i64;

        // Basic system properties
        let mut system_properties = HashMap::new();
        system_properties.insert(
            "java.vm.name".to_string(),
            "OpenJDK 64-Bit Server VM".to_string(),
        );
        system_properties.insert(
            "user.timezone".to_string(),
            env::var("TZ").unwrap_or_else(|_| "UTC".to_string()),
        );

        Ok(Response::new(GetBuildEnvironmentResponse {
            java_version,
            java_home,
            gradle_version,
            os_name,
            os_arch,
            available_processors,
            max_memory_bytes,
            system_properties,
        }))
    }

    async fn get_build_model(
        &self,
        request: Request<GetBuildModelRequest>,
    ) -> Result<Response<GetBuildModelResponse>, Status> {
        let req = request.into_inner();

        // Look up the project directory for this build_id
        let build_id = BuildId::from(req.build_id.clone());
        let project_dir = match self.build_registry.get(&build_id) {
            Some(entry) => entry.value().clone(),
            None => {
                return Err(Status::not_found(format!(
                    "Build '{}' not found — has InitBuild been called?",
                    req.build_id
                )))
            }
        };

        // Parse settings.gradle(.kts) to discover project structure.
        // We delegate to BuildInitServiceImpl's parser (shared logic).
        let parsed = BuildInitServiceImpl::parse_settings_file(&project_dir, "");

        let mut projects = Vec::new();

        // Root project model (path = `:`) with name from settings or directory name.
        let root_name = parsed
            .root_project_name
            .clone()
            .or_else(|| {
                std::path::Path::new(&project_dir)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "root".to_string());

        let root_build_file = build_file_for_project_dir(Path::new(&project_dir));

        let project_paths = project_paths_with_ancestors(&parsed.included_projects);

        // For the root, collect only direct children. Nested projects are linked
        // from their immediate parent below.
        let subproject_paths = direct_child_project_paths(":", &project_paths);

        projects.push(ProjectModel {
            path: ":".to_string(),
            name: root_name,
            build_file: root_build_file,
            subprojects: subproject_paths,
        });

        // Now create a ProjectModel for each included subproject. This mirrors
        // Gradle's Settings.include(), which creates missing ancestor project
        // descriptors while walking paths like `:lib:core`.
        for subproject_path in &project_paths {
            // Convert Gradle project path `:feature:login` into filesystem relative path `feature/login`
            let rel_dir = subproject_path
                .trim_start_matches(':')
                .replace(':', std::path::MAIN_SEPARATOR_STR);

            let subproject_dir = Path::new(&project_dir).join(&rel_dir);
            let subproject_name = subproject_path
                .split(':')
                .last()
                .unwrap_or(subproject_path)
                .to_string();

            let subproject_build_file = build_file_for_project_dir(&subproject_dir);
            let subprojects = direct_child_project_paths(subproject_path, &project_paths);

            projects.push(ProjectModel {
                path: subproject_path.clone(),
                name: subproject_name,
                build_file: subproject_build_file,
                subprojects,
            });
        }

        Ok(Response::new(GetBuildModelResponse { projects }))
    }

    async fn resolve_configuration(
        &self,
        request: Request<ResolveConfigRequest>,
    ) -> Result<Response<ResolveConfigResponse>, Status> {
        let req = request.into_inner();
        if req.build_id.trim().is_empty() {
            return Err(Status::invalid_argument("build_id must not be empty"));
        }
        if req.project_path.trim().is_empty() {
            return Err(Status::invalid_argument("project_path must not be empty"));
        }
        if req.configuration_name.trim().is_empty() {
            return Err(Status::invalid_argument(
                "configuration_name must not be empty",
            ));
        }

        let Some(store) = self.build_plan_shadow_store.as_ref() else {
            return Ok(Response::new(ResolveConfigResponse {
                success: false,
                artifacts: Vec::new(),
                error_message: "BuildPlanShadowStore is not configured".to_string(),
            }));
        };

        let artifact = match store.load_plan(&req.build_id) {
            Ok(Some(artifact)) => artifact,
            Ok(None) => {
                return Ok(Response::new(ResolveConfigResponse {
                    success: false,
                    artifacts: Vec::new(),
                    error_message: format!(
                        "No Rust build-plan shadow found for build '{}'",
                        req.build_id
                    ),
                }))
            }
            Err(error) => {
                return Err(Status::failed_precondition(format!(
                    "Failed to load Rust build-plan shadow for build '{}': {}",
                    req.build_id, error
                )))
            }
        };

        let matching = artifact
            .plan
            .dependencies
            .into_iter()
            .filter(|dependency| dependency.project_path == req.project_path)
            .filter(|dependency| dependency.configuration == req.configuration_name)
            .collect::<Vec<_>>();

        if matching.is_empty() {
            return Ok(Response::new(ResolveConfigResponse {
                success: false,
                artifacts: Vec::new(),
                error_message: format!(
                    "No cached Rust dependency graph for build '{}', project '{}', configuration '{}'",
                    req.build_id, req.project_path, req.configuration_name
                ),
            }));
        }

        let unsupported = matching
            .iter()
            .flat_map(|dependency| dependency.unsupported_features.iter())
            .filter(|feature| !feature.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if !unsupported.is_empty() {
            return Ok(Response::new(ResolveConfigResponse {
                success: false,
                artifacts: Vec::new(),
                error_message: format!(
                    "Cached Rust dependency graph contains unsupported features for build '{}', project '{}', configuration '{}': {}",
                    req.build_id,
                    req.project_path,
                    req.configuration_name,
                    unsupported.join(", ")
                ),
            }));
        }

        let mut artifacts = Vec::with_capacity(matching.len());
        for dependency in matching {
            let coordinate = match parse_resolved_artifact_notation(&dependency.notation) {
                Some(coordinate) => coordinate,
                None => {
                    return Ok(Response::new(ResolveConfigResponse {
                        success: false,
                        artifacts: Vec::new(),
                        error_message: format!(
                            "Cached Rust dependency graph contains unsupported notation '{}' for build '{}', project '{}', configuration '{}'",
                            dependency.notation,
                            req.build_id,
                            req.project_path,
                            req.configuration_name
                        ),
                    }))
                }
            };
            artifacts.push(ResolvedArtifact {
                group: coordinate.group,
                name: coordinate.name,
                version: coordinate.version,
                configuration: dependency.configuration,
                classifier: coordinate.classifier,
                extension: coordinate.extension,
                kind: if dependency.kind.trim().is_empty() {
                    "dependency".to_string()
                } else {
                    dependency.kind
                },
                repositories: dependency
                    .repositories
                    .into_iter()
                    .map(repository_to_proto)
                    .collect(),
                unsupported_features: Vec::new(),
            });
        }

        Ok(Response::new(ResolveConfigResponse {
            success: true,
            artifacts,
            error_message: String::new(),
        }))
    }

    async fn get_build_plan(
        &self,
        request: Request<GetBuildPlanRequest>,
    ) -> Result<Response<GetBuildPlanResponse>, Status> {
        let req = request.into_inner();
        if req.build_id.trim().is_empty() {
            return Err(Status::invalid_argument("build_id must not be empty"));
        }

        let Some(store) = self.build_plan_shadow_store.as_ref() else {
            return Ok(Response::new(GetBuildPlanResponse {
                success: false,
                error_message: "BuildPlanShadowStore is not configured".to_string(),
                plan: None,
                source: "rust-shadow-store-unconfigured".to_string(),
            }));
        };

        match store.load_plan(&req.build_id) {
            Ok(Some(artifact)) => Ok(Response::new(GetBuildPlanResponse {
                success: true,
                error_message: String::new(),
                plan: Some(to_proto(&artifact.plan)),
                source: artifact.source,
            })),
            Ok(None) => Ok(Response::new(GetBuildPlanResponse {
                success: false,
                error_message: format!(
                    "No Rust build-plan shadow found for build '{}'",
                    req.build_id
                ),
                plan: None,
                source: "rust-shadow-store-miss".to_string(),
            })),
            Err(error) => Err(Status::failed_precondition(format!(
                "Failed to load Rust build-plan shadow for build '{}': {}",
                req.build_id, error
            ))),
        }
    }

    async fn execute_task(
        &self,
        request: Request<ExecuteTaskRequest>,
    ) -> Result<Response<ExecuteTaskResponse>, Status> {
        let req = request.into_inner();
        if req.build_id.trim().is_empty() {
            return Err(Status::invalid_argument("build_id must not be empty"));
        }
        if req.task_path.trim().is_empty() {
            return Err(Status::invalid_argument("task_path must not be empty"));
        }
        if req.task_type.trim().is_empty() {
            return Err(Status::invalid_argument("task_type must not be empty"));
        }
        if !self.task_executors.has_executor(&req.task_type) {
            return Ok(Response::new(ExecuteTaskResponse {
                success: false,
                outcome: "UNSUPPORTED".to_string(),
                error_message: format!(
                    "Task type '{}' is not supported by Rust ExecuteTask",
                    req.task_type
                ),
                duration_ms: 0,
                execution_mode: "rust-native-unsupported".to_string(),
            }));
        }

        let input = match task_input_from_parameters_json(&req.task_type, &req.parameters_json) {
            Ok(input) => input,
            Err(error_message) => {
                return Ok(Response::new(ExecuteTaskResponse {
                    success: false,
                    outcome: "FAILED".to_string(),
                    error_message,
                    duration_ms: 0,
                    execution_mode: "rust-native-contract-error".to_string(),
                }))
            }
        };

        let execute = self.task_executors.execute(&input);
        let result = if req.timeout_ms > 0 {
            match tokio::time::timeout(
                std::time::Duration::from_millis(req.timeout_ms as u64),
                execute,
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    return Ok(Response::new(ExecuteTaskResponse {
                        success: false,
                        outcome: "TIMED_OUT".to_string(),
                        error_message: format!(
                            "Rust ExecuteTask timed out after {} ms for task '{}'",
                            req.timeout_ms, req.task_path
                        ),
                        duration_ms: req.timeout_ms,
                        execution_mode: "rust-native-timeout".to_string(),
                    }))
                }
            }
        } else {
            execute.await
        };

        Ok(Response::new(ExecuteTaskResponse {
            success: result.success,
            outcome: if result.success {
                "EXECUTED".to_string()
            } else {
                "FAILED".to_string()
            },
            error_message: result.error_message,
            duration_ms: result.duration_ms as i64,
            execution_mode: "rust-native".to_string(),
        }))
    }

    async fn evaluate_script(
        &self,
        request: Request<EvaluateScriptRequest>,
    ) -> Result<Response<EvaluateScriptResponse>, Status> {
        let req = request.into_inner();
        let script_type = if req.script_type.trim().is_empty() {
            "unknown"
        } else {
            req.script_type.trim()
        };
        let script_path = if req.script_path.trim().is_empty() {
            "<inline>"
        } else {
            req.script_path.trim()
        };

        Ok(Response::new(EvaluateScriptResponse {
            success: false,
            error_message: format!(
                "Script evaluation for '{}' ({}) is JVM-owned; use the JVM compatibility host",
                script_path, script_type
            ),
            applied_plugins: Vec::new(),
        }))
    }
}

fn build_file_for_project_dir(project_dir: &Path) -> String {
    let kotlin = project_dir.join("build.gradle.kts");
    if kotlin.exists() {
        return kotlin.to_string_lossy().to_string();
    }
    project_dir
        .join("build.gradle")
        .to_string_lossy()
        .to_string()
}

fn direct_child_project_paths(parent: &str, projects: &[String]) -> Vec<String> {
    projects
        .iter()
        .filter(|candidate| is_direct_child_project_path(parent, candidate))
        .cloned()
        .collect()
}

fn project_paths_with_ancestors(projects: &[String]) -> Vec<String> {
    let mut expanded = Vec::new();
    for project in projects {
        let mut path = String::new();
        for segment in project.trim_matches(':').split(':') {
            if segment.is_empty() {
                continue;
            }
            path.push(':');
            path.push_str(segment);
            if !expanded.contains(&path) {
                expanded.push(path.clone());
            }
        }
    }
    expanded
}

fn is_direct_child_project_path(parent: &str, candidate: &str) -> bool {
    if parent == ":" {
        let trimmed = candidate.trim_start_matches(':');
        return !trimmed.is_empty() && !trimmed.contains(':');
    }
    let prefix = format!("{parent}:");
    let Some(remainder) = candidate.strip_prefix(&prefix) else {
        return false;
    };
    !remainder.is_empty() && !remainder.contains(':')
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedArtifactCoordinate {
    group: String,
    name: String,
    version: String,
    classifier: String,
    extension: String,
}

fn parse_resolved_artifact_notation(value: &str) -> Option<ResolvedArtifactCoordinate> {
    let (base, extension) = value
        .split_once('@')
        .map_or((value, "jar"), |(base, extension)| (base, extension.trim()));
    if extension.is_empty() {
        return None;
    }

    let mut parts = base.split(':');
    let group = parts.next()?.trim();
    let name = parts.next()?.trim();
    let version = parts.next()?.trim();
    let classifier = parts.next().map(str::trim).unwrap_or_default();
    if parts.next().is_some()
        || group.is_empty()
        || name.is_empty()
        || version.is_empty()
        || classifier.contains('@')
    {
        return None;
    }

    Some(ResolvedArtifactCoordinate {
        group: group.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        classifier: classifier.to_string(),
        extension: extension.to_string(),
    })
}

fn repository_to_proto(repository: CanonicalBuildPlanRepository) -> RepositoryDescriptor {
    RepositoryDescriptor {
        id: repository.id,
        url: repository.url,
        m2compatible: repository.m2compatible,
        allow_insecure_protocol: repository.allow_insecure_protocol,
        credentials: repository.credentials.into_iter().collect(),
        layout: repository.layout,
        ivy_pattern: repository.ivy_pattern,
        include_groups: repository.include_groups,
        exclude_groups: repository.exclude_groups,
        include_group_prefixes: repository.include_group_prefixes,
        exclude_group_prefixes: repository.exclude_group_prefixes,
        include_modules: repository.include_modules,
        exclude_modules: repository.exclude_modules,
        include_module_versions: repository.include_module_versions,
        exclude_module_versions: repository.exclude_module_versions,
    }
}

fn task_input_from_parameters_json(
    task_type: &str,
    parameters_json: &str,
) -> Result<TaskInput, String> {
    let trimmed = parameters_json.trim();
    if trimmed.is_empty() {
        return Ok(TaskInput::new(task_type));
    }

    let value = serde_json::from_str::<serde_json::Value>(trimmed)
        .map_err(|error| format!("Invalid parameters_json: {}", error))?;
    let object = value
        .as_object()
        .ok_or_else(|| "parameters_json must be a JSON object".to_string())?;

    let mut input = TaskInput::new(task_type);
    if let Some(source_files) = object.get("source_files") {
        let source_files = source_files
            .as_array()
            .ok_or_else(|| "parameters_json.source_files must be an array".to_string())?;
        input.source_files = source_files
            .iter()
            .map(|value| {
                value.as_str().map(std::path::PathBuf::from).ok_or_else(|| {
                    "parameters_json.source_files entries must be strings".to_string()
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
    }
    if let Some(target_dir) = object.get("target_dir") {
        let target_dir = target_dir
            .as_str()
            .ok_or_else(|| "parameters_json.target_dir must be a string".to_string())?;
        input.target_dir = std::path::PathBuf::from(target_dir);
    }
    if let Some(options) = object.get("options") {
        let options = options
            .as_object()
            .ok_or_else(|| "parameters_json.options must be an object".to_string())?;
        input.options = options
            .iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|value| (key.clone(), value.to_string()))
                    .ok_or_else(|| format!("parameters_json.options.{} must be a string", key))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
    }
    if let Some(output_files) = object.get("output_files") {
        let output_files = output_files
            .as_array()
            .ok_or_else(|| "parameters_json.output_files must be an array".to_string())?;
        let values = output_files
            .iter()
            .map(|value| {
                value.as_str().map(str::to_string).ok_or_else(|| {
                    "parameters_json.output_files entries must be strings".to_string()
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !values.is_empty() {
            input.options.insert(
                "output_files_json".to_string(),
                serde_json::to_string(&values).unwrap_or_default(),
            );
        }
    }

    Ok(input)
}

impl JvmHostServiceImpl {
    /// Detect the Gradle version for this daemon.
    /// Reads from wrapper properties if present, falls back to env var or default.
    fn detect_gradle_version() -> String {
        // Prefer wrapper properties in current or parent directories
        if let Ok(mut current) = env::current_dir() {
            for _ in 0..10 {
                let props = current.join("gradle/wrapper/gradle-wrapper.properties");
                if props.exists() {
                    if let Ok(content) = std::fs::read_to_string(&props) {
                        for line in content.lines() {
                            if let Some(stripped) = line.strip_prefix("distributionUrl=") {
                                // Extract version from URL, e.g. .../gradle-9.6.0-milestone-2-bin.zip
                                if let Some(pos) = stripped.find("gradle-") {
                                    let ver_part = &stripped[pos + "gradle-".len()..];
                                    if let Some(end) = ver_part.find("-bin.zip") {
                                        return ver_part[..end].to_string();
                                    }
                                }
                            }
                        }
                    }
                }
                if !current.pop() {
                    break;
                }
            }
        }
        // Fallback to environment variable
        env::var("GRADLE_VERSION").unwrap_or_else(|_| "9.6.0-milestone-2".to_string())
    }

    /// Attempt to detect Java home and version by probing the system.
    /// Returns (java_home, java_version) on success.
    fn detect_java() -> Option<(String, String)> {
        // 1. Try JAVA_HOME first
        if let Ok(java_home) = env::var("JAVA_HOME") {
            if let Some(version) = Self::probe_java_home(&java_home) {
                return Some((java_home, version));
            }
        }

        // 2. Search PATH for java executable
        let path_env = env::var("PATH").ok()?;
        for dir in path_env.split(MAIN_SEPARATOR) {
            let java_exe = if cfg!(windows) {
                Path::new(dir).join("java.exe")
            } else {
                Path::new(dir).join("java")
            };
            if java_exe.is_file() {
                // Derive JAVA_HOME from the binary location (bin/java -> parent's parent)
                if let Some(parent) = java_exe.parent() {
                    if let Some(java_home) = parent.parent() {
                        let java_home = java_home.to_string_lossy().to_string();
                        if let Some(version) = Self::probe_java_home(&java_home) {
                            return Some((java_home, version));
                        }
                    }
                }
            }
        }

        None
    }

    /// Probe a specific JAVA_HOME to retrieve the Java version string.
    fn probe_java_home(java_home: &str) -> Option<String> {
        let java_bin = if cfg!(windows) {
            format!("{}\\bin\\java.exe", java_home)
        } else {
            format!("{}/bin/java", java_home)
        };
        let output = Command::new(java_bin).arg("-version").output().ok()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Find first quoted version string in stderr.
        for line in stderr.lines() {
            // Look for a segment like version "17.0.9"
            if let Some(idx) = line.find("version \"") {
                let rest = &line[idx + "version \"".len()..];
                if let Some(end) = rest.find('"') {
                    let version = rest[..end].to_string();
                    return Some(version);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::proto::bootstrap_service_server::BootstrapService;
    use crate::proto::{CompleteBuildRequest, InitBuildRequest};
    use crate::server::bootstrap::BootstrapServiceImpl;
    use crate::server::build_plan_ir::{
        CanonicalBuildPlan, CanonicalBuildPlanDependency, CanonicalBuildPlanProject,
        CanonicalBuildPlanRepository, CanonicalBuildPlanTask, BUILD_PLAN_SCHEMA_VERSION,
    };
    use base64::Engine;

    #[tokio::test]
    async fn get_build_model_returns_registered_project_tree() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(
            root.join("settings.gradle.kts"),
            r#"
rootProject.name = "demo"
include(":app", ":lib:core")
"#,
        )
        .unwrap();
        std::fs::write(root.join("build.gradle.kts"), "").unwrap();
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(root.join("app").join("build.gradle"), "").unwrap();
        std::fs::create_dir_all(root.join("lib/core")).unwrap();
        std::fs::write(root.join("lib/core").join("build.gradle.kts"), "").unwrap();

        let registry = Arc::new(DashMap::new());
        registry.insert(
            BuildId::from("build-1".to_string()),
            root.to_string_lossy().to_string(),
        );
        let service = JvmHostServiceImpl::new(registry);

        let response = service
            .get_build_model(Request::new(GetBuildModelRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(4, response.projects.len());
        let root_model = response
            .projects
            .iter()
            .find(|project| project.path == ":")
            .unwrap();
        assert_eq!("demo", root_model.name);
        assert!(root_model.build_file.ends_with("build.gradle.kts"));
        assert_eq!(
            vec![":app".to_string(), ":lib".to_string()],
            root_model.subprojects
        );

        let app = response
            .projects
            .iter()
            .find(|project| project.path == ":app")
            .unwrap();
        assert_eq!("app", app.name);
        assert!(app.build_file.ends_with("app/build.gradle"));
        assert!(app.subprojects.is_empty());

        let lib = response
            .projects
            .iter()
            .find(|project| project.path == ":lib")
            .unwrap();
        assert_eq!("lib", lib.name);
        assert!(lib.build_file.ends_with("lib/build.gradle"));
        assert_eq!(vec![":lib:core".to_string()], lib.subprojects);

        let core = response
            .projects
            .iter()
            .find(|project| project.path == ":lib:core")
            .unwrap();
        assert_eq!("core", core.name);
        assert!(core.build_file.ends_with("lib/core/build.gradle.kts"));
    }

    #[tokio::test]
    async fn get_build_model_uses_bootstrap_registry_and_rejects_completed_build() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("settings.gradle"), "include 'app'\n").unwrap();
        std::fs::create_dir_all(root.join("app")).unwrap();

        let registry = Arc::new(DashMap::new());
        let bootstrap = BootstrapServiceImpl::new().with_build_registry(Arc::clone(&registry));
        let service = JvmHostServiceImpl::new(registry);

        bootstrap
            .init_build(Request::new(InitBuildRequest {
                build_id: "build-bootstrap".to_string(),
                project_dir: root.to_string_lossy().to_string(),
                start_time_ms: 0,
                requested_parallelism: 4,
                system_properties: Default::default(),
                requested_features: vec![],
                session_id: String::new(),
            }))
            .await
            .unwrap();

        let response = service
            .get_build_model(Request::new(GetBuildModelRequest {
                build_id: "build-bootstrap".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(response
            .projects
            .iter()
            .any(|project| project.path == ":app"));

        bootstrap
            .complete_build(Request::new(CompleteBuildRequest {
                build_id: "build-bootstrap".to_string(),
                outcome: "SUCCESS".to_string(),
                duration_ms: 10,
            }))
            .await
            .unwrap();

        let error = service
            .get_build_model(Request::new(GetBuildModelRequest {
                build_id: "build-bootstrap".to_string(),
            }))
            .await
            .unwrap_err();

        assert_eq!(tonic::Code::NotFound, error.code());
    }

    #[tokio::test]
    async fn get_build_model_rejects_unknown_build_id() {
        let service = JvmHostServiceImpl::default();

        let error = service
            .get_build_model(Request::new(GetBuildModelRequest {
                build_id: "missing".to_string(),
            }))
            .await
            .unwrap_err();

        assert_eq!(tonic::Code::NotFound, error.code());
        assert!(error.message().contains("InitBuild"));
    }

    #[tokio::test]
    async fn get_build_plan_returns_cached_rust_shadow_plan() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(BuildPlanShadowStore::new(temp.path().to_path_buf()));
        let plan = sample_canonical_plan("build-plan-1");
        store.persist_plan(&plan, "test-shadow").unwrap();
        let service =
            JvmHostServiceImpl::default().with_build_plan_shadow_store(Arc::clone(&store));

        let response = service
            .get_build_plan(Request::new(GetBuildPlanRequest {
                build_id: "build-plan-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(response.success);
        assert!(response.error_message.is_empty());
        assert_eq!("test-shadow", response.source);
        let proto_plan = response.plan.expect("plan should be returned");
        assert_eq!("build-plan-1", proto_plan.build_id);
        assert_eq!(1, proto_plan.projects.len());
        assert_eq!(1, proto_plan.tasks.len());
        assert_eq!(":compileJava", proto_plan.tasks[0].path);
    }

    #[tokio::test]
    async fn get_build_plan_reports_missing_shadow_plan_without_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(BuildPlanShadowStore::new(temp.path().to_path_buf()));
        let service = JvmHostServiceImpl::default().with_build_plan_shadow_store(store);

        let response = service
            .get_build_plan(Request::new(GetBuildPlanRequest {
                build_id: "missing-plan".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.success);
        assert!(response.plan.is_none());
        assert_eq!("rust-shadow-store-miss", response.source);
        assert!(response.error_message.contains("missing-plan"));
    }

    #[tokio::test]
    async fn get_build_plan_requires_build_id_and_configured_store() {
        let service = JvmHostServiceImpl::default();

        let error = service
            .get_build_plan(Request::new(GetBuildPlanRequest {
                build_id: " ".to_string(),
            }))
            .await
            .unwrap_err();
        assert_eq!(tonic::Code::InvalidArgument, error.code());

        let response = service
            .get_build_plan(Request::new(GetBuildPlanRequest {
                build_id: "build-without-store".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!response.success);
        assert!(response.plan.is_none());
        assert_eq!("rust-shadow-store-unconfigured", response.source);
    }

    #[tokio::test]
    async fn resolve_configuration_returns_cached_resolved_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(BuildPlanShadowStore::new(temp.path().to_path_buf()));
        let mut plan = sample_canonical_plan("build-deps-1");
        plan.dependencies = vec![
            sample_dependency(
                ":",
                "compileClasspath",
                "org.example:demo:1.2.3:sources@jar",
                "dependency",
            ),
            sample_dependency(
                ":app",
                "compileClasspath",
                "org.example:other:1.0",
                "dependency",
            ),
        ];
        store.persist_plan(&plan, "dependency-shadow").unwrap();
        let service = JvmHostServiceImpl::default().with_build_plan_shadow_store(store);

        let response = service
            .resolve_configuration(Request::new(ResolveConfigRequest {
                build_id: "build-deps-1".to_string(),
                configuration_name: "compileClasspath".to_string(),
                project_path: ":".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(response.success);
        assert!(response.error_message.is_empty());
        assert_eq!(1, response.artifacts.len());
        let artifact = &response.artifacts[0];
        assert_eq!("org.example", artifact.group);
        assert_eq!("demo", artifact.name);
        assert_eq!("1.2.3", artifact.version);
        assert_eq!("sources", artifact.classifier);
        assert_eq!("jar", artifact.extension);
        assert_eq!("compileClasspath", artifact.configuration);
        assert_eq!("dependency", artifact.kind);
        assert_eq!(1, artifact.repositories.len());
        assert_eq!("central", artifact.repositories[0].id);
    }

    #[tokio::test]
    async fn resolve_configuration_reports_missing_plan_and_invalid_request() {
        let service = JvmHostServiceImpl::default();

        let invalid = service
            .resolve_configuration(Request::new(ResolveConfigRequest {
                build_id: String::new(),
                configuration_name: "compileClasspath".to_string(),
                project_path: ":".to_string(),
            }))
            .await
            .unwrap_err();
        assert_eq!(tonic::Code::InvalidArgument, invalid.code());

        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(BuildPlanShadowStore::new(temp.path().to_path_buf()));
        let service = JvmHostServiceImpl::default().with_build_plan_shadow_store(store);
        let missing = service
            .resolve_configuration(Request::new(ResolveConfigRequest {
                build_id: "missing-deps".to_string(),
                configuration_name: "compileClasspath".to_string(),
                project_path: ":".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!missing.success);
        assert!(missing.artifacts.is_empty());
        assert!(missing.error_message.contains("No Rust build-plan shadow"));
    }

    #[tokio::test]
    async fn resolve_configuration_fails_closed_for_unsupported_or_malformed_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(BuildPlanShadowStore::new(temp.path().to_path_buf()));
        let mut plan = sample_canonical_plan("build-deps-unsupported");
        let mut dependency = sample_dependency(
            ":",
            "runtimeClasspath",
            "org.example:demo:1.2.3",
            "dependency",
        );
        dependency.unsupported_features = vec!["component-metadata-rule".to_string()];
        plan.dependencies = vec![dependency];
        store.persist_plan(&plan, "dependency-shadow").unwrap();
        let service =
            JvmHostServiceImpl::default().with_build_plan_shadow_store(Arc::clone(&store));

        let unsupported = service
            .resolve_configuration(Request::new(ResolveConfigRequest {
                build_id: "build-deps-unsupported".to_string(),
                configuration_name: "runtimeClasspath".to_string(),
                project_path: ":".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!unsupported.success);
        assert!(unsupported
            .error_message
            .contains("component-metadata-rule"));

        let mut malformed_plan = sample_canonical_plan("build-deps-malformed");
        malformed_plan.dependencies = vec![sample_dependency(
            ":",
            "runtimeClasspath",
            "not-enough",
            "dependency",
        )];
        store
            .persist_plan(&malformed_plan, "dependency-shadow")
            .unwrap();

        let malformed = service
            .resolve_configuration(Request::new(ResolveConfigRequest {
                build_id: "build-deps-malformed".to_string(),
                configuration_name: "runtimeClasspath".to_string(),
                project_path: ":".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!malformed.success);
        assert!(malformed.error_message.contains("unsupported notation"));
    }

    #[tokio::test]
    async fn execute_task_runs_supported_write_file_task_in_rust() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("reports/output.txt");
        let encoded = base64::engine::general_purpose::STANDARD.encode("hello from rust\n");
        let service = JvmHostServiceImpl::default();

        let response = service
            .execute_task(Request::new(ExecuteTaskRequest {
                build_id: "build-exec".to_string(),
                task_path: ":writeReport".to_string(),
                task_type: "WriteFile".to_string(),
                parameters_json: serde_json::json!({
                    "target_dir": output.to_string_lossy(),
                    "options": {
                        "static_output_text_b64": encoded,
                    }
                })
                .to_string(),
                timeout_ms: 30_000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(response.success, "{}", response.error_message);
        assert_eq!("EXECUTED", response.outcome);
        assert_eq!("rust-native", response.execution_mode);
        assert_eq!(
            "hello from rust\n",
            std::fs::read_to_string(output).unwrap()
        );
    }

    #[tokio::test]
    async fn execute_task_runs_lifecycle_noop_in_rust() {
        let service = JvmHostServiceImpl::default();

        let response = service
            .execute_task(Request::new(ExecuteTaskRequest {
                build_id: "build-exec".to_string(),
                task_path: ":classes".to_string(),
                task_type: "Lifecycle".to_string(),
                parameters_json: "{}".to_string(),
                timeout_ms: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(response.success);
        assert_eq!("EXECUTED", response.outcome);
        assert_eq!("rust-native", response.execution_mode);
    }

    #[tokio::test]
    async fn execute_task_fails_closed_for_unsupported_or_malformed_contracts() {
        let service = JvmHostServiceImpl::default();

        let unsupported = service
            .execute_task(Request::new(ExecuteTaskRequest {
                build_id: "build-exec".to_string(),
                task_path: ":custom".to_string(),
                task_type: "CustomJvmTask".to_string(),
                parameters_json: "{}".to_string(),
                timeout_ms: 0,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!unsupported.success);
        assert_eq!("UNSUPPORTED", unsupported.outcome);
        assert_eq!("rust-native-unsupported", unsupported.execution_mode);

        let malformed = service
            .execute_task(Request::new(ExecuteTaskRequest {
                build_id: "build-exec".to_string(),
                task_path: ":copy".to_string(),
                task_type: "Copy".to_string(),
                parameters_json: "{not-json".to_string(),
                timeout_ms: 0,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!malformed.success);
        assert_eq!("FAILED", malformed.outcome);
        assert_eq!("rust-native-contract-error", malformed.execution_mode);
        assert!(malformed.error_message.contains("Invalid parameters_json"));
    }

    #[tokio::test]
    async fn execute_task_validates_required_identity_fields() {
        let service = JvmHostServiceImpl::default();

        let error = service
            .execute_task(Request::new(ExecuteTaskRequest {
                build_id: String::new(),
                task_path: ":classes".to_string(),
                task_type: "Lifecycle".to_string(),
                parameters_json: "{}".to_string(),
                timeout_ms: 0,
            }))
            .await
            .unwrap_err();

        assert_eq!(tonic::Code::InvalidArgument, error.code());
    }

    #[tokio::test]
    async fn evaluate_script_fails_closed_as_jvm_owned_surface() {
        let service = JvmHostServiceImpl::default();

        let response = service
            .evaluate_script(Request::new(EvaluateScriptRequest {
                script_path: "build.gradle.kts".to_string(),
                script_content: "plugins { java }".to_string(),
                script_type: "kotlin-dsl".to_string(),
                extra_properties: HashMap::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.success);
        assert!(response.applied_plugins.is_empty());
        assert!(response.error_message.contains("build.gradle.kts"));
        assert!(response.error_message.contains("kotlin-dsl"));
        assert!(response.error_message.contains("JVM-owned"));
    }

    #[tokio::test]
    async fn evaluate_script_empty_input_still_fails_closed_without_grpc_error() {
        let service = JvmHostServiceImpl::default();

        let response = service
            .evaluate_script(Request::new(EvaluateScriptRequest {
                script_path: String::new(),
                script_content: String::new(),
                script_type: String::new(),
                extra_properties: HashMap::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.success);
        assert!(response.applied_plugins.is_empty());
        assert!(response.error_message.contains("<inline>"));
        assert!(response.error_message.contains("unknown"));
    }

    #[test]
    fn parse_resolved_artifact_notation_preserves_classifier_and_extension() {
        assert_eq!(
            Some(ResolvedArtifactCoordinate {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.2.3".to_string(),
                classifier: String::new(),
                extension: "jar".to_string(),
            }),
            parse_resolved_artifact_notation("org.example:demo:1.2.3")
        );
        assert_eq!(
            Some(ResolvedArtifactCoordinate {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.2.3".to_string(),
                classifier: "debug".to_string(),
                extension: "aar".to_string(),
            }),
            parse_resolved_artifact_notation("org.example:demo:1.2.3:debug@aar")
        );
        assert!(parse_resolved_artifact_notation("org.example:demo").is_none());
    }

    #[test]
    fn direct_child_project_paths_respect_nested_hierarchy() {
        let projects = vec![
            ":app".to_string(),
            ":lib".to_string(),
            ":lib:core".to_string(),
            ":lib:util".to_string(),
            ":lib:util:test-fixtures".to_string(),
        ];

        assert_eq!(
            vec![":app".to_string(), ":lib".to_string()],
            direct_child_project_paths(":", &projects)
        );
        assert_eq!(
            vec![":lib:core".to_string(), ":lib:util".to_string()],
            direct_child_project_paths(":lib", &projects)
        );
        assert_eq!(
            vec![":lib:util:test-fixtures".to_string()],
            direct_child_project_paths(":lib:util", &projects)
        );
    }

    #[test]
    fn project_paths_with_ancestors_matches_gradle_include_walk() {
        assert_eq!(
            vec![
                ":app".to_string(),
                ":lib".to_string(),
                ":lib:core".to_string(),
                ":lib:util".to_string(),
            ],
            project_paths_with_ancestors(&[
                ":app".to_string(),
                ":lib:core".to_string(),
                ":lib:util".to_string(),
            ])
        );
    }

    fn sample_canonical_plan(build_id: &str) -> CanonicalBuildPlan {
        CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: build_id.to_string(),
            projects: vec![CanonicalBuildPlanProject {
                path: ":".to_string(),
                name: "root".to_string(),
                project_dir: "/repo".to_string(),
            }],
            tasks: vec![CanonicalBuildPlanTask {
                path: ":compileJava".to_string(),
                project_path: ":".to_string(),
                implementation_id: "JavaCompile".to_string(),
                depends_on: Vec::new(),
                inputs: BTreeMap::new(),
                outputs: vec!["/repo/build/classes/java/main".to_string()],
                worker_isolation: "none".to_string(),
                should_run_after: Vec::new(),
                must_run_after: Vec::new(),
                finalized_by: Vec::new(),
                cacheability: "cacheable".to_string(),
                local_state: Vec::new(),
                destroyables: Vec::new(),
                action_kind: "java-compile".to_string(),
                input_specs: Vec::new(),
                output_specs: Vec::new(),
                environment_inputs: Vec::new(),
                system_property_inputs: Vec::new(),
                diagnostics: Vec::new(),
            }],
            dependencies: Vec::new(),
            toolchains: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    fn sample_dependency(
        project_path: &str,
        configuration: &str,
        notation: &str,
        kind: &str,
    ) -> CanonicalBuildPlanDependency {
        CanonicalBuildPlanDependency {
            project_path: project_path.to_string(),
            configuration: configuration.to_string(),
            notation: notation.to_string(),
            kind: kind.to_string(),
            repositories: vec![CanonicalBuildPlanRepository {
                id: "central".to_string(),
                url: "https://repo.maven.apache.org/maven2".to_string(),
                m2compatible: true,
                allow_insecure_protocol: false,
                credentials: BTreeMap::new(),
                layout: "maven".to_string(),
                ivy_pattern: String::new(),
                include_groups: Vec::new(),
                exclude_groups: Vec::new(),
                include_group_prefixes: Vec::new(),
                exclude_group_prefixes: Vec::new(),
                include_modules: Vec::new(),
                exclude_modules: Vec::new(),
                include_module_versions: Vec::new(),
                exclude_module_versions: Vec::new(),
            }],
            unsupported_features: Vec::new(),
        }
    }
}
