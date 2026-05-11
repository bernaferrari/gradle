//! Transport callbacks used by solver-owned resolution orchestration.

use std::collections::HashSet;

use crate::proto::{DependencyDescriptor, RepositoryDescriptor, ResolvedDependency};

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
}
