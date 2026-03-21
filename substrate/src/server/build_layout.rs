use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    build_layout_service_server::BuildLayoutService, AddSubprojectRequest, AddSubprojectResponse,
    GetBuildFilePathRequest, GetBuildFilePathResponse, GetProjectTreeRequest,
    GetProjectTreeResponse, InitBuildLayoutRequest, InitBuildLayoutResponse, ListProjectsRequest,
    ListProjectsResponse, ProjectNode,
};

/// A registered project in the build layout.
struct Project {
    project_path: String,
    project_dir: String,
    build_file: String,
    display_name: String,
    children: Vec<String>,
}

/// A registered build layout.
struct BuildLayout {
    #[allow(dead_code)]
    build_id: String,
    #[allow(dead_code)]
    root_dir: String,
    #[allow(dead_code)]
    build_file: String,
    #[allow(dead_code)]
    build_name: String,
}

/// Rust-native build layout service.
/// Manages Gradle's multi-project structure, project tree,
/// and settings. Replaces the JVM-side SettingsProcessor.
#[derive(Default)]
pub struct BuildLayoutServiceImpl {
    builds: DashMap<String, BuildLayout>,
    projects: DashMap<String, Project>, // keyed by "build_id:project_path"
}

impl BuildLayoutServiceImpl {
    pub fn new() -> Self {
        Self {
            builds: DashMap::new(),
            projects: DashMap::new(),
        }
    }

    fn project_key(build_id: &str, project_path: &str) -> String {
        format!("{}:{}", build_id, project_path)
    }
}

#[tonic::async_trait]
impl BuildLayoutService for BuildLayoutServiceImpl {
    async fn init_build_layout(
        &self,
        request: Request<InitBuildLayoutRequest>,
    ) -> Result<Response<InitBuildLayoutResponse>, Status> {
        let req = request.into_inner();

        if req.root_dir.is_empty() {
            return Err(Status::invalid_argument("root_dir is required"));
        }

        let build_id = format!("build-{}", uuid::Uuid::new_v4().to_string().split_off(8));

        let layout = BuildLayout {
            build_id: build_id.clone(),
            root_dir: req.root_dir.clone(),
            build_file: req.build_file.clone(),
            build_name: req.build_name.clone(),
        };

        self.builds.insert(build_id.clone(), layout);

        // Register root project
        let root_project = Project {
            project_path: ":".to_string(),
            project_dir: req.root_dir,
            build_file: if req.build_file.is_empty() {
                "build.gradle".to_string()
            } else {
                req.build_file
            },
            display_name: if req.build_name.is_empty() {
                "root project".to_string()
            } else {
                req.build_name
            },
            children: Vec::new(),
        };

        self.projects.insert(
            Self::project_key(&build_id, ":"),
            root_project,
        );

        tracing::info!(
            build_id = %build_id,
            "Build layout initialized"
        );

        Ok(Response::new(InitBuildLayoutResponse {
            build_id,
            initialized: true,
            error_message: String::new(),
        }))
    }

    async fn add_subproject(
        &self,
        request: Request<AddSubprojectRequest>,
    ) -> Result<Response<AddSubprojectResponse>, Status> {
        let req = request.into_inner();

        if !self.builds.contains_key(&req.build_id) {
            return Ok(Response::new(AddSubprojectResponse {
                added: false,
                error_message: format!("Build {} not found", req.build_id),
            }));
        }

        let key = Self::project_key(&req.build_id, &req.project_path);

        if self.projects.contains_key(&key) {
            return Ok(Response::new(AddSubprojectResponse {
                added: false,
                error_message: format!("Project {} already exists", req.project_path),
            }));
        }

        let project = Project {
            project_path: req.project_path.clone(),
            project_dir: req.project_dir.clone(),
            build_file: req.build_file.clone(),
            display_name: req.display_name.clone(),
            children: Vec::new(),
        };

        self.projects.insert(key.clone(), project);

        // Add to parent's children list
        let parent_path = parent_project_path(&req.project_path);
        let parent_key = Self::project_key(&req.build_id, &parent_path);
        if let Some(mut parent) = self.projects.get_mut(&parent_key) {
            if !parent.children.contains(&req.project_path) {
                parent.children.push(req.project_path.clone());
            }
        }

        tracing::debug!(
            build_id = %req.build_id,
            project_path = %req.project_path,
            "Subproject added"
        );

        Ok(Response::new(AddSubprojectResponse {
            added: true,
            error_message: String::new(),
        }))
    }

