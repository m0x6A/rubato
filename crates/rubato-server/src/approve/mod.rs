//! Slice `approve`: accept a plan in Rubato.
//!
//! - *Accept plan* (ADR 0030): posts `Approved: plan <sha> on <branch>` as the person, through
//!   their user-to-server token, once per (person, issue, plan version), and finds a post GitHub
//!   did not confirm in the next comments pass. Refused after the cutover (ADR 0054).
//! - Approval in Rubato (ADR 0054, plan 122 §D6): a person approves a draft revision; the
//!   approval is a row, not a comment. Refused until the cutover is configured.
//! - The merged list of approvals, comments filtered by the cutover plus Rubato approvals, that
//!   the overview and the composition root's `BranchPrefs` read.
//!
//! Owns tables `plan_approvals` and `rubato_approvals`. Names `plan_drafts` (the version
//! approved), `plans`, `github_poll`, `auth`, `pins` and `launch`; never `companion_link`, which
//! will name it for handoff reports (PR 7): the owner's sessions come from the kernel's cache.

mod draft;
mod handlers;
mod merged;
mod reconcile;
mod records;
mod store;

#[cfg(test)]
mod tests;

pub use draft::approve_draft;
pub use handlers::approve;
pub use merged::{Approvals, approvals, merge, record_of};
pub use reconcile::{OPENING_LIMIT_SECS, reconcile};
pub use records::RubatoApproval;
