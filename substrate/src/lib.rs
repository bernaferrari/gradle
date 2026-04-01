//! Gradle Substrate Daemon — Rust execution substrate for Gradle builds.
//!
//! This crate provides a gRPC daemon that progressively replaces Java subsystems
//! in the Gradle build toolchain. It communicates with the JVM over Unix domain
//! sockets and supports 38 services across 11 authoritative subsystems.

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod client;
#[allow(missing_docs)]
pub mod error;
#[allow(missing_docs)]
pub mod server;
pub mod unsafe_audit;

// Use jemalloc in test builds to prevent runaway RSS.
// macOS system malloc retains freed pages, causing ~150GB RSS
// when 832+ tests run sequentially in one process.
#[cfg(test)]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[allow(missing_docs)]
pub mod proto {
    tonic::include_proto!("gradle.substrate.v1");
}

#[allow(missing_docs)]
pub const PROTOCOL_VERSION: &str = "1.0.0";
#[allow(missing_docs)]
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
