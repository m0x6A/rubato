//! Types on the `web-server` seam: what the server's REST API returns and accepts. The web
//! frontend's TypeScript is generated from these schemas.

use crate::github::{DecisionRecord, PullRequestState};
use crate::ids::{IssueRef, MapRef};
use crate::launch::LaunchCommand;
use crate::map::SystemMap;
use crate::plan::Plan;
use crate::progress::Progress;
use crate::session::SessionState;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Me {
    pub login: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MapResponse {
    pub map: SystemMap,
    /// Git sha the map was loaded from.
    pub version: String,
    pub loaded_at: DateTime<Utc>,
}

/// Everything the map view needs about one issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IssueSummary {
    pub issue: IssueRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub pins: Vec<MapRef>,
    pub progress: Progress,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<DecisionRecord>,
    /// Sum over this issue's sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_usd: Option<f64>,
}

/// Map versus territory, and plans that disagree with the map. Shown, never suppressed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Finding {
    /// A plan names ids the map does not declare.
    UnknownIds { issue: IssueRef, ids: Vec<String> },
    /// Two or more open plans touch the same seam.
    SeamCollision { seam: String, issues: Vec<IssueRef> },
    /// A plan file exists but does not parse.
    PlanParseError {
        issue: IssueRef,
        path: String,
        error: String,
    },
    /// A session was launched against a different map version than the server holds.
    MapVersionMismatch {
        issue: IssueRef,
        launched_with: String,
        current: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PairingInfo {
    pub companion_id: Uuid,
    pub user: String,
    pub hostname: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    /// A websocket is open right now.
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PairApproveRequest {
    pub user_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PinRequest {
    pub issue: IssueRef,
    pub target: MapRef,
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

/// The whole picture in one payload: what the web view renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Overview {
    pub map: MapResponse,
    pub issues: Vec<IssueSummary>,
    pub findings: Vec<Finding>,
    pub companions: Vec<PairingInfo>,
    /// Sessions not attached to any issue (branch outside the convention).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpinned_sessions: Vec<SessionState>,
    pub generated_at: DateTime<Utc>,
}

/// Pushed to browsers over `/ws/web`. Clients refetch the overview on `Changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebEvent {
    Changed { at: DateTime<Utc>, reason: String },
}

/// Route paths on the `web-server` seam.
pub mod routes {
    pub const ME: &str = "/api/me";
    pub const OVERVIEW: &str = "/api/overview";
    pub const MAP: &str = "/api/map";
    pub const MAP_RELOAD: &str = "/api/map/reload";
    pub const PINS: &str = "/api/pins";
    pub const LAUNCH: &str = "/api/launch";
    pub const PAIRINGS: &str = "/api/pairings";
    pub const PAIR_APPROVE: &str = "/api/pair/approve";
    pub const WEB_WS: &str = "/ws/web";
    pub const LOGIN: &str = "/auth/github/login";
    pub const CALLBACK: &str = "/auth/github/callback";
    pub const LOGOUT: &str = "/auth/logout";
}
