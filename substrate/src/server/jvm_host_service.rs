//! JvmHostService — Rust implementation of the JVM compatibility host.
//!
//! The JvmHostService exposes build environment, configuration resolution,
//! build model, and task execution APIs that originally lived in the JVM
//! Gradle daemon. This module progressively ports those RPCs to pure Rust.
//!
//! Currently implemented: GetBuildEnvironment.
//! Remaining: EvaluateScript, GetBuildModel, ResolveConfiguration,
//! GetBuildPlan, ExecuteTask (stubbed for future work).

use std::collections::HashMap;
use std::env;
use std::path::{Path, MAIN_SEPARATOR};
use std::process::Command;

use tonic::{Request, Response, Status};

use crate::proto::jvm_host_service_server::JvmHostService;
use crate::proto::{
    EvaluateScriptRequest, EvaluateScriptResponse, ExecuteTaskRequest, ExecuteTaskResponse,
    GetBuildEnvironmentRequest, GetBuildEnvironmentResponse, GetBuildModelRequest,
    GetBuildModelResponse, GetBuildPlanRequest, GetBuildPlanResponse, ResolveConfigRequest,
    ResolveConfigResponse,
};

use crate::server::platform::{CurrentPlatform, PlatformOps};

/// JvmHostService implementation.
#[derive(Debug, Default, Clone)]
pub struct JvmHostServiceImpl;

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
        _request: Request<GetBuildModelRequest>,
    ) -> Result<Response<GetBuildModelResponse>, Status> {
        // TODO: Port build model extraction from JVM to Rust
        Err(Status::unimplemented("GetBuildModel not yet ported to Rust"))
    }

    async fn resolve_configuration(
        &self,
        _request: Request<ResolveConfigRequest>,
    ) -> Result<Response<ResolveConfigResponse>, Status> {
        // TODO: Port dependency resolution to Rust (large subsystem)
        Err(Status::unimplemented("ResolveConfiguration not yet ported to Rust"))
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
        Err(Status::unimplemented("EvaluateScript not yet ported to Rust"))
    }
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
