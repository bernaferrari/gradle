//! Transport callbacks used by solver-owned resolution orchestration.

use crate::proto::RepositoryDescriptor;

#[tonic::async_trait]
pub(crate) trait DependencyResolverTransport {
    async fn fetch_pom(
        &self,
        group: &str,
        name: &str,
        version: &str,
        repo: &RepositoryDescriptor,
    ) -> Result<String, String>;
}
