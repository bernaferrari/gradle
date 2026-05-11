//! Transport callbacks used by solver-owned resolution orchestration.

use std::collections::HashSet;

use crate::proto::{DependencyDescriptor, RepositoryDescriptor, ResolvedDependency};

use super::maven_metadata::MavenMetadata;

#[tonic::async_trait]
pub(crate) trait DependencyResolverTransport {
    async fn fetch_pom(
        &self,
        group: &str,
        name: &str,
        version: &str,
        repo: &RepositoryDescriptor,
    ) -> Result<String, String>;

    async fn fetch_gradle_module_metadata(
        &self,
        group: &str,
        name: &str,
        version: &str,
        repo: &RepositoryDescriptor,
    ) -> Result<Option<String>, String>;

    async fn resolve_dependency(
        &self,
        dep: &DependencyDescriptor,
        repos: &[RepositoryDescriptor],
        visited: &mut HashSet<(String, String)>,
        depth: u32,
        inherited_exclusions: &[(String, String)],
        lenient: bool,
    ) -> ResolvedDependency;

    async fn fetch_available_versions(
        &self,
        group: &str,
        name: &str,
        repos: &[RepositoryDescriptor],
    ) -> (Vec<String>, Option<MavenMetadata>);

    async fn resolve_snapshot_version(
        &self,
        group: &str,
        name: &str,
        raw_version: &str,
        repos: &[RepositoryDescriptor],
    ) -> String;

    async fn gradle_module_metadata_artifact_url(
        &self,
        group: &str,
        name: &str,
        version: &str,
        scope: &str,
        repos: &[RepositoryDescriptor],
    ) -> Result<Option<String>, String>;
}
