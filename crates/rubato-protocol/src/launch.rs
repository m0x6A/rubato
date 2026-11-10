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
