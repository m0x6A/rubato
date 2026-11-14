//! Accept a plan in Rubato, on the `web-server` seam.
//!
//! - *Accept plan* (ADR 0030): the server posts `Approved: plan <sha> on <branch>` as the
//!   person, through their GitHub App user-to-server token; approval is that comment (ADR 0027),
//!   counted only while the plan fits (ADR 0029). Refused after the cutover (ADR 0054).
//! - Approval in Rubato (ADR 0054, plan 122 §D6): a person approves one revision of a draft as
//!   themselves; the approval is a `rubato_approvals` row, not a comment. Refused until the
//!   cutover is configured.

use crate::ids::IssueRef;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Approve the plan of `issue` as it is now. `sha` is the plan's approval hash the person saw
/// (`IssueSummary.plan_sha`); a changed plan is refused, not approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApproveRequest {
    pub issue: IssueRef,
    pub sha: String,
}

/// Where the approval is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApproveState {
    /// GitHub answered: the comment exists.
    Posted,
    /// Sent, and GitHub did not answer: the next comments pass finds it (at most 5 minutes).
    Opening,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApproveResponse {
    pub state: ApproveState,
    /// The comment body that was posted.
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_url: Option<String>,
}

/// Approve one version of a draft (ADR 0054): the newest revision of the draft on `branch`,
/// named by its plan's approval hash, and by its number when the person saw one. `summary` is
/// the discussion summary as the person edited it, kept with the approval (ADR 0055).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftApproveRequest {
    pub repo: String,
    pub number: u64,
    pub branch: String,
    pub plan_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<u32>,
    #[serde(default)]
    pub summary: String,
}

/// A recorded approval. A second press of the same version answers the first one, with
/// `created: false`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftApproveResponse {
    pub approval_id: Uuid,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
    /// The person approved their own agent's plan: allowed, and said (ADR 0054).
    pub self_approved: bool,
    pub plan_sha: String,
    /// The hash of the whole bundle approved: the plan, its design note, its ADRs (§D3).
    pub bundle_hash: String,
    /// False when this press found an approval of the same version already recorded.
    pub created: bool,
}

/// Why an approval was refused: the `reason` of a 409 or 502 answer, and `rubato.reason` on
/// `rubato.web.rejected`.
pub mod reason {
    /// The plan changed since the person saw it.
    pub const PLAN_CHANGED: &str = "plan_changed";
    /// The plan does not fit the map (ADR 0029).
    pub const PLAN_DOES_NOT_FIT: &str = "plan_does_not_fit";
    /// No plan file for the issue.
    pub const NO_PLAN: &str = "no_plan";
    /// Rubato holds no token to act as this person (ADR 0030).
    pub const NEEDS_GITHUB_AUTHORIZATION: &str = "needs_github_authorization";
    /// GitHub refused the comment.
    pub const GITHUB_REFUSED: &str = "github_refused";
    /// After the cutover, *Accept plan* posts nothing: plans are approved in Rubato (ADR 0054).
    pub const APPROVE_IN_RUBATO: &str = "approve_in_rubato";
    /// No cutover is configured, so approval in Rubato is off (ADR 0054, plan 122 A4).
    pub const CUTOVER_UNSET: &str = "cutover_unset";
    /// No draft on that branch of the issue.
    pub const NO_DRAFT: &str = "no_draft";
    /// The version named is not the draft's newest revision.
    pub const NOT_LATEST: &str = "not_latest";
    /// The draft owner's session on the branch is working: the files may change under the
    /// approval.
    pub const NOT_SETTLED: &str = "not_settled";
    /// A file of the bundle was missing when the revision was mirrored.
    pub const BUNDLE_INCOMPLETE: &str = "bundle_incomplete";
    /// A row of `## ADRs proposed` has no Blob, or one that is not the mirrored file's: the
    /// handoff would fail `adr_changed`.
    pub const ADRS_UNPINNED: &str = "adrs_unpinned";
}

pub mod routes {
    pub const PLAN_APPROVE: &str = "/api/plans/approve";
    /// Map-scoped, for the fit check (ADR 0049).
    pub const PLAN_DRAFT_APPROVE: &str = "/api/plans/drafts/approve";
}
