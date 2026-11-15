//! Plan drafts that live in Rubato until approval (ADR 0053, plan 122 design §D4). The companion
//! mirrors a launched session's plan bundle (the plan, the design note it links, the ADRs it
//! lists; `plan_hash::bundle_paths`) at the session's settle points; the server keeps each
//! settled revision. Nothing is committed.

use crate::ids::IssueRef;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One file of a bundle, as it is in the worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftFile {
    /// Repository path, as `bundle_paths` gives it.
    pub path: String,
    pub text: String,
}

/// Why a bundle was sent without its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DraftRefusal {
    /// The files together are larger than `Limits.draft_max_bytes`.
    TooLarge,
}

/// Companion → server: the plan bundle of one launched session at a settle point (`Stop`,
/// `idle_prompt`, `SessionEnd`), sent only when its hash changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftBundle {
    pub launch_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub issue: IssueRef,
    pub branch: String,
    /// The plan first, then the rest of the bundle that exists in the worktree.
    pub files: Vec<DraftFile>,
    /// Bundle paths the plan names that are not in the worktree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// `plan_hash::bundle_hash` of `files` (empty when refused).
    pub bundle_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<DraftRefusal>,
    pub at: DateTime<Utc>,
}

/// Server → companion, in `LaunchCommand.draft`: a parked draft's newest settled revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftRestore {
    pub draft_id: Uuid,
    pub rev: u32,
    /// Repository paths, relative, as `bundle_paths` gives them; the plan first.
    pub files: Vec<DraftFile>,
}

/// Why a companion did not write a launch's draft (ADR 0053, plan 122 §D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestoreSkipped {
    /// The launch reused a worktree whose files differ from the draft: they are left alone.
    WorktreeDiffers,
    /// A path is absolute or leaves the worktree: nothing was written.
    PathOutside,
}

impl RestoreSkipped {
    pub fn name(self) -> &'static str {
        match self {
            Self::WorktreeDiffers => "worktree_differs",
            Self::PathOutside => "path_outside",
        }
    }
}

/// A draft as the overview carries it, per issue: its newest settled revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftSummary {
    pub draft_id: Uuid,
    pub branch: String,
    /// The paired user whose launch first mirrored to this branch.
    pub owner: String,
    /// The newest settled revision, 1-based.
    pub rev: u32,
    pub at: DateTime<Utc>,
    /// The approval hash of the plan in that revision (ADR 0054).
    pub plan_sha: String,
    pub bundle_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// Why the plan in that revision does not parse, when it does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    /// The owner's launched session for this branch is live: the draft is being worked on.
    /// Otherwise it is parked. Derived on each overview build.
    pub live: bool,
    /// The branch an imported plan was found on (ADR 0058).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
    /// Who last committed the imported plan file, kept with the draft the importer owns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_author: Option<String>,
}

/// One settled revision, without its files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftRevisionInfo {
    pub rev: u32,
    pub at: DateTime<Utc>,
    pub plan_sha: String,
    pub bundle_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
}

/// A draft and its revisions, oldest first (`GET /api/drafts/{draft_id}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftView {
    pub draft_id: Uuid,
    pub issue: IssueRef,
    pub branch: String,
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_author: Option<String>,
    pub revisions: Vec<DraftRevisionInfo>,
}

/// One revision with its files (`GET /api/drafts/{draft_id}/revisions/{rev}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftRevision {
    pub draft_id: Uuid,
    #[serde(flatten)]
    pub info: DraftRevisionInfo,
    pub files: Vec<DraftFile>,
}

/// Import a plan written outside Rubato (ADR 0058): the plan on `branch` of the issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImportRequest {
    pub repo: String,
    pub number: u64,
    pub branch: String,
}

/// What Import did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ImportResponse {
    /// The plan parses and fits: a draft the importer owns, with the files at the head as
    /// revision 1.
    Imported {
        draft_id: Uuid,
        rev: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_author: Option<String>,
    },
    /// The plan does not parse or fit: a session of the importer's rewrites it into Rubato's
    /// form on that branch.
    Rewriting { launch_id: Uuid, branch: String },
}

/// Why an import was refused: the `reason` of its 4xx answer.
pub mod import_reason {
    /// The branch has a draft already.
    pub const ALREADY_IN_RUBATO: &str = "already_in_rubato";
    /// An approval counts for the issue's plan: it is not outside Rubato.
    pub const APPROVED: &str = "approved";
    pub const NO_BRANCH: &str = "no_branch";
    pub const NO_PLAN: &str = "no_plan";
    /// The branch is not `issue/<n>-<slug>` of the issue.
    pub const BAD_BRANCH: &str = "bad_branch";
    /// A rewrite needs a launch, and the launch was refused; `detail` says why.
    pub const LAUNCH_REFUSED: &str = "launch_refused";
}

pub mod routes {
    /// `POST` an `ImportRequest`, map-scoped: the fit check reads the workspace (ADR 0058).
    pub const PLAN_IMPORT: &str = "/api/plans/import";
    /// `GET` a draft and its revisions. Everyone who may log in reads drafts.
    pub const DRAFT: &str = "/api/drafts/{draft_id}";
    /// `GET` one revision with its files.
    pub const DRAFT_REVISION: &str = "/api/drafts/{draft_id}/revisions/{rev}";
}
