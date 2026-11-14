//! The plan's room (ADR 0057, plan 122 design §D8): a plan is talked about where everyone who may
//! log in reads it. A room belongs to a plan, keyed like its draft by `(repo, number, branch)`.
//! People post; Rubato posts a system notice when this plan and another touch the same seam.
//! Agents are never in a room by default; calling one in is PR 11.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Who wrote a room message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomAuthorKind {
    /// A person who may log in.
    Person,
    /// An agent called into the room by its owner (PR 11).
    Agent,
    /// Rubato itself: seam notices, limits reached.
    System,
}

impl RoomAuthorKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }
}

/// One message of a room, oldest first in a `RoomView`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoomMessage {
    pub id: Uuid,
    pub author_kind: RoomAuthorKind,
    /// The person's login, the agent's handle (`<owner>-agent`), or `rubato`.
    pub author: String,
    pub text: String,
    pub at: DateTime<Utc>,
    /// For a seam notice: the other issue and the seam both plans touch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<SeamNotice>,
    /// Agent handles this message addresses (`@<owner>-agent`), PR 11.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addressed: Vec<String>,
    /// The turn this message made, or the turn an agent's message answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<Uuid>,
}

/// An agent called into a room (ADR 0057): who is in, called by whom, since when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoomAgent {
    /// `<owner>-agent`, what a message names to address it.
    pub handle: String,
    pub owner: String,
    pub launch_id: Uuid,
    pub called_by: String,
    pub joined_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_reason: Option<LeftReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_at: Option<DateTime<Utc>>,
}

/// Why an agent left a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LeftReason {
    /// Its owner, or whoever called it in, dismissed it.
    Dismissed,
    /// Not addressed for `Limits.room_agent_idle_secs`.
    TimedOut,
    /// Its session ended.
    SessionEnded,
}

/// A teammate's request to call someone else's agent in, pending until the owner answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoomRequest {
    pub id: Uuid,
    pub owner: String,
    pub launch_id: Uuid,
    pub requested_by: String,
    pub at: DateTime<Utc>,
    pub state: RequestState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    Pending,
    Accepted,
    Declined,
    Withdrawn,
}

/// Call an agent in (`POST` `routes::ROOM_AGENTS`): the owner's own launch joins at once; anyone
/// else's call is a request the owner answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallInRequest {
    pub repo: String,
    pub number: u64,
    pub branch: String,
    pub launch_id: Uuid,
}

/// What a call-in did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CallInResponse {
    Joined { agent: RoomAgent },
    Requested { request: RoomRequest },
}

/// The owner answers a request (`POST` `routes::ROOM_REQUEST`), or its asker withdraws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestAnswer {
    Accept,
    Decline,
    Withdraw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RequestAnswerBody {
    pub answer: RequestAnswer,
}

/// Dismiss an agent (`DELETE` `routes::ROOM_AGENTS`): its owner, or whoever called it in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DismissRequest {
    pub repo: String,
    pub number: u64,
    pub branch: String,
    pub handle: String,
}

/// Companion → server: an agent's answer to a room turn (ADR 0057).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RoomReply {
    /// The `room_turn` session message this answers.
    pub message_id: Uuid,
    pub turn_id: Uuid,
    /// The reply, cut to `Limits.session_log_max_chars`; empty when the turn failed.
    pub text: String,
    /// A fork turn's session, for the next turn's `--resume`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_session_id: Option<String>,
    /// What a fork turn cost; it counts on the issue and the owner's spend cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub at: DateTime<Utc>,
}

/// An agent's handle in a room.
pub fn agent_handle(owner: &str) -> String {
    format!("{owner}-agent")
}

/// The handles a message addresses: each `@<owner>-agent` it names, lower-cased, once.
pub fn addressed_handles(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| {
        c.is_whitespace() || matches!(c, ',' | '.' | ':' | ';' | '!' | '?' | '(' | ')')
    }) {
        if let Some(h) = word.strip_prefix('@')
            && h.len() > "-agent".len()
            && h.to_ascii_lowercase().ends_with("-agent")
        {
            let h = h.to_ascii_lowercase();
            if !out.contains(&h) {
                out.push(h);
            }
        }
    }
    out
}

