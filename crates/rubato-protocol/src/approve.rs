//! Accept a plan in Rubato (ADR 0030), on the `web-server` seam. The server posts
//! `Approved: plan <sha> on <branch>` as the person, through their GitHub App user-to-server
//! token; approval is still that comment (ADR 0027), counted only while the plan fits (ADR 0029).

use crate::ids::IssueRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
}

pub mod routes {
    pub const PLAN_APPROVE: &str = "/api/plans/approve";
}
