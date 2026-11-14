//! Slice `rooms`: the plan's room (ADR 0057, plan 122 design §D8). A room belongs to a plan,
//! keyed like its draft by `(repo, number, branch)`. People post and everyone who may log in
//! reads it; Rubato posts one system notice per pair of issue and seam when this plan and another
//! open plan or settled draft touch the same seam. Agents are never in a room uninvited; calling
//! one in is PR 11. Owns table `room_messages`. Names `plans` and `plan_drafts` (does a plan exist,
//! which seams it touches), `github_poll` (merged plans are not open) and `auth`.

mod handlers;
mod notices;
mod store;

#[cfg(test)]
mod tests;

pub use handlers::{post, room};
pub use notices::{SYSTEM_AUTHOR, post_seam_notices};
