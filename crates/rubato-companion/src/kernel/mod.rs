//! Leaf infrastructure every slice may use: configuration and state files, telemetry, and the
//! adapters to tmux, tmux-ide and git. The kernel names no slice (ADR 0015, design D2).

pub mod config;
pub mod gitx;
pub mod telemetry;
pub mod tmux;
pub mod tmux_ide;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
