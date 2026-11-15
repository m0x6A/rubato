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
}

/// Team routes: rooms are a plan's, not one workspace's, so they are never `scoped()`.
pub mod routes {
    /// `GET ?repo=&number=&branch=`: a `RoomView`. Everyone who may log in reads rooms.
    pub const ROOM: &str = "/api/room";
    /// `POST` a `RoomPostRequest`: the new `RoomMessage`, 201.
    pub const ROOM_MESSAGES: &str = "/api/room/messages";
}

#[cfg(test)]
mod tests {
    use super::*;

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
                .get("notice")
                .is_none()
        );
    }
}
