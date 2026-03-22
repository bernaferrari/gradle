use std::path::Path;
use std::sync::Arc;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use super::scopes::{BuildId, ScopeRegistry, SessionId};

use crate::proto::{
    build_init_service_server::BuildInitService, BuildInitStatus, GetBuildInitStatusRequest,
    GetBuildInitStatusResponse, InitBuildSettingsRequest, InitBuildSettingsResponse,
    InitScriptInfo, RecordInitScriptRequest, RecordInitScriptResponse,
    RecordSettingsDetailRequest, RecordSettingsDetailResponse, SettingsDetailEntry,
};

/// Tracked build initialization state.
struct BuildInitState {
    build_id: String,
    root_dir: String,
    initialized: bool,
    init_duration_ms: i64,
    settings_details: Vec<SettingsDetailEntry>,
    init_scripts: Vec<InitScriptRecord>,
    included_projects: Vec<String>,
    included_builds: Vec<String>,
    root_project_name: Option<String>,
    settings_file_exists: bool,
    gradle_version: String,
}

/// Record of an executed init script.
struct InitScriptRecord {
    path: String,
    success: bool,
    duration_ms: i64,
}

/// Rust-native build initialization service.
/// Manages build startup, settings processing, init scripts, and settings file parsing.
#[derive(Default)]
pub struct BuildInitServiceImpl {
    builds: DashMap<BuildId, BuildInitState>,
    scope_registry: Option<Arc<ScopeRegistry>>,
}

impl BuildInitServiceImpl {
    pub fn new() -> Self {
        Self {
            builds: DashMap::new(),
            scope_registry: None,
        }
    }

    pub fn with_scope_registry(scope_registry: Arc<ScopeRegistry>) -> Self {
        Self {
            builds: DashMap::new(),
            scope_registry: Some(scope_registry),
        }
    }

    /// Parse a Gradle settings file to extract root project name, included builds,
    /// and other build structure information.
    fn parse_settings_file(root_dir: &str, settings_file: &str) -> ParsedSettings {
        let mut result = ParsedSettings::default();

        let settings_path = if settings_file.is_empty() {
            Path::new(root_dir).join("settings.gradle")
        } else {
            Path::new(settings_file).to_path_buf()
        };

        // Also check for settings.gradle.kts
        let settings_kts_path = Path::new(root_dir).join("settings.gradle.kts");
        let actual_settings = if settings_path.exists() {
            Some(settings_path)
        } else if settings_kts_path.exists() {
            Some(settings_kts_path)
        } else {
            None
        };

        result.settings_file_exists = actual_settings.is_some();

        if let Some(path) = actual_settings {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let is_kts = path
                    .extension()
                    .map(|e| e == "kts")
                    .unwrap_or(false);
                result.is_kotlin_dsl = is_kts;
                Self::extract_settings_info(&content, is_kts, &mut result);
            }
        }

        // Detect root project name from directory if not set in settings
        if result.root_project_name.is_none() {
            result.root_project_name = Some(
                Path::new(root_dir)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unnamed")
                    .to_string(),
            );
        }

