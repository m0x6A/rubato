//! Slice `hooks`: the loopback hook receiver (ADR 0013), the hooks template, and what a hook
//! event does to the registry, and the `/handoff` route that forwards `plan.ts handoff` reports
//! (ADR 0055). The source of trusted session state.

mod apply;
mod arm;
mod handoff;
mod server;
mod template;
mod wire;

pub use arm::{HookOutcome, count_dropped, on_event, seed_forwarded};
pub use handoff::{Forwarded, on_handoff};
pub use server::{HANDOFF_ROUTE, HookServer, LAUNCH_HEADER, new_token};
pub use template::{HOOKS_TEMPLATE, render_hooks};
pub use wire::PAYLOAD_ALLOWLIST;
