use std::io;

use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};

use crate::proto::{
    jvm_host_service_client::JvmHostServiceClient, GetBuildEnvironmentRequest,
    GetBuildEnvironmentResponse, GetBuildModelRequest, GetBuildModelResponse,
    ResolveConfigRequest, ResolveConfigResponse, EvaluateScriptRequest, EvaluateScriptResponse,
};

/// Client for calling back into the JVM via the JvmHostService.
/// Used by the Rust daemon to evaluate scripts, access build model, and resolve configurations.
pub struct JvmHostClient {
    client: JvmHostServiceClient<Channel>,
}

impl JvmHostClient {
    /// Connect to the JVM host gRPC server over a Unix domain socket.
    pub async fn connect(socket_path: &str) -> Result<Self, tonic::transport::Error> {
        let path = socket_path.to_string();
        let channel = Endpoint::from_shared("http://localhost".to_string())?
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let path = path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(&path).await?;
                    let io = hyper_util::rt::TokioIo::new(stream);
                    Ok::<_, io::Error>(io)
                }
            }))
            .await?;
        Ok(Self {
            client: JvmHostServiceClient::new(channel),
        })
    }

    /// Query the JVM for build environment information (Java version, OS, memory, etc.).
    pub async fn get_build_environment(
        &mut self,
    ) -> Result<GetBuildEnvironmentResponse, Status> {
        let response = self
            .client
            .get_build_environment(Request::new(GetBuildEnvironmentRequest {}))
            .await?;
        Ok(response.into_inner())
    }

    /// Request the JVM to evaluate a build script.
    pub async fn evaluate_script(
        &mut self,
        script_path: &str,
        script_content: &str,
        script_type: &str,
    ) -> Result<EvaluateScriptResponse, Status> {
        let request = Request::new(EvaluateScriptRequest {
            script_path: script_path.to_string(),
            script_content: script_content.to_string(),
            script_type: script_type.to_string(),
            extra_properties: Default::default(),
        });
        let response = self.client.evaluate_script(request).await?;
        Ok(response.into_inner())
    }

    /// Request the JVM for the build model (projects, subprojects, build files).
    pub async fn get_build_model(
        &mut self,
        build_id: &str,
    ) -> Result<GetBuildModelResponse, Status> {
        let request = Request::new(GetBuildModelRequest {
            build_id: build_id.to_string(),
        });
        let response = self.client.get_build_model(request).await?;
        Ok(response.into_inner())
    }

    /// Request the JVM to resolve a dependency configuration.
    pub async fn resolve_configuration(
        &mut self,
        build_id: &str,
        configuration_name: &str,
        project_path: &str,
    ) -> Result<ResolveConfigResponse, Status> {
        let request = Request::new(ResolveConfigRequest {
            build_id: build_id.to_string(),
            configuration_name: configuration_name.to_string(),
            project_path: project_path.to_string(),
        });
        let response = self.client.resolve_configuration(request).await?;
        Ok(response.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::proto::jvm_host_service_server::{JvmHostService, JvmHostServiceServer};
    use tokio::net::UnixListener;
    use tonic::transport::Server;
    use tonic::Response;

    /// Mock JVM host service that returns fixed responses.
    struct MockJvmHostService;

    #[tonic::async_trait]
    impl JvmHostService for MockJvmHostService {
        async fn evaluate_script(
            &self,
            request: tonic::Request<crate::proto::EvaluateScriptRequest>,
        ) -> Result<Response<crate::proto::EvaluateScriptResponse>, tonic::Status> {
            let req = request.into_inner();
            let content = &req.script_content;
            let mut plugins = Vec::new();
            let mut order = 1u32;

            // Simple mock: extract id("...") patterns using string scanning
            let mut search_from = 0;
            while search_from < content.len() {
                if let Some(pos) = content[search_from..].find("id(\"") {
                    let abs_pos = search_from + pos + 4; // after id("
                    if let Some(end) = content[abs_pos..].find("\")") {
                        let plugin_id = &content[abs_pos..abs_pos + end];
                        plugins.push(crate::proto::AppliedPlugin {
                            plugin_id: plugin_id.to_string(),
                            apply_order: order.to_string(),
                        });
                        order += 1;
                        search_from = abs_pos + end + 1;
                    } else {
                        break;
                    }
                } else if let Some(pos) = content[search_from..].find("apply plugin: \"") {
                    let abs_pos = search_from + pos + 15; // after apply plugin: "
                    if let Some(end) = content[abs_pos..].find("\"") {
                        let plugin_id = &content[abs_pos..abs_pos + end];
                        plugins.push(crate::proto::AppliedPlugin {
                            plugin_id: plugin_id.to_string(),
                            apply_order: order.to_string(),
                        });
                        order += 1;
                        search_from = abs_pos + end + 1;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            Ok(Response::new(crate::proto::EvaluateScriptResponse {
                success: true,
                error_message: String::new(),
                applied_plugins: plugins,
            }))
        }

        async fn get_build_model(
            &self,
            request: tonic::Request<crate::proto::GetBuildModelRequest>,
        ) -> Result<Response<crate::proto::GetBuildModelResponse>, tonic::Status> {
            let req = request.into_inner();
            let response = crate::proto::GetBuildModelResponse {
                projects: vec![
                    crate::proto::ProjectModel {
                        path: ":".to_string(),
                        name: "root".to_string(),
                        build_file: "/build.gradle.kts".to_string(),
                        subprojects: vec![":app".to_string(), ":lib".to_string()],
                    },
                    crate::proto::ProjectModel {
                        path: ":app".to_string(),
                        name: "app".to_string(),
                        build_file: "/app/build.gradle.kts".to_string(),
                        subprojects: vec![],
                    },
                    crate::proto::ProjectModel {
                        path: ":lib".to_string(),
                        name: "lib".to_string(),
                        build_file: "/lib/build.gradle".to_string(),
                        subprojects: vec![],
                    },
                ],
            };
            tracing::debug!(build_id = %req.build_id, "Mock: returning build model");
            Ok(Response::new(response))
        }

        async fn resolve_configuration(
            &self,
            request: tonic::Request<crate::proto::ResolveConfigRequest>,
        ) -> Result<Response<crate::proto::ResolveConfigResponse>, tonic::Status> {
            let req = request.into_inner();
            let response = crate::proto::ResolveConfigResponse {
                success: true,
                artifacts: vec![
                    crate::proto::ResolvedArtifact {
                        group: "org.springframework".to_string(),
                        name: "spring-core".to_string(),
                        version: "6.1.0".to_string(),
                        configuration: req.configuration_name.clone(),
                    },
                    crate::proto::ResolvedArtifact {
                        group: "org.slf4j".to_string(),
                        name: "slf4j-api".to_string(),
                        version: "2.0.9".to_string(),
                        configuration: req.configuration_name.clone(),
                    },
                ],
                error_message: String::new(),
            };
            Ok(Response::new(response))
        }

        async fn get_build_environment(
            &self,
            _request: tonic::Request<crate::proto::GetBuildEnvironmentRequest>,
        ) -> Result<Response<crate::proto::GetBuildEnvironmentResponse>, tonic::Status> {
            let mut props = HashMap::new();
            props.insert("java.vm.name".to_string(), "OpenJDK 64-Bit Server VM".to_string());
            props.insert("user.timezone".to_string(), "UTC".to_string());
            let response = crate::proto::GetBuildEnvironmentResponse {
                java_version: "17.0.9".to_string(),
                java_home: "/usr/lib/jvm/java-17".to_string(),
                gradle_version: "8.5".to_string(),
                os_name: "Linux".to_string(),
                os_arch: "amd64".to_string(),
                available_processors: 8,
                max_memory_bytes: 4_294_967_296,
                system_properties: props,
            };
            Ok(Response::new(response))
        }
    }

    /// Spawn a mock JVM host server on a temp Unix socket, return the socket path and temp dir.
    /// The TempDir must be held for the lifetime of the server.
    async fn spawn_mock_server() -> (String, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("jvm-host-test.sock");

        // Clean up stale socket
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).unwrap();
        let path_str = socket_path.to_string_lossy().to_string();

        tokio::spawn(async move {
            Server::builder()
                .add_service(JvmHostServiceServer::new(MockJvmHostService))
                .serve_with_incoming(tokio_stream::wrappers::UnixListenerStream::new(listener))
                .await
                .unwrap();
        });

        // Give the server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        (path_str, dir)
    }

    #[tokio::test]
    async fn test_get_build_environment() {
        let _guard = tracing_subscriber::fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();

        let (socket_path, _dir) = spawn_mock_server().await;
        let mut client = JvmHostClient::connect(&socket_path).await.unwrap();

        let env = client.get_build_environment().await.unwrap();

        assert_eq!(env.java_version, "17.0.9");
        assert_eq!(env.java_home, "/usr/lib/jvm/java-17");
        assert_eq!(env.gradle_version, "8.5");
        assert_eq!(env.os_name, "Linux");
        assert_eq!(env.os_arch, "amd64");
        assert_eq!(env.available_processors, 8);
        assert_eq!(env.max_memory_bytes, 4_294_967_296);
        assert_eq!(
            env.system_properties.get("java.vm.name").unwrap(),
            "OpenJDK 64-Bit Server VM"
        );
    }

    #[tokio::test]
    async fn test_get_build_model() {
        let (socket_path, _dir) = spawn_mock_server().await;
        let mut client = JvmHostClient::connect(&socket_path).await.unwrap();

        let model = client.get_build_model("build-123").await.unwrap();

        assert_eq!(model.projects.len(), 3);
        assert_eq!(model.projects[0].path, ":");
        assert_eq!(model.projects[0].name, "root");
        assert_eq!(model.projects[0].subprojects.len(), 2);
        assert_eq!(model.projects[0].subprojects[0], ":app");
        assert_eq!(model.projects[1].path, ":app");
        assert_eq!(model.projects[1].build_file, "/app/build.gradle.kts");
        assert_eq!(model.projects[2].build_file, "/lib/build.gradle");
    }

    #[tokio::test]
    async fn test_resolve_configuration() {
        let (socket_path, _dir) = spawn_mock_server().await;
        let mut client = JvmHostClient::connect(&socket_path).await.unwrap();

        let result = client
            .resolve_configuration("build-123", "compileClasspath", ":app")
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.artifacts.len(), 2);
        assert_eq!(result.artifacts[0].group, "org.springframework");
        assert_eq!(result.artifacts[0].name, "spring-core");
        assert_eq!(result.artifacts[0].version, "6.1.0");
        assert_eq!(result.artifacts[0].configuration, "compileClasspath");
        assert_eq!(result.artifacts[1].group, "org.slf4j");
        assert_eq!(result.error_message, "");
    }

    #[tokio::test]
    async fn test_connect_to_nonexistent_socket_fails() {
        let result = JvmHostClient::connect("/tmp/nonexistent-jvm-host-socket-xyz.sock").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_evaluate_script_extracts_plugins() {
        let (socket_path, _dir) = spawn_mock_server().await;
        let mut client = JvmHostClient::connect(&socket_path).await.unwrap();

        let script = r#"
            plugins {
                id("java")
                id("org.springframework.boot") version "3.2.0"
            }
        "#;
        let result = client
            .evaluate_script("build.gradle.kts", script, "kotlin")
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.error_message, "");
        assert_eq!(result.applied_plugins.len(), 2);
        assert_eq!(result.applied_plugins[0].plugin_id, "java");
        assert_eq!(result.applied_plugins[1].plugin_id, "org.springframework.boot");
        assert_eq!(result.applied_plugins[0].apply_order, "1");
        assert_eq!(result.applied_plugins[1].apply_order, "2");
    }

    #[tokio::test]
    async fn test_evaluate_script_legacy_groovy_plugins() {
        let (socket_path, _dir) = spawn_mock_server().await;
        let mut client = JvmHostClient::connect(&socket_path).await.unwrap();

        let script = r#"
            apply plugin: "java"
            apply plugin: "idea"
        "#;
        let result = client
            .evaluate_script("build.gradle", script, "groovy")
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.applied_plugins.len(), 2);
        assert_eq!(result.applied_plugins[0].plugin_id, "java");
        assert_eq!(result.applied_plugins[1].plugin_id, "idea");
    }

    #[tokio::test]
    async fn test_evaluate_script_empty_returns_no_plugins() {
        let (socket_path, _dir) = spawn_mock_server().await;
        let mut client = JvmHostClient::connect(&socket_path).await.unwrap();

        let result = client
            .evaluate_script("build.gradle.kts", "", "kotlin")
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.applied_plugins.is_empty());
    }

    #[tokio::test]
    async fn test_multiple_sequential_calls() {
        let (socket_path, _dir) = spawn_mock_server().await;
        let mut client = JvmHostClient::connect(&socket_path).await.unwrap();

        // First call: get environment
        let env = client.get_build_environment().await.unwrap();
        assert_eq!(env.java_version, "17.0.9");

        // Second call: get build model
        let model = client.get_build_model("build-1").await.unwrap();
        assert_eq!(model.projects.len(), 3);

        // Third call: resolve configuration
        let result = client
            .resolve_configuration("build-1", "runtimeClasspath", ":app")
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.artifacts.len(), 2);
    }
}
