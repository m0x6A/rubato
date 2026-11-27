//! Leaf infrastructure every slice may use: configuration, telemetry, the storage connection,
//! the shared state holder, cookie signing, the request-error helper, the GitHub port and the
//! admission port that lets `auth` reach tenancy without naming it.
//! The kernel names no slice (ADR 0015, design D2).

pub mod admission;
pub mod config;
pub mod db;
pub mod github;
pub mod http;
pub mod secrets;
pub mod signer;
pub mod speech;
pub mod state;
pub mod telemetry;
pub mod tenancy;
pub mod workspaces;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
