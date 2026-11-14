//! Leaf infrastructure every slice may use: configuration, telemetry, the storage connection,
//! the shared state holder, cookie signing, the request-error helper and the GitHub port.
//! The kernel names no slice (ADR 0015, design D2).

pub mod config;
pub mod db;
pub mod github;
pub mod http;
pub mod signer;
pub mod state;
pub mod telemetry;
pub mod workspaces;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
