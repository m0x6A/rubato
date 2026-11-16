//! A person's own lead agent and the workers it launched (ADR 0072, plan 134), behind the
//! orchestrator flag on both sides (capability `fleet`), off by default. A lead asks over the agent
//! MCP (ADR 0071); its companion sends one `FleetRequest` per call and the server answers one
//! `FleetResult`. The server decides whose agents those are from the pairing and the stored
//! launches, never from what the agent says.

use crate::ids::IssueRef;
use crate::progress::ProgressState;
use crate::session::{ClientKind, SessionStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use uuid::Uuid;

/// What a launch is for: an ordinary worker, or a person's lead agent (ADR 0072).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LaunchRole {
    #[default]
    Worker,
    Lead,
}

impl LaunchRole {
    pub fn is_worker(&self) -> bool {
        *self == Self::Worker
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Lead => "lead",
        }
    }
}

/// Companion → server: one fleet tool call of a lead launch. Sent only while the link is up and
/// never kept in the outbox, so a reconnect never replays it late.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FleetRequest {
    /// The agent's own id when it retries a pending call, else a new one the companion made.
    pub request_id: Uuid,
    /// The launch the call came from: the companion took it from `X-Rubato-Launch`.
    pub lead_launch_id: Uuid,
    pub action: FleetAction,
}

/// What a lead asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FleetAction {
    /// Its owner's launches in the lead's workspace.
    List,
    /// A worker on an issue, as its owner, on one of the owner's connected companions.
    Launch {
        issue: IssueRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        companion_id: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slug: Option<String>,
    },
    /// A chat message to a worker this lead launched.
    Message { launch_id: Uuid, text: String },
    /// A stop for a worker this lead launched.
    Stop { launch_id: Uuid },
}

impl FleetAction {
    /// `rubato.fleet.action`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Launch { .. } => "launch",
            Self::Message { .. } => "message",
            Self::Stop { .. } => "stop",
        }
    }

    /// The launch a message or stop is aimed at.
    pub fn target(&self) -> Option<Uuid> {
        match self {
            Self::Message { launch_id, .. } | Self::Stop { launch_id } => Some(*launch_id),
            _ => None,
        }
    }

    /// A hash of the action, so the same request id with another action is refused.
    pub fn hash(&self) -> String {
        let text = serde_json::to_string(self).unwrap_or_default();
        Sha1::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// What a request came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FleetOutcome {
    Ok,
    Refused,
    /// The same request is still being acted on.
    InProgress,
    /// It was claimed and never finished (the server stopped in between): never acted on again.
    Unknown,
}

impl FleetOutcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Refused => "refused",
            Self::InProgress => "in_progress",
            Self::Unknown => "unknown",
        }
    }
}

/// Server → companion: the answer to one `FleetRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FleetResult {
    pub request_id: Uuid,
    pub lead_launch_id: Uuid,
    pub outcome: FleetOutcome,
    /// One of `reason`, when refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Words for the agent: why, or what to do next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// For `list`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet: Option<Vec<FleetEntry>>,
    /// For `launch`: the new worker's launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_id: Option<Uuid>,
    /// For `message` and `stop`: the session message queued or sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Uuid>,
}

/// One of the owner's launches, as `fleet_list` shows it: the same data the team already sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FleetEntry {
    pub launch_id: Uuid,
    pub issue: IssueRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub companion_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<ClientKind>,
    /// `None` is *unknown*: the session is not in its companion's snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<SessionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressState>,
    pub role: LaunchRole,
    /// `person`, or `lead:<launch id>`.
    pub started_by: String,
    pub queued_messages: u32,
    /// The first line of the last reply, at most `limits::LAST_REPLY_CHARS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reply: Option<String>,
}

/// A lead and its live workers, for the web (`Overview.leads`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LeadView {
    pub launch_id: Uuid,
    pub owner: String,
    pub workers: Vec<Uuid>,
}

/// The header carrying a lead's secret on its MCP calls, from its own `mcp.json` (ADR 0072).
pub const LEAD_HEADER: &str = "X-Rubato-Lead";

