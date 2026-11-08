//! `rubato-companion` as a library, so the Gherkin suite drives the same code the binary runs.
//!
//! Behaviour is specified in `features/*.feature` and run with cucumber-rs against a private
//! tmux server (`TMUX_TMPDIR`) and temporary git repositories. No mocks of tmux or git.

pub mod client;
pub mod config;
pub mod daemon;
pub mod gitx;
pub mod hooks;
pub mod launch;
pub mod pairing;
pub mod sessions;
pub mod spend;
pub mod telemetry;
pub mod tmux;
pub mod tmux_ide;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
