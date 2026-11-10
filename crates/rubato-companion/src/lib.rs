//! `rubato-companion` as a library, so the Gherkin suite drives the same code the binary runs.
//!
//! Behaviour is specified in `features/*.feature` and run with cucumber-rs against a private
//! tmux server (`TMUX_TMPDIR`) and temporary git repositories. No mocks of tmux or git.
//!
//! Organised by vertical slices (ADR 0015): one module per capability, a `kernel` of leaf
//! infrastructure that names no slice, and a composition root (`lib.rs`, `runtime.rs`) that
//! names every slice. Slices: `pairing`, `server_link`, `sessions`, `hooks`, `launch`, `spend`,
//! `stale`. `slices.rs` checks the layout.

pub mod kernel;
pub mod runtime;

pub mod hooks;
pub mod launch;
pub mod pairing;
pub mod server_link;
pub mod sessions;
pub mod spend;
pub mod stale;

#[cfg(test)]
mod slices;

pub use kernel::VERSION;
