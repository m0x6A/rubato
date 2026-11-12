//! Slice `approve`: accept a plan in Rubato (ADR 0030). Posts `Approved: plan <sha> on
//! <branch>` as the person, through their user-to-server token, once per (person, issue, plan
//! version), and finds a post GitHub did not confirm in the next comments pass. Owns table
//! `plan_approvals`.

mod handlers;
mod reconcile;
mod store;

pub use handlers::approve;
pub use reconcile::{OPENING_LIMIT_SECS, reconcile};
