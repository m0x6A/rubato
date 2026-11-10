//! Slice `hooks`: the loopback hook receiver (ADR 0013), the hooks template, and what a hook
//! event does to the registry. The source of trusted session state.

mod apply;
mod arm;
mod server;
mod template;

pub use arm::{HookOutcome, count_dropped, on_event, seed_forwarded};
pub use server::{HookServer, new_token};
pub use template::{HOOKS_TEMPLATE, render_hooks};
