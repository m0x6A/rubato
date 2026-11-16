//! `rubato-server` as a library so integration tests build the same router the binary serves.
//!
//! Organised by vertical slices (ADR 0015): one module per capability, a `kernel` of leaf
//! infrastructure that names no slice, and a composition root (`lib.rs`, `app.rs`, `tasks.rs`)
//! that names every slice. Slices: `auth`, `pairing`, `companion_link`, `pins`, `launch`, `map`,
//! `plans`, `github_poll`, `pulls`, `traffic`, `overview`, `web_link`, `webhooks`. `slices.rs` checks the layout.

pub mod app;
pub mod kernel;
pub mod tasks;

pub mod agent_reports;
pub mod approve;
pub mod auth;
pub mod companion_link;
pub mod github_poll;
pub mod import;
pub mod launch;
pub mod layout;
pub mod map;
pub mod map_draft;
pub mod new_issue;
pub mod overview;
pub mod pairing;
pub mod pins;
pub mod plan_drafts;
pub mod plans;
pub mod pulls;
pub mod queue;
pub mod rooms;
pub mod session_log;
pub mod session_messages;
pub mod spend_history;
pub mod traffic;
pub mod voice;
pub mod web_link;
pub mod webhooks;
pub mod workspace_settings;

#[cfg(test)]
mod slices;

pub use kernel::VERSION;
