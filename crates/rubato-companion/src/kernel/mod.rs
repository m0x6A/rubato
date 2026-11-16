//! Leaf infrastructure every slice may use: configuration and state files, telemetry, and the
//! adapters to tmux, git, `ps` and `claude`. The kernel names no slice (ADR 0015, design D2).

pub mod claude;
pub mod config;
pub mod gitx;
pub mod proc;
pub mod ps;
pub mod skill;
pub mod telemetry;
pub mod tmux;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
