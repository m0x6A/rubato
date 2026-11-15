//! Slice `session_log`: a launched session's prompts and last replies, from its hooks, visible
//! to the team (ADR 0052, plan 122 design §D2). Owns table `session_log_entries`. Only from
//! companions that announce `hook_allowlist`, only for sessions with a launch.

mod handlers;
mod intake;
mod store;

#[cfg(test)]
mod tests;

pub use handlers::log;
pub use intake::on_hook;
