use dashmap::DashMap;
use tonic::{Request, Response, Status};

use super::scopes::BuildId;
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
    build_id: String,
    root_dir: String,
    settings_file: String,
    build_file: String,
    build_name: String,
}

/// Rust-native build layout service.
/// Manages Gradle's multi-project structure, project tree,
/// and settings. Replaces the JVM-side SettingsProcessor.
#[derive(Default)]
pub struct BuildLayoutServiceImpl {
    builds: DashMap<BuildId, BuildLayout>,
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
        let build_key = BuildId::from(build_id.clone());

        let default_build_file = if req.build_file.is_empty() {
            "build.gradle".to_string()
        } else {
            req.build_file.clone()
        };

        let default_build_name = if req.build_name.is_empty() {
            "root project".to_string()
        } else {
            req.build_name.clone()
        };

        let layout = BuildLayout {
            build_id: build_id.clone(),
            root_dir: req.root_dir.clone(),
            settings_file: req.settings_file.clone(),
            build_file: default_build_file.clone(),
            build_name: default_build_name.clone(),
        };

        self.builds.insert(build_key.clone(), layout);

        // Register root project
        let root_project = Project {
            project_path: ":".to_string(),
            project_dir: req.root_dir,
            build_file: default_build_file,
            display_name: default_build_name,
            children: Vec::new(),
        };

        self.projects.insert(
            Self::project_key(&build_id, ":"),
            root_project,
        );

        tracing::info!(
            build_id = %build_id,
            root_dir = %self.builds.get(&build_key).unwrap().root_dir,
            build_name = %self.builds.get(&build_key).unwrap().build_name,
            settings_file = %self.builds.get(&build_key).unwrap().settings_file,
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
        let build_key = BuildId::from(req.build_id.clone());

        if !self.builds.contains_key(&build_key) {
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

        // Validate that the subproject directory is under the build root
        let layout = self.builds.get(&build_key).unwrap();
        if !req.project_dir.starts_with(&layout.root_dir) {
            return Ok(Response::new(AddSubprojectResponse {
                added: false,
                error_message: format!(
                    "Project directory '{}' is not under build root '{}'",
                    req.project_dir, layout.root_dir
                ),
            }));
        }

        // Use the build-level default build file if none specified for the subproject
        let resolved_build_file = if req.build_file.is_empty() {
            layout.build_file.clone()
        } else {
            req.build_file.clone()
        };

        let resolved_display_name = if req.display_name.is_empty() {
            // Derive a display name from the project path, e.g. ":lib:utils" -> "lib:utils"
            req.project_path.trim_start_matches(':').to_string()
        } else {
            req.display_name.clone()
        };

        let project = Project {
            project_path: req.project_path.clone(),
            project_dir: req.project_dir.clone(),
            build_file: resolved_build_file,
            display_name: resolved_display_name,
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
            build_id = %layout.build_id,
            project_path = %req.project_path,
            project_dir = %req.project_dir,
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
        let build_key = BuildId::from(req.build_id.clone());

        if !self.builds.contains_key(&build_key) {
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

    #[tokio::test]
    async fn test_init_without_root_dir() {
        let svc = BuildLayoutServiceImpl::new();

        let result = svc
            .init_build_layout(Request::new(InitBuildLayoutRequest {
                root_dir: String::new(),
                settings_file: String::new(),
                build_file: String::new(),
                build_name: String::new(),
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_add_subproject_to_nonexistent_build() {
        let svc = BuildLayoutServiceImpl::new();

        let resp = svc
            .add_subproject(Request::new(AddSubprojectRequest {
                build_id: "nonexistent".to_string(),
                project_path: ":app".to_string(),
                project_dir: "/tmp".to_string(),
                build_file: String::new(),
                display_name: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.added);
        assert!(!resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_project_tree_nonexistent_build() {
        let svc = BuildLayoutServiceImpl::new();

        let result = svc
            .get_project_tree(Request::new(GetProjectTreeRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_default_build_file_and_name() {
        let svc = BuildLayoutServiceImpl::new();

        let resp = svc
            .init_build_layout(Request::new(InitBuildLayoutRequest {
                root_dir: "/tmp/default".to_string(),
                settings_file: String::new(),
                build_file: String::new(), // should default to "build.gradle"
                build_name: String::new(), // should default to "root project"
            }))
            .await
            .unwrap()
            .into_inner();

        let tree = svc
            .get_project_tree(Request::new(GetProjectTreeRequest {
                build_id: resp.build_id,
            }))
            .await
            .unwrap()
            .into_inner();

        let root = tree.root.unwrap();
        assert_eq!(root.build_file, "build.gradle");
        assert_eq!(root.display_name, "root project");
    }

    #[tokio::test]
    async fn test_list_projects_empty_build() {
        let svc = BuildLayoutServiceImpl::new();

        let list = svc
            .list_projects(Request::new(ListProjectsRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(list.project_paths.is_empty());
    }

    #[test]
    fn test_parent_project_path() {
        assert_eq!(parent_project_path(":"), ":");
        assert_eq!(parent_project_path(":app"), ":");
        assert_eq!(parent_project_path(":lib:utils"), ":lib");
        assert_eq!(parent_project_path(":a:b:c"), ":a:b");
    }
}
