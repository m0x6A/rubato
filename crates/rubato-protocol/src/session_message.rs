//! Messages a person sends from Rubato to a session they launched themselves (ADR 0051, plan 122
//! design §D1). The server queues them; the companion types one only when the session waits for
//! input, and answers once. Delivery is at most once: nothing is typed twice.

use crate::ids::IssueRef;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Server → companion: something to type into one launched session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionMessage {
    pub id: Uuid,
    /// GitHub login of the session's paired user. The companion refuses any other.
    pub user: String,
    /// The launch the message is for. A `/clear` changes the session id, not the launch.
    pub launch_id: Uuid,
    /// The launch's Claude Code session id as the server last knew it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub issue: IssueRef,
    #[serde(flatten)]
    pub kind: SessionMessageKind,
    pub created_at: DateTime<Utc>,
}

/// What the message is. More kinds come with their own capability (`plan_approved`, `room_turn`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionMessageKind {
    /// Text a person typed in Rubato for their own session.
    Chat { text: String, author: String },
}

impl SessionMessageKind {
    /// The kind as the wire spells it, for attributes and the stored row.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Chat { .. } => "chat",
        }
    }

    /// The text the companion types.
    pub fn text(&self) -> &str {
        match self {
            Self::Chat { text, .. } => text,
        }
    }
}

/// Why the companion did not type a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRefusal {
    /// The message's user is not this companion's paired user (ADR 0004).
    NotPairedUser,
    /// No session this companion launched has that launch id.
    NotLaunchedHere,
    /// The launch is known but has no live session to type into.
    UnknownSession,
    /// Longer than `Limits.session_message_max_chars`.
    TooLong,
    /// The session's tmux pane is not known.
    NoPane,
}

impl MessageRefusal {
    pub fn name(self) -> &'static str {
        match self {
            Self::NotPairedUser => "not_paired_user",
            Self::NotLaunchedHere => "not_launched_here",
            Self::UnknownSession => "unknown_session",
            Self::TooLong => "too_long",
            Self::NoPane => "no_pane",
        }
    }
}

/// What happened to one message on the companion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SessionMessageOutcome {
    /// Typed into the session, once. A repeated push of a delivered id answers this again
    /// without typing.
    Delivered,
    /// The session was working; nothing was typed. The server queues it again.
    NotIdle,
    Refused {
        reason: MessageRefusal,
    },
    /// The companion stopped while pasting this id: whether it reached the session is unknown.
    Interrupted,
    /// The paste failed; nothing was typed.
    Failed {
        error: String,
    },
}

impl SessionMessageOutcome {
    /// The outcome as the wire spells it, for the `rubato.outcome` attribute.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::NotIdle => "not_idle",
            Self::Refused { .. } => "refused",
            Self::Interrupted => "interrupted",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Companion → server: the answer to one `SessionMessage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionMessageResult {
    pub message_id: Uuid,
    #[serde(flatten)]
    pub outcome: SessionMessageOutcome,
    pub at: DateTime<Utc>,
}

/// Where a message is, as the server holds it. Derived from what happened; a row is never
/// pushed again once it is `delivery_unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionMessageState {
    /// Waiting on the server: the session is working, or its companion is away.
    Queued,
    /// Pushed to the companion; no answer yet.
    Sent,
    Delivered,
    Refused,
    Failed,
    /// Pushed, but no answer while the companion stayed live, or it stopped while pasting.
    DeliveryUnknown,
}

impl SessionMessageState {
    pub const ALL: [Self; 6] = [
        Self::Queued,
        Self::Sent,
        Self::Delivered,
        Self::Refused,
        Self::Failed,
        Self::DeliveryUnknown,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Sent => "sent",
            Self::Delivered => "delivered",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::DeliveryUnknown => "delivery_unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == s)
    }
}

/// Web → server: send this text to the launch's session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionMessageRequest {
    pub text: String,
}

/// One message as the web shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionMessageView {
    pub id: Uuid,
    pub launch_id: Uuid,
    /// `chat` for now.
    pub kind: String,
    pub text: String,
    pub author: String,
    pub state: SessionMessageState,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_at: Option<DateTime<Utc>>,
    /// The refusal reason or the error, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The messages of one launch, newest last, and whether its companion can take them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LaunchMessages {
    pub launch_id: Uuid,
    /// The launch's paired user: only they may send.
    pub owner: String,
    /// The companion announced `session_input` the last time it said Hello. False for an older
    /// companion: the web says "upgrade the companion".
    pub companion_can_receive: bool,
    /// The companion is connected and heard from within two heartbeats.
    pub companion_live: bool,
    pub messages: Vec<SessionMessageView>,
}

pub mod routes {
    /// `GET` the launch's messages; `POST` a `SessionMessageRequest` (owner only).
    pub const LAUNCH_MESSAGES: &str = "/api/launches/{launch_id}/messages";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_and_its_result_round_trip_with_flat_tags() {
        let m = SessionMessage {
            id: Uuid::nil(),
            user: "m0x6A".into(),
            launch_id: Uuid::nil(),
            session_id: Some("s".into()),
            issue: IssueRef::new("m0x6A/rubato", 122),
            kind: SessionMessageKind::Chat {
                text: "line one\nline two".into(),
                author: "m0x6A".into(),
            },
            created_at: DateTime::from_timestamp(0, 0).unwrap(),
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["kind"], "chat");
        assert_eq!(v["text"], "line one\nline two");
        assert_eq!(serde_json::from_value::<SessionMessage>(v).unwrap(), m);
        let r = SessionMessageResult {
            message_id: Uuid::nil(),
            outcome: SessionMessageOutcome::Refused {
                reason: MessageRefusal::NotPairedUser,
            },
            at: DateTime::from_timestamp(0, 0).unwrap(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["outcome"], "refused");
        assert_eq!(v["reason"], "not_paired_user");
        assert_eq!(
            serde_json::from_value::<SessionMessageResult>(v).unwrap(),
            r
        );
    }

    #[test]
    fn every_state_parses_back() {
        for s in SessionMessageState::ALL {
            assert_eq!(SessionMessageState::parse(s.name()), Some(s));
        }
        assert_eq!(SessionMessageState::parse("nope"), None);
    }
}
