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