/// Why a fleet request was refused: `FleetResult.reason` and `rubato.reason`.
pub mod reason {
    /// The server's or the companion's orchestrator flag is off.
    pub const ORCHESTRATOR_OFF: &str = "orchestrator_off";
    /// The lead is not a launch of this pairing, or the lead secret did not match. Pages.
    pub const IDENTITY: &str = "identity";
    /// The calling launch is not a lead.
    pub const NOT_A_LEAD: &str = "not_a_lead";
    /// The target is another person's launch. Pages.
    pub const OTHER_OWNER: &str = "other_owner";
    /// The target is the owner's own launch that this lead did not start, or the lead itself.
    pub const NOT_LED: &str = "not_led";
    pub const UNKNOWN_LAUNCH: &str = "unknown_launch";
    /// The lead has `RUBATO_FLEET_MAX` live workers.
    pub const FLEET_FULL: &str = "fleet_full";
    /// Another person launched the issue in the last 30 days or owns a draft of it.
    pub const SOMEONE_ELSES_ISSUE: &str = "someone_elses_issue";
    /// The same request id came with another action.
    pub const REQUEST_ID_REUSED: &str = "request_id_reused";
    /// A lead launch to a companion that did not announce `fleet`.
    pub const COMPANION_CANNOT_LEAD: &str = "companion_cannot_lead";
    pub const EMPTY: &str = "empty";
    pub const TOO_LONG: &str = "too_long";
    /// `launch::start` refused; `message` says why.
    pub const LAUNCH_REFUSED: &str = "launch_refused";
    /// The companion's link to the server is down.
    pub const LINK_DOWN: &str = "link_down";
    /// The two refusals that mean the line between people was tested: `FleetCrossOwner` pages.
    pub const PAGES: [&str; 2] = [OTHER_OWNER, IDENTITY];
}

/// Bounds of plan 134 (A2–A9).
pub mod limits {
    /// Live workers per lead unless `RUBATO_FLEET_MAX` says otherwise.
    pub const FLEET_MAX_DEFAULT: u32 = 4;
    /// How long the companion waits for a `FleetResult` before answering `pending`.
    pub const RESULT_WAIT_SECS: u64 = 15;
    /// A claim still `in_progress` after this is answered `unknown`.
    pub const IN_PROGRESS_SECS: i64 = 60;
    /// `fleet_list`: launches live or started in the last 24 hours, at most 50.
    pub const LIST_HOURS: i64 = 24;
    pub const LIST_MAX: u32 = 50;
    pub const LAST_REPLY_CHARS: usize = 200;
    /// `fleet_requests` rows older than this are deleted on insert.
    pub const RETENTION_DAYS: i64 = 30;
    /// At most one fleet notice per lead this often.
    pub const NOTICE_EVERY_SECS: i64 = 60;
    /// Another person's launch of an issue this recent makes it theirs.
    pub const SOMEONE_ELSES_DAYS: i64 = 30;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_round_trips_and_names_its_action() {
        let r = FleetRequest {
            request_id: Uuid::nil(),
            lead_launch_id: Uuid::from_u128(1),
            action: FleetAction::Launch {
                issue: IssueRef::new("m0x6A/rubato", 12),
                companion_id: None,
                slug: None,
            },
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["action"]["kind"], "launch");
        assert!(v["action"].get("companion_id").is_none());
        assert_eq!(serde_json::from_value::<FleetRequest>(v).unwrap(), r);
        assert_eq!(r.action.name(), "launch");
        assert_eq!(FleetAction::List.target(), None);
        let stop = FleetAction::Stop {
            launch_id: Uuid::from_u128(9),
        };
        assert_eq!(stop.target(), Some(Uuid::from_u128(9)));
    }

    #[test]
    fn the_action_hash_tells_two_actions_apart() {
        let a = FleetAction::Message {
            launch_id: Uuid::nil(),
            text: "go".into(),
        };
        let b = FleetAction::Message {
            launch_id: Uuid::nil(),
            text: "stop".into(),
        };
        assert_eq!(a.hash(), a.clone().hash());
        assert_ne!(a.hash(), b.hash());
        assert_eq!(a.hash().len(), 40);
    }

    #[test]
    fn a_result_omits_what_it_does_not_carry() {
        let r = FleetResult {
            request_id: Uuid::nil(),
            lead_launch_id: Uuid::nil(),
            outcome: FleetOutcome::Refused,
            reason: Some(reason::OTHER_OWNER.into()),
            message: None,
            fleet: None,
            launch_id: None,
            message_id: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["outcome"], "refused");
        assert_eq!(v["reason"], "other_owner");
        for k in ["fleet", "launch_id", "message_id", "message"] {
            assert!(v.get(k).is_none(), "{k}");
        }
        assert_eq!(LaunchRole::default(), LaunchRole::Worker);
        assert_eq!(
            serde_json::to_value(LaunchRole::Lead).unwrap(),
            serde_json::json!("lead")
        );
    }
}