    async fn get_project_tree(
        &self,
        request: Request<GetProjectTreeRequest>,
    ) -> Result<Response<GetProjectTreeResponse>, Status> {
        let req = request.into_inner();

        if !self.builds.contains_key(&req.build_id) {
            return Err(Status::not_found(format!(
                "Build {} not found",
                req.build_id
            )));
        }

        let mut all_projects = Vec::new();
        let mut root_node = None;

        for entry in self.projects.iter() {
            if !entry.key().starts_with(&format!("{}:", req.build_id)) {
                continue;
            }

            let node = ProjectNode {
                project_path: entry.project_path.clone(),
                project_dir: entry.project_dir.clone(),
                build_file: entry.build_file.clone(),
                display_name: entry.display_name.clone(),
                children: entry.children.clone(),
            };

            if entry.project_path == ":" {
                root_node = Some(node);
            } else {
                all_projects.push(node);
            }
        }

        let root = root_node.unwrap_or(ProjectNode {
            project_path: ":".to_string(),
            project_dir: String::new(),
            build_file: String::new(),
            display_name: "root".to_string(),
            children: Vec::new(),
        });

        Ok(Response::new(GetProjectTreeResponse {
            root: Some(root),
            all_projects,
        }))
    }

    async fn get_build_file_path(
        &self,
        request: Request<GetBuildFilePathRequest>,
    ) -> Result<Response<GetBuildFilePathResponse>, Status> {
        let req = request.into_inner();
        let key = Self::project_key(&req.build_id, &req.project_path);

        if let Some(project) = self.projects.get(&key) {
            Ok(Response::new(GetBuildFilePathResponse {
                build_file_path: project.build_file.clone(),
                found: true,
            }))
        } else {
            Ok(Response::new(GetBuildFilePathResponse {
                build_file_path: String::new(),
                found: false,
            }))
        }
    }

    async fn list_projects(
        &self,
        request: Request<ListProjectsRequest>,
    ) -> Result<Response<ListProjectsResponse>, Status> {
        let req = request.into_inner();

        let prefix = format!("{}:", req.build_id);
        let mut paths = Vec::new();
        let mut dirs = Vec::new();

        for entry in self.projects.iter() {
            if entry.key().starts_with(&prefix) {
                paths.push(entry.project_path.clone());
                dirs.push(entry.project_dir.clone());
            }
        }

        Ok(Response::new(ListProjectsResponse {
            project_paths: paths,
            project_dirs: dirs,
        }))
    }
}

