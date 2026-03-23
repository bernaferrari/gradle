use std::sync::Arc;

use tonic::Status;

use super::jvm_host::JvmHostClient;
use crate::proto::{
    GetBuildEnvironmentResponse, GetBuildModelResponse, ResolveConfigResponse,
};

/// Shared bridge to the JVM host, allowing multiple services to call back
/// into the Gradle JVM. Uses interior mutability so the client can be set
/// after the initial handshake.
pub struct JvmHostBridge {
    client: tokio::sync::Mutex<Option<JvmHostClient>>,
}

impl JvmHostBridge {
    /// Create a new (empty) bridge. Call `set_client` once the JVM host
    /// socket path is known and the connection is established.
    pub fn new() -> Self {
        Self {
            client: tokio::sync::Mutex::new(None),
        }
    }

    /// Set the JVM host client. Called once after the daemon connects
    /// to the JVM host socket.
    pub async fn set_client(&self, client: JvmHostClient) {
        *self.client.lock().await = Some(client);
    }

    /// Check whether the JVM host client is connected.
    pub async fn is_connected(&self) -> bool {
        self.client.lock().await.is_some()
    }

    /// Get the JVM build environment (Java version, OS, memory, etc.).
    /// Returns `None` if the JVM host is not connected.
    pub async fn get_build_environment(
        &self,
    ) -> Result<Option<GetBuildEnvironmentResponse>, Status> {
        let mut guard = self.client.lock().await;
        let client = match guard.as_mut() {
            Some(c) => c,
            None => return Ok(None),
        };
        let response = client.get_build_environment().await?;
        Ok(Some(response))
    }

    /// Get the build model from the JVM (projects, subprojects, build files).
    /// Returns `None` if the JVM host is not connected.
    pub async fn get_build_model(
        &self,
        build_id: &str,
    ) -> Result<Option<GetBuildModelResponse>, Status> {
        let mut guard = self.client.lock().await;
        let client = match guard.as_mut() {
            Some(c) => c,
            None => return Ok(None),
        };
        let response = client.get_build_model(build_id).await?;
        Ok(Some(response))
    }

    /// Resolve a dependency configuration via the JVM.
    /// Returns `None` if the JVM host is not connected.
    pub async fn resolve_configuration(
        &self,
        build_id: &str,
        configuration_name: &str,
        project_path: &str,
    ) -> Result<Option<ResolveConfigResponse>, Status> {
        let mut guard = self.client.lock().await;
        let client = match guard.as_mut() {
            Some(c) => c,
            None => return Ok(None),
        };
        let response = client
            .resolve_configuration(build_id, configuration_name, project_path)
            .await?;
        Ok(Some(response))
    }
}

impl Default for JvmHostBridge {
    fn default() -> Self {
        Self::new()
    }
}

/// A shared reference to the JVM host bridge.
pub type SharedJvmHostBridge = Arc<JvmHostBridge>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bridge_not_connected_initially() {
        let bridge = JvmHostBridge::new();
        assert!(!bridge.is_connected().await);
    }

    #[tokio::test]
    async fn test_bridge_returns_none_when_not_connected() {
        let bridge = JvmHostBridge::new();

        let env = bridge.get_build_environment().await.unwrap();
        assert!(env.is_none());

        let model = bridge.get_build_model("build-1").await.unwrap();
        assert!(model.is_none());

        let resolved = bridge
            .resolve_configuration("build-1", "compileClasspath", ":app")
            .await
            .unwrap();
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn test_bridge_default() {
        let bridge = JvmHostBridge::default();
        assert!(!bridge.is_connected().await);
    }

    #[tokio::test]
    async fn test_shared_bridge_type() {
        let bridge: SharedJvmHostBridge = Arc::new(JvmHostBridge::new());
        assert!(!bridge.is_connected().await);
    }
}
