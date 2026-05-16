//! JvmHostService — Rust implementation of the JVM compatibility host.
//!
//! The JvmHostService exposes build environment, configuration resolution,
//! build model, and task execution APIs that originally lived in the JVM
//! Gradle daemon. This module progressively ports those RPCs to pure Rust.
//!
//! Currently implemented: GetBuildEnvironment, GetBuildModel.
//! Remaining: ResolveConfiguration, GetBuildPlan, ExecuteTask, EvaluateScript.

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
    ResolveConfigRequest, ResolveConfigResponse,
};

use crate::server::build_init::BuildInitServiceImpl;
use crate::server::platform::{CurrentPlatform, PlatformOps};
use crate::server::scopes::BuildId;

/// JvmHostService implementation.
#[derive(Debug, Clone)]
pub struct JvmHostServiceImpl {
    build_registry: Arc<DashMap<BuildId, String>>,
}

impl JvmHostServiceImpl {
    /// Create a new JvmHostService with access to the shared build registry.
    pub fn new(build_registry: Arc<DashMap<BuildId, String>>) -> Self {
        Self { build_registry }
    }
}

impl Default for JvmHostServiceImpl {
    fn default() -> Self {
        // In production, JvmHostServiceImpl is always constructed with a real
        // build_registry from BootstrapService. This default exists only for
        // tests that don't need registry lookups.
        Self {
            build_registry: Arc::new(DashMap::new()),
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
        _request: Request<ResolveConfigRequest>,
    ) -> Result<Response<ResolveConfigResponse>, Status> {
        // TODO: Port dependency resolution to Rust (large subsystem)
        Err(Status::unimplemented(
            "ResolveConfiguration not yet ported to Rust",
        ))
    }

    async fn get_build_plan(
        &self,
        _request: Request<GetBuildPlanRequest>,
    ) -> Result<Response<GetBuildPlanResponse>, Status> {
        // TODO: Port build plan generation to Rust
        Err(Status::unimplemented("GetBuildPlan not yet ported to Rust"))
    }

    async fn execute_task(
        &self,
        _request: Request<ExecuteTaskRequest>,
    ) -> Result<Response<ExecuteTaskResponse>, Status> {
        // TODO: Port task execution to Rust
        Err(Status::unimplemented("ExecuteTask not yet ported to Rust"))
    }

    async fn evaluate_script(
        &self,
        _request: Request<EvaluateScriptRequest>,
    ) -> Result<Response<EvaluateScriptResponse>, Status> {
        // TODO: Port Groovy/Kotlin script evaluator to Rust
        Err(Status::unimplemented(
            "EvaluateScript not yet ported to Rust",
        ))
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
    use super::*;
    use crate::proto::bootstrap_service_server::BootstrapService;
    use crate::proto::{CompleteBuildRequest, InitBuildRequest};
    use crate::server::bootstrap::BootstrapServiceImpl;

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
}
