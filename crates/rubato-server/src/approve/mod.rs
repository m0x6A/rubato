//! Slice `approve`: accept a plan in Rubato, and follow the approval to its branch.
//!
//! - *Accept plan* (ADR 0030): posts `Approved: plan <sha> on <branch>` as the person, through
//!   their user-to-server token, once per (person, issue, plan version), and finds a post GitHub
//!   did not confirm in the next comments pass. Refused after the cutover (ADR 0054).
//! - Approval in Rubato (ADR 0054, plan 122 §D6): a person approves a draft revision; the
//!   approval is a row, not a comment. Refused until the cutover is configured. A new approval
//!   queues `plan_approved` for the owner's live launched session (ADR 0055).
//! - The handoff (ADR 0055, §D7): the companion's handoff report recorded on the approval; the
//!   plan at the branch head compared with the record each tick; the link comment posted once as
//!   the approver; the findings and gauges of a handoff that did not happen.
//! - The merged list of approvals, comments filtered by the cutover plus Rubato approvals, that
//!   the overview and the composition root's `BranchPrefs` read.
//!
//! Owns tables `plan_approvals`, `rubato_approvals` and `plan_links`. Names `plan_drafts` (the
//! version approved), `session_messages` (the queue `plan_approved` goes into), `plans`,
//! `github_poll`, `auth`, `pins` and `launch`; never `companion_link`, which names it for handoff
//! reports: the owner's sessions come from the kernel's cache.

mod deliver;
mod draft;
mod handlers;
mod handoff;
mod link;
mod links;
mod merged;
mod reconcile;
mod records;
mod store;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod handoff_tests;

pub use deliver::{approval_url, deliver_to_launch, plan_approved};
pub use draft::approve_draft;
pub use handlers::approve;
pub use handoff::on_handoff_report;
pub use link::{
    Assessment, HANDOFF_OVERDUE_SECS, LINK_WAITING_SECS, LinkFacts, assess, keys_match, link_body,
    link_findings, reconcile_links,
};
pub use merged::{Approvals, approvals, merge, record_of};
pub use reconcile::{OPENING_LIMIT_SECS, reconcile};
pub use records::{HandoffFacts, RubatoApproval};
