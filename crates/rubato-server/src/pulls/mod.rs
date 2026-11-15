//! Slice `pulls`: a pull request's view, read on demand and cached (ADR 0034), and the rule that
//! says whether the person may merge it from Rubato (ADR 0033, `rubato_protocol::pulls`). Names
//! `auth` (the person's token), `github_poll` (the stored listing and coverage), `plans` (the plan
//! path and hash) and `overview` (the derived approval, plan 82 Decision 1) through their roots.
//! Owns table `pr_merges`.

mod cache;
mod facts;
mod merge;
mod reconcile;
mod store;
#[cfg(test)]
mod tests;
mod view;

pub use merge::merge;
pub use reconcile::{OPENING_LIMIT_SECS, reconcile};
pub use view::pull_view;
