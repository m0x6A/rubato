//! Slice `launch`: handle a `LaunchCommand` (ADR 0004). Refuse anything not addressed to the
//! paired user; one worktree and one tmux session per issue; Claude Code started with
//! `--session-id`, `--settings` (hooks) and the OTel resource attributes.

mod apply;
mod arm;
mod launcher;
mod script;

pub use apply::LiveSession;
pub use arm::{on_command, run};
pub use launcher::{Launcher, Started};
