//! Slice `rooms`: the plan's room (ADR 0057, plan 122 design §D8). A room belongs to a plan,
//! keyed like its draft by `(repo, number, branch)`. People post and everyone who may log in
//! reads it; Rubato posts one system notice per pair of issue and seam when this plan and another
//! open plan or settled draft touch the same seam. Agents are never in a room uninvited: the owner
//! calls theirs in, or accepts a teammate's request; an addressed message makes a turn for it.
//!
//! Owns tables `room_messages`, `room_agents` and `room_requests`. Names `plans` and
//! `plan_drafts` (does a plan exist, is it on the branch, which seams it touches), `github_poll`
//! (merged plans are not open), `launch` (whose launch an agent is), `session_messages` (turns are
//! queued there), `session_log` (a session-mode reply is a `Stop` of the prompt that carried the
//! turn), `spend_history` (a fork turn's cost) and `auth`. `companion_link` names it for hooks and
//! replies.

mod agents;
mod callin;
mod handlers;
mod notices;
mod store;
mod turns;

#[cfg(test)]
mod tests;

pub use agents::{leave_person, leave_repos};
pub use callin::{UNKNOWN_AGENT, UNKNOWN_REQUEST, answer, call_in, dismiss, sweep_agents};
pub use handlers::{
    PostRefused, PostSource, Posted, agents_present, has_plan, message, post, post_as, room,
    turn_was_spoken,
};
pub use notices::{SYSTEM_AUTHOR, post_seam_notices};
pub use turns::{ASKED_ALOUD, TURN_LIMIT_TEXT, on_hook, on_room_reply};
