use tonic::{Request, Response, Status};

use crate::{PROTOCOL_VERSION, SERVER_VERSION};
use crate::proto::{
    control_service_server::ControlService, HandshakeRequest, HandshakeResponse,
    ShutdownRequest, ShutdownResponse,
};

pub struct ControlServiceImpl {
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
}

impl ControlServiceImpl {
    pub fn new(shutdown_tx: tokio::sync::broadcast::Sender<()>) -> Self {
        Self { shutdown_tx }
    }
}

#[tonic::async_trait]
impl ControlService for ControlServiceImpl {
    async fn handshake(
        &self,
        request: Request<HandshakeRequest>,
    ) -> Result<Response<HandshakeResponse>, Status> {
        let req = request.into_inner();

        if req.protocol_version != PROTOCOL_VERSION {
            return Ok(Response::new(HandshakeResponse {
                accepted: false,
                server_version: SERVER_VERSION.to_string(),
                protocol_version: PROTOCOL_VERSION.to_string(),
                error_message: format!(
                    "Protocol version mismatch: client={}, server={}",
                    req.protocol_version, PROTOCOL_VERSION
                ),
            }));
        }

        tracing::info!(
            client_version = %req.client_version,
            client_pid = req.client_pid,
            "Handshake successful"
        );

        Ok(Response::new(HandshakeResponse {
            accepted: true,
            server_version: SERVER_VERSION.to_string(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            error_message: String::new(),
        }))
    }

    async fn shutdown(
        &self,
        _request: Request<ShutdownRequest>,
    ) -> Result<Response<ShutdownResponse>, Status> {
        tracing::info!("Shutdown requested");
        let _ = self.shutdown_tx.send(());
        Ok(Response::new(ShutdownResponse {
            acknowledged: true,
        }))
    }
}