        result
    }

    /// Extract information from settings file content.
    fn extract_settings_info(content: &str, is_kts: bool, result: &mut ParsedSettings) {
        // Extract rootProject.name
        for line in content.lines() {
            let trimmed = line.trim();

            // Groovy: rootProject.name = "foo" or rootProject.name = 'foo'
            if !is_kts {
                if let Some(name) = Self::extract_groovy_assignment(trimmed, "rootProject.name") {
                    result.root_project_name = Some(name);
                }
                // include ':subproject'
                if trimmed.starts_with("include ") || trimmed.starts_with("include(") {
                    Self::extract_included_projects_groovy(trimmed, &mut result.included_projects);
                }
                // includeBuild 'path' or includeBuild("path")
                if trimmed.starts_with("includeBuild ") || trimmed.starts_with("includeBuild(") {
                    Self::extract_included_builds_groovy(trimmed, &mut result.included_builds);
                }
            } else {
                // Kotlin DSL: rootProject.name = "foo"
                if let Some(name) = Self::extract_kotlin_assignment(trimmed, "rootProject.name") {
                    result.root_project_name = Some(name);
                }
                // include(":subproject")
                if trimmed.starts_with("include(") || trimmed.contains("include(\"") {
                    Self::extract_included_projects_kotlin(trimmed, &mut result.included_projects);
                }
                // includeBuild("path")
                if trimmed.starts_with("includeBuild(") || trimmed.contains("includeBuild(\"") {
                    Self::extract_included_builds_kotlin(trimmed, &mut result.included_builds);
                }
            }
        }
    }

    fn extract_groovy_assignment(line: &str, prefix: &str) -> Option<String> {
        let expected = format!("{} =", prefix);
        if let Some(idx) = line.find(&expected) {
            let value = line[idx + expected.len()..].trim();
            // Remove quotes (single or double)
            let value = value
                .strip_prefix('"')
                .or_else(|| value.strip_prefix('\''))
                .and_then(|v| v.strip_suffix('"').or_else(|| v.strip_suffix('\'')));
            if let Some(name) = value {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
        None
    }

    fn extract_kotlin_assignment(line: &str, prefix: &str) -> Option<String> {
        let expected = format!("{} =", prefix);
        if let Some(idx) = line.find(&expected) {
            let value = line[idx + expected.len()..].trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'));
            if let Some(name) = value {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
        None
    }

    fn extract_included_projects_groovy(line: &str, projects: &mut Vec<String>) {
        // include ':app', ':lib' or include ':app', ':lib'
        let rest = if let Some(r) = line.strip_prefix("include ") {
            r
        } else if let Some(r) = line.strip_prefix("include(") {
            r
        } else {
            return;
        };

        // Split by commas
        for part in rest.split(',') {
            let part = part.trim();
            let cleaned = part
                .strip_prefix("'")
                .and_then(|v| v.strip_suffix("'"))
                .or_else(|| part.strip_prefix('"').and_then(|v| v.strip_suffix('"')))
                .unwrap_or(part);
            let cleaned = cleaned.trim();
            if !cleaned.is_empty() && cleaned.starts_with(':') {
                projects.push(cleaned.to_string());
            }
        }
    }

    fn extract_included_projects_kotlin(line: &str, projects: &mut Vec<String>) {
        // include(":app", ":lib")
        if !line.contains("include(") {
            return;
        }
        // Find the content between the first ( and last )
        if let Some(start) = line.find('(') {
            let rest = &line[start + 1..];
            let end = rest.find(')').unwrap_or(rest.len());
            let content = &rest[..end];

            for part in content.split(',') {
                let part = part.trim();
                let cleaned = part
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'));
                if let Some(name) = cleaned {
                    let name = name.trim();
                    if !name.is_empty() && name.starts_with(':') {
                        projects.push(name.to_string());
                    }
                }
            }
        }
    }

    fn extract_included_builds_groovy(line: &str, builds: &mut Vec<String>) {
        let rest = if let Some(r) = line.strip_prefix("includeBuild ") {
            r
        } else if let Some(r) = line.strip_prefix("includeBuild(") {
            r
        } else {
            return;
        };

        let cleaned = rest
            .trim()
            .strip_prefix("'")
            .and_then(|v| v.strip_suffix("'"))
            .or_else(|| {
                rest.trim()
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
            })
            .or_else(|| rest.trim().strip_suffix(')').and_then(|v| v.strip_suffix('"')))
            .unwrap_or(rest.trim());

        let cleaned = cleaned.trim();
        if !cleaned.is_empty() {
            builds.push(cleaned.to_string());
        }
    }

    fn extract_included_builds_kotlin(line: &str, builds: &mut Vec<String>) {
        // includeBuild("path")
        if !line.contains("includeBuild") {
            return;
        }
        if let Some(start) = line.find("includeBuild") {
            let rest = &line[start + 13..]; // skip "includeBuild"
            // Skip whitespace and opening paren
            let rest = rest.trim_start();
            let rest = rest.strip_prefix('(').unwrap_or(rest);
            let rest = rest.trim_start();

            // Find the closing paren or end of string
            let end = rest.find(')').unwrap_or(rest.len());
            let path = rest[..end].trim();

            let cleaned = path
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(path);

            if !cleaned.is_empty() {
                builds.push(cleaned.to_string());
            }
        }
    }
}

/// Parsed settings file information.
#[derive(Default)]
struct ParsedSettings {
    root_project_name: Option<String>,
    included_projects: Vec<String>,
    included_builds: Vec<String>,
    settings_file_exists: bool,
    is_kotlin_dsl: bool,
}

#[tonic::async_trait]
impl BuildInitService for BuildInitServiceImpl {
    async fn init_build_settings(
        &self,
        request: Request<InitBuildSettingsRequest>,
    ) -> Result<Response<InitBuildSettingsResponse>, Status> {
        let req = request.into_inner();
        let start = std::time::Instant::now();

        let build_id = req.build_id.clone();
        let root_dir = req.root_dir.clone();
        let root_dir_log = root_dir.clone();

        // Parse the settings file
        let parsed = Self::parse_settings_file(&root_dir, &req.settings_file);

        let mut settings_details = Vec::new();
        settings_details.push(SettingsDetailEntry {
            key: "settingsFileExists".to_string(),
            value: if parsed.settings_file_exists {
                "true"
            } else {
                "false"
            }
            .to_string(),
        });

        if parsed.is_kotlin_dsl {
            settings_details.push(SettingsDetailEntry {
                key: "settingsDsl".to_string(),
                value: "kotlin".to_string(),
            });
        }

        if let Some(ref name) = parsed.root_project_name {
            settings_details.push(SettingsDetailEntry {
                key: "rootProjectName".to_string(),
                value: name.clone(),
            });
        }

        if !parsed.included_projects.is_empty() {
            settings_details.push(SettingsDetailEntry {
                key: "includedProjects".to_string(),
                value: parsed.included_projects.join(","),
            });
        }

        if !parsed.included_builds.is_empty() {
            settings_details.push(SettingsDetailEntry {
                key: "includedBuilds".to_string(),
                value: parsed.included_builds.join(","),
            });
        }

        let build_key = BuildId::from(build_id.clone());

        // Register build in scope registry if session_id is provided
        if let Some(ref registry) = self.scope_registry {
            if !req.session_id.is_empty() {
                registry.register_build(
                    SessionId::from(req.session_id.clone()),
                    build_key.clone(),
                );
                tracing::debug!(
                    build_id = %build_id,
                    session_id = %req.session_id,
                    "Registered build in scope registry (build-init)"
                );
            }
        }

        self.builds.insert(
            build_key,
            BuildInitState {
                build_id,
                root_dir,
                initialized: true,
                init_duration_ms: 0,
                settings_details,
                init_scripts: Vec::new(),
                included_projects: parsed.included_projects,
                included_builds: parsed.included_builds,
                root_project_name: parsed.root_project_name,
                settings_file_exists: parsed.settings_file_exists,
                gradle_version: String::new(),
            },
        );

        let init_duration_ms = start.elapsed().as_millis() as i64;

        if let Some(mut build) = self.builds.get_mut(&BuildId::from(req.build_id.clone())) {
            build.init_duration_ms = init_duration_ms;
        }

        tracing::info!(
            build_id = %req.build_id,
            root_dir = %root_dir_log,
            root_project = ?self.builds.get(&BuildId::from(req.build_id.clone())).and_then(|b| b.root_project_name.clone()),
            init_ms = init_duration_ms,
            "Build initialized"
        );

        Ok(Response::new(InitBuildSettingsResponse {
            build_id: req.build_id,
            initialized: true,
            init_duration_ms,
            error_message: String::new(),
        }))
    }

    async fn record_settings_detail(
        &self,
        request: Request<RecordSettingsDetailRequest>,
    ) -> Result<Response<RecordSettingsDetailResponse>, Status> {
        let req = request.into_inner();

        let detail = req
            .detail
            .ok_or_else(|| Status::invalid_argument("SettingsDetail is required"))?;

        if let Some(mut build) = self.builds.get_mut(&BuildId::from(req.build_id)) {
            // Update if key exists, else push
            if let Some(existing) = build.settings_details.iter_mut().find(|d| d.key == detail.key) {
                existing.value = detail.value;
            } else {
                build.settings_details.push(detail);
            }
        }

        Ok(Response::new(RecordSettingsDetailResponse { accepted: true }))
    }

    async fn get_build_init_status(
        &self,
        request: Request<GetBuildInitStatusRequest>,
    ) -> Result<Response<GetBuildInitStatusResponse>, Status> {
        let req = request.into_inner();

        if let Some(build) = self.builds.get(&BuildId::from(req.build_id)) {
            let script_infos: Vec<InitScriptInfo> = build
                .init_scripts
                .iter()
                .map(|s| InitScriptInfo {
                    path: s.path.clone(),
                    success: s.success,
                    duration_ms: s.duration_ms,
                })
                .collect();

            Ok(Response::new(GetBuildInitStatusResponse {
                status: Some(BuildInitStatus {
                    build_id: build.build_id.clone(),
                    root_dir: build.root_dir.clone(),
                    initialized: build.initialized,
                    init_duration_ms: build.init_duration_ms,
                    settings_details: build.settings_details.clone(),
                    executed_init_scripts: script_infos,
                    included_projects: build.included_projects.clone(),
                    included_builds: build.included_builds.clone(),
                    settings_file_exists: build.settings_file_exists,
                    gradle_version: if build.gradle_version.is_empty() {
                        None
                    } else {
                        Some(build.gradle_version.clone())
                    },
                }),
            }))
        } else {
            Ok(Response::new(GetBuildInitStatusResponse { status: None }))
        }
    }

    async fn record_init_script(
        &self,
        request: Request<RecordInitScriptRequest>,
    ) -> Result<Response<RecordInitScriptResponse>, Status> {
        let req = request.into_inner();

        if let Some(mut build) = self.builds.get_mut(&BuildId::from(req.build_id)) {
            build.init_scripts.push(InitScriptRecord {
                path: req.script_path.clone(),
                success: req.success,
                duration_ms: req.duration_ms,
            });

            if !req.success {
                tracing::warn!(
                    script = %req.script_path,
                    error = %req.error_message,
                    "Init script failed"
                );
            }
        }

        Ok(Response::new(RecordInitScriptResponse { accepted: true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_and_get_status() {
        let svc = BuildInitServiceImpl::new();

        svc.init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: "build-1".to_string(),
            root_dir: "/tmp/project".to_string(),
            settings_file: "/tmp/project/settings.gradle".to_string(),
            gradle_user_home: "/tmp/gradle-home".to_string(),
            init_scripts: vec![],
            requested_build_features: vec![],
            current_dir: "/tmp/project".to_string(),
            session_id: String::new(),
        }))
        .await
        .unwrap();

        let resp = svc
            .get_build_init_status(Request::new(GetBuildInitStatusRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let status = resp.status.unwrap();
        assert!(status.initialized);
        assert_eq!(status.root_dir, "/tmp/project");
    }

    #[tokio::test]
    async fn test_settings_details() {
        let svc = BuildInitServiceImpl::new();

        svc.init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: "build-2".to_string(),
            root_dir: "/tmp".to_string(),
            settings_file: String::new(),
            gradle_user_home: String::new(),
            init_scripts: vec![],
            requested_build_features: vec![],
            current_dir: String::new(),
            session_id: String::new(),
        }))
        .await
        .unwrap();

        svc.record_settings_detail(Request::new(RecordSettingsDetailRequest {
            build_id: "build-2".to_string(),
            detail: Some(SettingsDetailEntry {
                key: "rootProjectName".to_string(),
                value: "my-project".to_string(),
            }),
        }))
        .await
        .unwrap();

        svc.record_settings_detail(Request::new(RecordSettingsDetailRequest {
            build_id: "build-2".to_string(),
            detail: Some(SettingsDetailEntry {
                key: "includedProjects".to_string(),
                value: ":app,:lib".to_string(),
            }),
        }))
        .await
        .unwrap();

        // Update existing
        svc.record_settings_detail(Request::new(RecordSettingsDetailRequest {
            build_id: "build-2".to_string(),
            detail: Some(SettingsDetailEntry {
                key: "rootProjectName".to_string(),
                value: "updated-project".to_string(),
            }),
        }))
        .await
        .unwrap();

        let status = svc
            .get_build_init_status(Request::new(GetBuildInitStatusRequest {
                build_id: "build-2".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .status
            .unwrap();

        assert!(status.settings_details.len() >= 2);
        // Find the rootProjectName detail and check it was updated
        let root_name: Option<&str> = status
            .settings_details
            .iter()
            .find(|d| d.key == "rootProjectName")
            .map(|d| d.value.as_str());
        assert_eq!(root_name, Some("updated-project"));
    }

    #[tokio::test]
    async fn test_init_scripts() {
        let svc = BuildInitServiceImpl::new();

        svc.init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: "build-3".to_string(),
            root_dir: "/tmp".to_string(),
            settings_file: String::new(),
            gradle_user_home: String::new(),
            init_scripts: vec![],
            requested_build_features: vec![],
            current_dir: String::new(),
            session_id: String::new(),
        }))
        .await
        .unwrap();

        svc.record_init_script(Request::new(RecordInitScriptRequest {
            build_id: "build-3".to_string(),
            script_path: "/tmp/init.gradle".to_string(),
            success: true,
            error_message: String::new(),
            duration_ms: 50,
        }))
        .await
        .unwrap();

        let status = svc
            .get_build_init_status(Request::new(GetBuildInitStatusRequest {
                build_id: "build-3".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .status
            .unwrap();

        assert_eq!(status.executed_init_scripts.len(), 1);
        assert_eq!(status.executed_init_scripts[0].path, "/tmp/init.gradle");
        assert!(status.executed_init_scripts[0].success);
        assert_eq!(status.executed_init_scripts[0].duration_ms, 50);
    }

    #[tokio::test]
    async fn test_unknown_build() {
        let svc = BuildInitServiceImpl::new();

        let resp = svc
            .get_build_init_status(Request::new(GetBuildInitStatusRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.status.is_none());
    }

    #[tokio::test]
    async fn test_settings_file_parsing_groovy() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.gradle");
        std::fs::write(
            &settings_path,
            r#"
rootProject.name = 'my-app'
include ':app', ':lib', ':common'
includeBuild 'platform'
includeBuild 'plugins'
"#,
        )
        .unwrap();

        let parsed = BuildInitServiceImpl::parse_settings_file(
            dir.path().to_str().unwrap(),
            settings_path.to_str().unwrap(),
        );

        assert_eq!(parsed.root_project_name, Some("my-app".to_string()));
        assert_eq!(parsed.settings_file_exists, true);
        assert!(!parsed.is_kotlin_dsl);
        assert!(parsed.included_projects.contains(&":app".to_string()));
        assert!(parsed.included_projects.contains(&":lib".to_string()));
        assert!(parsed.included_projects.contains(&":common".to_string()));
        assert!(parsed.included_builds.contains(&"platform".to_string()));
        assert!(parsed.included_builds.contains(&"plugins".to_string()));
    }

    #[tokio::test]
    async fn test_settings_file_parsing_kotlin() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.gradle.kts");
        std::fs::write(
            &settings_path,
            r#"
rootProject.name = "kotlin-app"
include(":app", ":lib")
includeBuild("platform")
"#,
        )
        .unwrap();

        let parsed = BuildInitServiceImpl::parse_settings_file(
            dir.path().to_str().unwrap(),
            settings_path.to_str().unwrap(),
        );

        assert_eq!(parsed.root_project_name, Some("kotlin-app".to_string()));
        assert!(parsed.is_kotlin_dsl);
        assert!(parsed.included_projects.contains(&":app".to_string()));
        assert!(parsed.included_projects.contains(&":lib".to_string()));
        assert!(parsed.included_builds.contains(&"platform".to_string()));
    }

    #[tokio::test]
    async fn test_settings_file_missing() {
        let dir = tempfile::tempdir().unwrap();

        let parsed = BuildInitServiceImpl::parse_settings_file(
            dir.path().to_str().unwrap(),
            "",
        );

        assert!(!parsed.settings_file_exists);
        // Falls back to directory name
        assert!(parsed.root_project_name.is_some());
    }

    #[tokio::test]
    async fn test_init_with_real_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.gradle");
        std::fs::write(
            &settings_path,
            r#"rootProject.name = 'test-project'
include ':core'
"#,
        )
        .unwrap();

        let svc = BuildInitServiceImpl::new();

        svc.init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: "build-settings".to_string(),
            root_dir: dir.path().to_str().unwrap().to_string(),
            settings_file: settings_path.to_str().unwrap().to_string(),
            gradle_user_home: String::new(),
            init_scripts: vec![],
            requested_build_features: vec![],
            current_dir: dir.path().to_str().unwrap().to_string(),
            session_id: String::new(),
        }))
        .await
        .unwrap();

        let status = svc
            .get_build_init_status(Request::new(GetBuildInitStatusRequest {
                build_id: "build-settings".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .status
            .unwrap();

        assert_eq!(status.included_projects, vec![":core".to_string()]);
        assert!(status.settings_file_exists);
        assert!(status.gradle_version.is_none()); // not set during init

        // Check settings details
        let root_name: Vec<&str> = status
            .settings_details
            .iter()
            .filter(|d| d.key == "rootProjectName")
            .map(|d| d.value.as_str())
            .collect();
        assert_eq!(root_name, vec!["test-project"]);
    }

    #[tokio::test]
    async fn test_record_settings_detail_missing() {
        let svc = BuildInitServiceImpl::new();

        // Recording to nonexistent build should succeed (silently ignored)
        let resp = svc
            .record_settings_detail(Request::new(RecordSettingsDetailRequest {
                build_id: "nonexistent".to_string(),
                detail: Some(SettingsDetailEntry {
                    key: "k".to_string(),
                    value: "v".to_string(),
                }),
            }))
            .await
        .unwrap()
        .into_inner();

        assert!(resp.accepted);
    }

    #[tokio::test]
    async fn test_record_init_script_missing_build() {
        let svc = BuildInitServiceImpl::new();

        // Recording init script for nonexistent build should succeed
        let resp = svc
            .record_init_script(Request::new(RecordInitScriptRequest {
                build_id: "nonexistent".to_string(),
                script_path: "/tmp/init.gradle".to_string(),
                success: true,
                error_message: String::new(),
                duration_ms: 50,
            }))
            .await
        .unwrap()
        .into_inner();

        assert!(resp.accepted);
    }

    #[tokio::test]
    async fn test_multiple_init_scripts() {
        let svc = BuildInitServiceImpl::new();

        svc.init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: "multi-scripts".to_string(),
            root_dir: "/tmp".to_string(),
            settings_file: String::new(),
            gradle_user_home: String::new(),
            init_scripts: vec![],
            requested_build_features: vec![],
            current_dir: String::new(),
            session_id: String::new(),
        }))
        .await
        .unwrap();

        svc.record_init_script(Request::new(RecordInitScriptRequest {
            build_id: "multi-scripts".to_string(),
            script_path: "/tmp/init1.gradle".to_string(),
            success: true,
            error_message: String::new(),
            duration_ms: 10,
        }))
        .await
        .unwrap();

        svc.record_init_script(Request::new(RecordInitScriptRequest {
            build_id: "multi-scripts".to_string(),
            script_path: "/tmp/init2.gradle".to_string(),
            success: false,
            error_message: "boom".to_string(),
            duration_ms: 5,
        }))
        .await
        .unwrap();

        let status = svc
            .get_build_init_status(Request::new(GetBuildInitStatusRequest {
                build_id: "multi-scripts".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .status
            .unwrap();

        assert_eq!(status.executed_init_scripts.len(), 2);
        assert_eq!(status.executed_init_scripts[0].path, "/tmp/init1.gradle");
        assert!(status.executed_init_scripts[0].success);
        assert_eq!(status.executed_init_scripts[0].duration_ms, 10);
        assert_eq!(status.executed_init_scripts[1].path, "/tmp/init2.gradle");
        assert!(!status.executed_init_scripts[1].success);
        assert_eq!(status.executed_init_scripts[1].duration_ms, 5);
    }

    #[tokio::test]
    async fn test_reinit_build() {
        let svc = BuildInitServiceImpl::new();

        svc.init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: "reinit".to_string(),
            root_dir: "/tmp/old".to_string(),
            settings_file: String::new(),
            gradle_user_home: String::new(),
            init_scripts: vec![],
            requested_build_features: vec![],
            current_dir: String::new(),
            session_id: String::new(),
        }))
        .await
        .unwrap();

        // Re-init with different root_dir
        svc.init_build_settings(Request::new(InitBuildSettingsRequest {
            build_id: "reinit".to_string(),
            root_dir: "/tmp/new".to_string(),
            settings_file: String::new(),
            gradle_user_home: String::new(),
            init_scripts: vec![],
            requested_build_features: vec![],
            current_dir: String::new(),
            session_id: String::new(),
        }))
        .await
        .unwrap();

        let status = svc
            .get_build_init_status(Request::new(GetBuildInitStatusRequest {
                build_id: "reinit".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .status
            .unwrap();

        assert_eq!(status.root_dir, "/tmp/new");
        assert!(status.initialized);
    }
}
