//! Launching a session on an issue, and the caps that stop it (ADR 0004).

use crate::ids::IssueRef;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Server → companion: start a session. Addressed to exactly one pairing; the companion refuses
/// any command whose `user` is not its paired user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LaunchCommand {
    pub id: Uuid,
    /// GitHub login the command is addressed to.
    pub user: String,
    pub issue: IssueRef,
    /// Clone URL of the repository.
    pub repo_url: String,
    /// `issue/<n>-<slug>`.
    pub branch: String,
    /// Ref to branch from, such as `origin/main`.
    pub base_ref: String,
    /// Map node ids the issue is pinned to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_ids: Vec<String>,
    pub spend_cap_usd: f64,
    pub attempt_limit: u32,
    /// Which attempt this is, 1-based.
    pub attempt: u32,
    /// Claude Code `--session-id` the companion must use, so `session_id → issue` is known up front.
    pub session_id: Uuid,
    /// Initial prompt for the session.
    pub prompt: String,
    /// OTel resource attributes for the session (`rubato.repo`, `rubato.issue`, ...).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub otel_resource: BTreeMap<String, String>,
    /// Git sha of the map the server resolved pins against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_version: Option<String>,
    /// Where the plan lives in the worktree, by the map's rule (ADR 0009): what the companion
    /// mirrors, with the bundle it names (ADR 0053). Absent from an older server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_path: Option<String>,
    /// The newest settled revision of the launcher's parked draft on this branch, for the
    /// companion to write into a worktree this launch creates (ADR 0053, plan 122 §D5). Sent
    /// only to a companion that announced `draft_restore`, and only while no plan is on the branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<crate::plan_draft::DraftRestore>,
    /// How the session plans (ADR 0053): `git` (default, today's flow) or `rubato`.
    #[serde(default, skip_serializing_if = "crate::handoff::PlanningMode::is_git")]
    pub planning: crate::handoff::PlanningMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// The command's user is not this companion's paired user.
    NotPairedUser,
    AttemptLimitReached,
    /// A session for this issue is already running on this machine.
    AlreadyRunning,
    /// tmux-ide or claude missing or below the pinned version.
    MissingDependency,
}

/// Companion → server: what happened to a launch command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum LaunchResult {
    Started {
        command_id: Uuid,
        session_id: Uuid,
        worktree: String,
        tmux_session: String,
        at: DateTime<Utc>,
        /// The command's `draft` was written into the new worktree (ADR 0053).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        draft_restored: bool,
        /// Why the command's `draft` was not written, when it was not.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restore_skipped: Option<crate::plan_draft::RestoreSkipped>,
    },
    Refused {
        command_id: Uuid,
        reason: RefusalReason,
        detail: String,
        at: DateTime<Utc>,
    },
    Failed {
        command_id: Uuid,
        error: String,
        at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapKind {
    Spend,
    Attempts,
}

/// A cap was hit. An event, not a log line; the issue becomes *Blocked*.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CapHit {
    pub issue: IssueRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub kind: CapKind,
    pub value: f64,
    pub limit: f64,
    pub at: DateTime<Utc>,
}

/// Start a session on an issue. Only the caller's own companions are eligible (ADR 0004).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LaunchRequest {
    pub issue: IssueRef,
    /// One of the caller's companions; the only connected one when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion_id: Option<Uuid>,
    /// Branch slug; derived from the issue title when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// The branch to launch on, `issue/<n>-<slug>`: one of the caller's drafts of the issue
    /// when they own several (ADR 0053); otherwise the only one, or a slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_cap_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LaunchResponse {
    pub command: LaunchCommand,
    pub companion_id: Uuid,
}

/// Route path for launches.
pub mod routes {
    pub const LAUNCH: &str = "/api/launch";
}

/// Why a launch was refused before it reached a companion, in the body's first word.
pub mod reason {
    /// The caller owns several drafts of the issue: `LaunchRequest.branch` names which.
    pub const PASS_BRANCH: &str = "pass_branch";
}
