//! Slice `fleet`: a person's own lead agent and the workers it launched (ADR 0072, plan 134),
//! behind `RUBATO_ORCHESTRATOR`, off by default. A lead's fleet tool call arrives as one
//! `FleetRequest` on its companion's websocket and gets one `FleetResult`. The request is
//! claimed before it acts (so a repeat never acts twice), then checked: the lead is a launch of
//! this pairing with `role: lead`, its owner is the pairing's user, and a message or stop goes
//! only to a worker this lead launched for that owner. `other_owner` and `identity` page.
//!
//! Owns table `fleet_requests`. Names `launch` (the launch path and the launches it reads),
//! `session_messages` (a lead's chat and stop, and notices), `session_log` (the last replies)
//! and `plan_drafts` (someone else's draft). Named by `companion_link` (the request, its replay,
//! the snapshot that makes notices) and `overview` (the leads).

mod handle;
mod notices;
mod store;

#[cfg(test)]
mod tests;

pub use handle::{live_workers, on_replay, on_request};
pub use notices::{AUTHOR as NOTICE_AUTHOR, leads, on_sessions};
