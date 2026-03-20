use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    build_init_service_server::BuildInitService, BuildInitStatus, GetBuildInitStatusRequest,
    GetBuildInitStatusResponse, InitBuildSettingsRequest, InitBuildSettingsResponse,
    RecordInitScriptRequest, RecordInitScriptResponse, RecordSettingsDetailRequest,
    RecordSettingsDetailResponse, SettingsDetailEntry,
};

/// Tracked build initialization state.
struct BuildInitState {
    build_id: String,
    root_dir: String,
    initialized: bool,
    init_duration_ms: i64,
    settings_details: Vec<SettingsDetailEntry>,
    init_scripts: Vec<String>,
    included_projects: Vec<String>,
}

/// Rust-native build initialization service.
/// Manages build startup, settings processing, and init scripts.
pub struct BuildInitServiceImpl {
    builds: DashMap<String, BuildInitState>,
}

impl BuildInitServiceImpl {
    pub fn new() -> Self {
        Self {
            builds: DashMap::new(),
        }
    }
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

        self.builds.insert(
            build_id.clone(),
            BuildInitState {
                build_id,
                root_dir: root_dir,
                initialized: true,
                init_duration_ms: 0,
                settings_details: Vec::new(),
                init_scripts: Vec::new(),
                included_projects: Vec::new(),
            },
        );

        let init_duration_ms = start.elapsed().as_millis() as i64;

        if let Some(mut build) = self.builds.get_mut(&req.build_id) {
            build.init_duration_ms = init_duration_ms;
        }

        tracing::info!(
            build_id = %req.build_id,
            root_dir = root_dir_log,
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

        if let Some(mut build) = self.builds.get_mut(&req.build_id) {
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

        if let Some(build) = self.builds.get(&req.build_id) {
            Ok(Response::new(GetBuildInitStatusResponse {
                status: Some(BuildInitStatus {
                    build_id: build.build_id.clone(),
                    root_dir: build.root_dir.clone(),
                    initialized: build.initialized,
                    init_duration_ms: build.init_duration_ms,
                    settings_details: build.settings_details.clone(),
                    executed_init_scripts: build.init_scripts.clone(),
                    included_projects: build.included_projects.clone(),
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

        if let Some(mut build) = self.builds.get_mut(&req.build_id) {
            build.init_scripts.push(req.script_path.clone());

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

        assert_eq!(status.settings_details.len(), 2);
        assert_eq!(status.settings_details[0].value, "updated-project");
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
}
