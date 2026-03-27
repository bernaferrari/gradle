use std::sync::Arc;

use tonic::{Request, Response, Status};

use super::authoritative::AuthoritativeConfig;
use crate::proto::{
    control_service_server::ControlService, GetAuthoritativeModeRequest,
    GetAuthoritativeModeResponse, HandshakeRequest, HandshakeResponse, SetAuthoritativeModeRequest,
    SetAuthoritativeModeResponse, ShutdownRequest, ShutdownResponse, SubsystemAuthStatus,
};
use crate::{PROTOCOL_VERSION, SERVER_VERSION};

#[derive(Clone)]
pub struct ControlServiceImpl {
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
    authoritative_config: Arc<AuthoritativeConfig>,
    jvm_host_socket_path: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl Default for ControlServiceImpl {
    fn default() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(1);
        Self::with_config(tx, Arc::new(AuthoritativeConfig::new()))
    }
}

impl ControlServiceImpl {
    pub fn new(shutdown_tx: tokio::sync::broadcast::Sender<()>) -> Self {
        Self::with_config(shutdown_tx, Arc::new(AuthoritativeConfig::new()))
    }

    pub fn with_config(
        shutdown_tx: tokio::sync::broadcast::Sender<()>,
        authoritative_config: Arc<AuthoritativeConfig>,
    ) -> Self {
        Self {
            shutdown_tx,
            authoritative_config,
            jvm_host_socket_path: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Get the JVM host socket path provided during handshake.
    pub async fn get_jvm_host_socket_path(&self) -> Option<String> {
        self.jvm_host_socket_path.read().await.clone()
    }
}

fn mode_label(authoritative: bool) -> &'static str {
    if authoritative {
        "authoritative"
    } else {
        "shadow"
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

        // Store JVM host socket path for reverse-direction RPC
        if !req.jvm_host_socket_path.is_empty() {
            *self.jvm_host_socket_path.write().await = Some(req.jvm_host_socket_path.clone());
            tracing::info!(jvm_host_socket = %req.jvm_host_socket_path, "JVM host socket path registered");
        }

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
        Ok(Response::new(ShutdownResponse { acknowledged: true }))
    }

    async fn set_authoritative_mode(
        &self,
        request: Request<SetAuthoritativeModeRequest>,
    ) -> Result<Response<SetAuthoritativeModeResponse>, Status> {
        let req = request.into_inner();
        let subsystem = req.subsystem.trim();
        let authoritative = req.authoritative;

        if subsystem.is_empty() || subsystem == "all" {
            self.authoritative_config.set_all(authoritative);
            tracing::info!(mode = %mode_label(authoritative), "Set authoritative mode for all subsystems");
            return Ok(Response::new(SetAuthoritativeModeResponse {
                accepted: true,
                previous_mode: String::new(), // "all" mode doesn't track individual previous state
            }));
        }

        match self
            .authoritative_config
            .set_subsystem(subsystem, authoritative)
        {
            Some(previous) => {
                tracing::info!(
                    subsystem = %subsystem,
                    previous_mode = %mode_label(previous),
                    new_mode = %mode_label(authoritative),
                    "Set authoritative mode"
                );
                Ok(Response::new(SetAuthoritativeModeResponse {
                    accepted: true,
                    previous_mode: mode_label(previous).to_string(),
                }))
            }
            None => {
                let valid = super::authoritative::SubsystemModes::subsystem_names()
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(Status::invalid_argument(format!(
                    "Unknown subsystem '{}'. Valid subsystems: {}",
                    subsystem, valid
                )))
            }
        }
    }

    async fn get_authoritative_mode(
        &self,
        _request: Request<GetAuthoritativeModeRequest>,
    ) -> Result<Response<GetAuthoritativeModeResponse>, Status> {
        let modes = self.authoritative_config.get_modes();
        let subsystems = modes
            .as_pairs()
            .into_iter()
            .map(|(name, authoritative)| SubsystemAuthStatus {
                subsystem: name.to_string(),
                authoritative,
            })
            .collect();

        Ok(Response::new(GetAuthoritativeModeResponse { subsystems }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_service() -> ControlServiceImpl {
        let (tx, _) = tokio::sync::broadcast::channel(1);
        ControlServiceImpl::with_config(tx, Arc::new(AuthoritativeConfig::new()))
    }

    #[tokio::test]
    async fn get_authoritative_mode_defaults() {
        let svc = make_service();
        let resp = svc
            .get_authoritative_mode(Request::new(GetAuthoritativeModeRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(resp.subsystems.len(), 10);
        for s in &resp.subsystems {
            assert!(
                !s.authoritative,
                "{} should be shadow by default",
                s.subsystem
            );
        }
    }

    #[tokio::test]
    async fn set_authoritative_mode_single() {
        let svc = make_service();
        let resp = svc
            .set_authoritative_mode(Request::new(SetAuthoritativeModeRequest {
                subsystem: "hashing".to_string(),
                authoritative: true,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.accepted);
        assert_eq!(resp.previous_mode, "shadow");

        // Verify via get
        let get_resp = svc
            .get_authoritative_mode(Request::new(GetAuthoritativeModeRequest {}))
            .await
            .unwrap()
            .into_inner();
        let hashing = get_resp
            .subsystems
            .iter()
            .find(|s| s.subsystem == "hashing")
            .unwrap();
        assert!(hashing.authoritative);
        let cache_keys = get_resp
            .subsystems
            .iter()
            .find(|s| s.subsystem == "cache_keys")
            .unwrap();
        assert!(!cache_keys.authoritative);
    }

    #[tokio::test]
    async fn set_authoritative_mode_all() {
        let svc = make_service();
        let resp = svc
            .set_authoritative_mode(Request::new(SetAuthoritativeModeRequest {
                subsystem: "all".to_string(),
                authoritative: true,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.accepted);

        let get_resp = svc
            .get_authoritative_mode(Request::new(GetAuthoritativeModeRequest {}))
            .await
            .unwrap()
            .into_inner();
        for s in &get_resp.subsystems {
            assert!(s.authoritative, "{} should be authoritative", s.subsystem);
        }
    }

    #[tokio::test]
    async fn set_authoritative_mode_unknown_subsystem() {
        let svc = make_service();
        let err = svc
            .set_authoritative_mode(Request::new(SetAuthoritativeModeRequest {
                subsystem: "nonexistent".to_string(),
                authoritative: true,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("Unknown subsystem"));
    }

    #[tokio::test]
    async fn set_authoritative_mode_back_to_shadow() {
        let svc = make_service();
        // Set to authoritative
        svc.set_authoritative_mode(Request::new(SetAuthoritativeModeRequest {
            subsystem: "hashing".to_string(),
            authoritative: true,
        }))
        .await
        .unwrap();

        // Set back to shadow
        let resp = svc
            .set_authoritative_mode(Request::new(SetAuthoritativeModeRequest {
                subsystem: "hashing".to_string(),
                authoritative: false,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.accepted);
        assert_eq!(resp.previous_mode, "authoritative");
    }

    #[tokio::test]
    async fn handshake_stores_jvm_host_socket_path() {
        let svc = make_service();
        assert!(svc.get_jvm_host_socket_path().await.is_none());

        // Handshake with JVM host socket path
        let resp = svc
            .handshake(Request::new(HandshakeRequest {
                client_version: "test".to_string(),
                protocol_version: PROTOCOL_VERSION.to_string(),
                client_pid: 1234,
                jvm_host_socket_path: "/tmp/jvm-host.sock".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.accepted);

        let path = svc.get_jvm_host_socket_path().await;
        assert_eq!(path.as_deref(), Some("/tmp/jvm-host.sock"));
    }

    #[tokio::test]
    async fn handshake_without_jvm_host_path_stays_none() {
        let svc = make_service();

        svc.handshake(Request::new(HandshakeRequest {
            client_version: "test".to_string(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            client_pid: 1234,
            jvm_host_socket_path: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

        assert!(svc.get_jvm_host_socket_path().await.is_none());
    }
}
