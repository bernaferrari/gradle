pub mod error;
pub mod server;

pub mod proto {
    tonic::include_proto!("gradle.substrate.v1");
}

pub const PROTOCOL_VERSION: &str = "1.0.0";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