/// Given a project path like ":lib:utils", return ":lib".
fn parent_project_path(path: &str) -> String {
    if path == ":" {
        return ":".to_string();
    }

    // ":app" -> ":"
    // ":lib:utils" -> ":lib"
    let trimmed = path.trim_start_matches(':');
    if let Some(last_colon) = trimmed.rfind(':') {
        format!(":{}", &trimmed[..last_colon])
    } else {
        ":".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_build_layout() {
        let svc = BuildLayoutServiceImpl::new();

        let resp = svc
            .init_build_layout(Request::new(InitBuildLayoutRequest {
                root_dir: "/tmp/project".to_string(),
                settings_file: "/tmp/project/settings.gradle".to_string(),
                build_file: "build.gradle.kts".to_string(),
                build_name: "my-project".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.initialized);
        assert!(!resp.build_id.is_empty());

        // Root project should exist
        let tree = svc
            .get_project_tree(Request::new(GetProjectTreeRequest {
                build_id: resp.build_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(tree.root.as_ref().unwrap().project_path, ":");
    }

    #[tokio::test]
    async fn test_add_subprojects() {
        let svc = BuildLayoutServiceImpl::new();

        let build = svc
            .init_build_layout(Request::new(InitBuildLayoutRequest {
                root_dir: "/tmp/multi".to_string(),
                settings_file: String::new(),
                build_file: "build.gradle".to_string(),
                build_name: "multi".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let build_id = build.build_id;

        svc.add_subproject(Request::new(AddSubprojectRequest {
            build_id: build_id.clone(),
            project_path: ":app".to_string(),
            project_dir: "/tmp/multi/app".to_string(),
            build_file: "/tmp/multi/app/build.gradle".to_string(),
            display_name: "app".to_string(),
        }))
        .await
        .unwrap();

        svc.add_subproject(Request::new(AddSubprojectRequest {
            build_id: build_id.clone(),
            project_path: ":lib".to_string(),
            project_dir: "/tmp/multi/lib".to_string(),
            build_file: "/tmp/multi/lib/build.gradle".to_string(),
            display_name: "lib".to_string(),
        }))
        .await
        .unwrap();

        svc.add_subproject(Request::new(AddSubprojectRequest {
            build_id: build_id.clone(),
            project_path: ":lib:utils".to_string(),
            project_dir: "/tmp/multi/lib/utils".to_string(),
            build_file: "/tmp/multi/lib/utils/build.gradle".to_string(),
            display_name: "lib:utils".to_string(),
        }))
        .await
        .unwrap();

        let tree = svc
            .get_project_tree(Request::new(GetProjectTreeRequest {
                build_id: build_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(tree.all_projects.len(), 3);
        assert_eq!(tree.root.as_ref().unwrap().children.len(), 2); // :app, :lib

        // Verify nesting
        let lib = tree
            .all_projects
            .iter()
            .find(|p| p.project_path == ":lib")
            .unwrap();
        assert_eq!(lib.children.len(), 1);
        assert_eq!(lib.children[0], ":lib:utils");
    }

    #[tokio::test]
    async fn test_duplicate_subproject() {
        let svc = BuildLayoutServiceImpl::new();

        let build = svc
            .init_build_layout(Request::new(InitBuildLayoutRequest {
                root_dir: "/tmp/dup".to_string(),
                settings_file: String::new(),
                build_file: String::new(),
                build_name: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        svc.add_subproject(Request::new(AddSubprojectRequest {
            build_id: build.build_id.clone(),
            project_path: ":app".to_string(),
            project_dir: "/tmp/dup/app".to_string(),
            build_file: String::new(),
            display_name: String::new(),
        }))
        .await
        .unwrap();

        let dup = svc
            .add_subproject(Request::new(AddSubprojectRequest {
                build_id: build.build_id,
                project_path: ":app".to_string(),
                project_dir: "/tmp/dup/app".to_string(),
                build_file: String::new(),
                display_name: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!dup.added);
        assert!(!dup.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_list_projects() {
        let svc = BuildLayoutServiceImpl::new();

        let build = svc
            .init_build_layout(Request::new(InitBuildLayoutRequest {
                root_dir: "/tmp/list".to_string(),
                settings_file: String::new(),
                build_file: String::new(),
                build_name: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        svc.add_subproject(Request::new(AddSubprojectRequest {
            build_id: build.build_id.clone(),
            project_path: ":core".to_string(),
            project_dir: "/tmp/list/core".to_string(),
            build_file: String::new(),
            display_name: String::new(),
        }))
        .await
        .unwrap();

        let list = svc
            .list_projects(Request::new(ListProjectsRequest {
                build_id: build.build_id,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(list.project_paths.len(), 2); // root + :core
        assert!(list.project_paths.contains(&":".to_string()));
        assert!(list.project_paths.contains(&":core".to_string()));
    }

    #[tokio::test]
    async fn test_get_build_file_path() {
        let svc = BuildLayoutServiceImpl::new();

        let build = svc
            .init_build_layout(Request::new(InitBuildLayoutRequest {
                root_dir: "/tmp/bf".to_string(),
                settings_file: String::new(),
                build_file: "build.gradle.kts".to_string(),
                build_name: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        svc.add_subproject(Request::new(AddSubprojectRequest {
            build_id: build.build_id.clone(),
            project_path: ":sub".to_string(),
            project_dir: "/tmp/bf/sub".to_string(),
            build_file: "/tmp/bf/sub/build.gradle.kts".to_string(),
            display_name: String::new(),
        }))
        .await
        .unwrap();

        let resp = svc
            .get_build_file_path(Request::new(GetBuildFilePathRequest {
                build_id: build.build_id.clone(),
                project_path: ":sub".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.build_file_path, "/tmp/bf/sub/build.gradle.kts");

        // Nonexistent
        let resp2 = svc
            .get_build_file_path(Request::new(GetBuildFilePathRequest {
                build_id: build.build_id,
                project_path: ":nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp2.found);
    }

    #[test]
    fn test_parent_project_path() {
        assert_eq!(parent_project_path(":"), ":");
        assert_eq!(parent_project_path(":app"), ":");
        assert_eq!(parent_project_path(":lib:utils"), ":lib");
        assert_eq!(parent_project_path(":a:b:c"), ":a:b");
    }
}
