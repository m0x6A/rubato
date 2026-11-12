//! Slice `sessions`: the registry of sessions on this machine and their discovery from tmux and
//! tmux-ide. Identity comes from the branch (ADR 0006). Other slices add their own
//! `impl Registry` blocks (design D7).

mod discovery;
mod identity;
mod registry;
mod tasks;

pub use identity::recheck;
pub use registry::{Entry, Identity, Registry, identify};
pub use tasks::{reconcile, watch_events};
