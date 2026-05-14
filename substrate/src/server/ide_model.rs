//! IDE model service for Gradle Tooling API compatibility.
//!
//! Provides project structure, task list, and dependency information
//! for IDEs (IntelliJ, VS Code) via gRPC.

use dashmap::DashMap;
use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::proto::{
    ide_model_service_server::IdeModelService, GetDependenciesRequest,
    GetDependenciesResponse, GetProjectModelRequest, GetProjectModelResponse,
    GetTaskDescriptionsRequest, GetTaskDescriptionsResponse, IdeDependency,
    IdeProject, IdeSourceSet, IdeTaskDescription,
};

/// Cached IDE project model entry.
#[derive(Debug, Clone)]
pub struct CachedIdeProject {
    pub name: String,
    pub path: String,
    pub description: String,
    pub tasks: Vec<String>,
    pub source_sets: Vec<CachedSourceSet>,
}

impl From<&CachedIdeProject> for IdeProject {
    fn from(cached: &CachedIdeProject) -> Self {
        Self {
            name: cached.name.clone(),
            path: cached.path.clone(),
            description: cached.description.clone(),
            source_sets: cached
                .source_sets
                .iter()
                .map(|s| IdeSourceSet {
                    name: s.name.clone(),
                    source_dirs: s.source_dirs.clone(),
                    resource_dirs: s.resource_dirs.clone(),
                    output_dirs: s.output_dirs.clone(),
                })
                .collect(),
            tasks: cached.tasks.clone(),
        }
    }
}

/// Cached source set information.
#[derive(Debug, Clone)]
pub struct CachedSourceSet {
    pub name: String,
    pub source_dirs: Vec<String>,
    pub resource_dirs: Vec<String>,
    pub output_dirs: Vec<String>,
}

/// Cached dependency entry.
#[derive(Debug, Clone)]
pub struct CachedIdeDependency {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub configuration: String,
}

impl From<&CachedIdeDependency> for IdeDependency {
    fn from(cached: &CachedIdeDependency) -> Self {
        Self {
            group: cached.group.clone(),
            artifact: cached.artifact.clone(),
            version: cached.version.clone(),
            configuration: cached.configuration.clone(),
        }
    }
}

/// In-memory IDE model cache.
#[derive(Default, Clone)]
pub struct IdeModelServiceImpl {
    projects: Arc<DashMap<String, CachedIdeProject>>,
    tasks: Arc<DashMap<String, Vec<IdeTaskDescription>>>,
    dependencies: Arc<DashMap<(String, String), Vec<CachedIdeDependency>>>,
}

impl IdeModelServiceImpl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or update a project in the cache.
    pub fn put_project(&self, project: CachedIdeProject) {
        self.projects.insert(project.path.clone(), project);
    }

    /// Insert or update task descriptions for a project.
    pub fn put_tasks(&self, project_path: String, tasks: Vec<IdeTaskDescription>) {
        self.tasks.insert(project_path, tasks);
    }

    /// Insert or update dependencies for a project+configuration.
    pub fn put_dependencies(
        &self,
        project_path: String,
        configuration: String,
        deps: Vec<CachedIdeDependency>,
    ) {
        self.dependencies.insert((project_path, configuration), deps);
    }
}

#[tonic::async_trait]
impl IdeModelService for IdeModelServiceImpl {
    async fn get_project_model(
        &self,
        request: Request<GetProjectModelRequest>,
    ) -> Result<Response<GetProjectModelResponse>, Status> {
        let req = request.into_inner();
        let project_path = req.project_path;

        let mut projects = Vec::new();

        if let Some(cached) = self.projects.get(&project_path) {
            projects.push(IdeProject::from(cached.value()));
        }

        Ok(Response::new(GetProjectModelResponse {
            projects,
            error_message: String::new(),
        }))
    }

    async fn get_task_descriptions(
        &self,
        request: Request<GetTaskDescriptionsRequest>,
    ) -> Result<Response<GetTaskDescriptionsResponse>, Status> {
        let req = request.into_inner();
        let project_path = req.project_path;

        let tasks = self
            .tasks
            .get(&project_path)
            .map(|v| v.clone())
            .unwrap_or_default();

        Ok(Response::new(GetTaskDescriptionsResponse { tasks }))
    }

    async fn get_dependencies(
        &self,
        request: Request<GetDependenciesRequest>,
    ) -> Result<Response<GetDependenciesResponse>, Status> {
        let req = request.into_inner();
        let project_path = req.project_path;
        let configuration = req.configuration;

        let deps = self
            .dependencies
            .get(&(project_path.clone(), configuration.clone()))
            .map(|v| v.iter().map(IdeDependency::from).collect())
            .unwrap_or_default();

        Ok(Response::new(GetDependenciesResponse { dependencies: deps }))
    }
}
