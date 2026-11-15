//! Slice `sessions`: the registry of sessions on this machine and their discovery from tmux and
//! tmux-ide, and `launches.json`, the launches kept across a restart (plan 122 §D1). Identity
//! comes from the branch (ADR 0006). Other slices add their own `impl Registry` blocks (design
//! D7).

mod assignment;
mod discovery;
mod headless;
mod identity;
mod launches;
mod registry;
mod tasks;

pub(crate) use assignment::PendingLaunch;
pub use assignment::{Assignment, AssignmentView};
pub use headless::{
    Headless, HeadlessConfig, TurnRecord, reap_previous, transcript_path_for, turn_script_path,
};
pub use identity::recheck;
pub use launches::{LaunchRecord, launches_path, load_launches, save_launches_if_changed};
pub use registry::{Entry, Identity, PASTED_KEPT, Registry, identify};
pub use tasks::{reconcile, watch_events};