/// What a system seam notice is about (ADR 0057, Q6): posted once per pair of issue and seam.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SeamNotice {
    pub seam: String,
    pub other: crate::ids::IssueRef,
    pub other_branch: String,
}

/// A room and its messages (`GET` `routes::ROOM`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoomView {
    pub repo: String,
    pub number: u64,
    pub branch: String,
    pub messages: Vec<RoomMessage>,
    /// Agents called in, present ones first (PR 11).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<RoomAgent>,
    /// Requests to call an agent in, pending ones first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requests: Vec<RoomRequest>,
}

/// Which room: the query of `GET` `routes::ROOM`, and the key of a post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoomKey {
    pub repo: String,
    pub number: u64,
    pub branch: String,
}

/// A person's message to a room (`POST` `routes::ROOM_MESSAGES`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoomPostRequest {
    pub repo: String,
    pub number: u64,
    pub branch: String,
    pub text: String,
}

/// Why a post was refused: the `reason` of its 4xx answer, and `rubato.reason` on
/// `rubato.web.rejected`.
pub mod reason {
    /// The text is empty after trimming.
    pub const EMPTY: &str = "empty";
    /// Longer than `Limits.room_message_max_chars`.
    pub const TOO_LONG: &str = "too_long";
    /// No plan and no draft on that branch: a room belongs to a plan.
    pub const NO_PLAN: &str = "no_plan";
    /// The launch is not one of this issue's on that branch.
    pub const NOT_THIS_PLAN: &str = "not_this_plan";
    /// The agent is in the room already.
    pub const ALREADY_IN: &str = "already_in";
    /// Only the owner answers a request, only its asker withdraws it, and only the owner or
    /// whoever called the agent in dismisses it.
    pub const NOT_YOURS: &str = "not_yours";
    /// The request was answered already.
    pub const ANSWERED: &str = "answered";
    /// The owner's companion cannot take room turns (capability `room_turns`).
    pub const COMPANION_CANNOT: &str = "companion_cannot";
}

/// Team routes: rooms are a plan's, not one workspace's, so they are never `scoped()`.
pub mod routes {
    /// `GET ?repo=&number=&branch=`: a `RoomView`. Everyone who may log in reads rooms.
    pub const ROOM: &str = "/api/room";
    /// `POST` a `RoomPostRequest`: the new `RoomMessage`, 201.
    pub const ROOM_MESSAGES: &str = "/api/room/messages";
    /// `POST` a `CallInRequest`; `DELETE` a `DismissRequest`.
    pub const ROOM_AGENTS: &str = "/api/room/agents";
    /// `POST` a `RequestAnswerBody` for one request.
    pub const ROOM_REQUEST: &str = "/api/room/requests/{request_id}";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_addresses_each_agent_it_names_once() {
        assert_eq!(
            addressed_handles("@m0x6A-agent, what about the seam? cc @Alice-agent @m0x6a-agent."),
            ["m0x6a-agent", "alice-agent"]
        );
        assert!(addressed_handles("mail me@example.com about the -agent and @agent").is_empty());
        assert_eq!(agent_handle("m0x6A"), "m0x6A-agent");
    }

    #[test]
    fn a_seam_notice_round_trips_and_a_person_message_has_none() {
        let m = RoomMessage {
            id: Uuid::nil(),
            author_kind: RoomAuthorKind::System,
            author: "rubato".into(),
            text: "This plan and m0x6A/rubato#7 both touch web-server.".into(),
            at: DateTime::from_timestamp(0, 0).unwrap(),
            notice: Some(SeamNotice {
                seam: "web-server".into(),
                other: crate::ids::IssueRef::new("m0x6A/rubato", 7),
                other_branch: "issue/7-x".into(),
            }),
            addressed: vec![],
            turn_id: None,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["author_kind"], "system");
        assert_eq!(serde_json::from_value::<RoomMessage>(v).unwrap(), m);
        let person = RoomMessage {
            author_kind: RoomAuthorKind::Person,
            notice: None,
            ..m
        };
        assert!(
            serde_json::to_value(&person)
                .unwrap()
                .get("addressed")
                .is_none()
        );
        assert!(
            serde_json::to_value(&person)
                .unwrap()
                .get("notice")
                .is_none()
        );
    }
}
