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

/// The prompt the companion types for `plan_approved` (ADR 0055): the record travels as
/// arguments, so nothing outside the worktree has to be read.
pub fn handoff_prompt(issue: &IssueRef, kind: &SessionMessageKind) -> Option<String> {
    let SessionMessageKind::PlanApproved {
        approval_id,
        approved_by,
        approved_at,
        approved_sha,
        bundle_hash,
        branch,
        self_approved,
        url,
        ..
    } = kind
    else {
        return None;
    };
    use chrono::{Datelike, Timelike};
    let at = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        approved_at.year(),
        approved_at.month(),
        approved_at.day(),
        approved_at.hour(),
        approved_at.minute(),
        approved_at.second()
    );
    let self_flag = if *self_approved { " --self" } else { "" };
    Some(format!(
        "Rubato: the plan for {issue} on {branch} was approved by {approved_by} at {at} (approval {approval_id}, plan sha {approved_sha}). Hand it off now with the rubato skill: plan.ts handoff --approval {approval_id} --sha {approved_sha} --bundle {bundle_hash} --by {approved_by} --at {at}{self_flag} --url {url}. Then reply with the rubato.plan.handoff line it prints."
    ))
}

/// What the message is. More kinds come with their own capability (`plan_approved`, `room_turn`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionMessageKind {
    /// Text a person typed in Rubato for their own session.
    Chat { text: String, author: String },
    /// A team member approved the session's plan in Rubato (ADR 0054): the companion types the
    /// handoff command with the approval record as arguments (ADR 0055). Goes first.
    PlanApproved {
        approval_id: Uuid,
        approved_by: String,
        approved_at: DateTime<Utc>,
        approved_sha: String,
        bundle_hash: String,
        branch: String,
        plan_path: String,
        /// The approver owns the draft.
        self_approved: bool,
        /// Rubato's link to the approval, written into accepted ADRs.
        url: String,
    },
    /// The owner stops their own session (ADR 0059): the companion kills the running turn's
    /// process group (headless) or the tmux session (interactive). Never queued, not held
    /// behind an in-flight message, and not waiting for idle. Sent only to a companion that
    /// announced `session_stop`.
    Stop { requested_by: String },
}

impl SessionMessageKind {
    /// The kind as the wire spells it, for attributes and the stored row.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Chat { .. } => "chat",
            Self::PlanApproved { .. } => "plan_approved",
            Self::Stop { .. } => "stop",
        }
    }

    /// `stop` skips the idle check and the in-flight rule (ADR 0059).
    pub fn is_stop(&self) -> bool {
        matches!(self, Self::Stop { .. })
    }

    /// The text a person sees for it: the chat text, or a line naming the approval. For
    /// `plan_approved` the companion renders the typed prompt itself (`handoff_prompt`).
    pub fn text(&self) -> String {
        match self {
            Self::Chat { text, .. } => text.clone(),
            Self::PlanApproved {
                approved_by,
                approved_sha,
                ..
            } => format!(
                "Plan approved by {approved_by} (plan {}): hand it off",
                &approved_sha[..approved_sha.len().min(7)]
            ),
            Self::Stop { requested_by } => format!("Stopped by {requested_by}"),
        }
    }

    /// The author shown with it.
    pub fn author(&self) -> &str {
        match self {
            Self::Chat { author, .. } => author,
            Self::PlanApproved { approved_by, .. } => approved_by,
            Self::Stop { requested_by } => requested_by,
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
    /// `plan_approved` for a companion whose installed skill has no `plan.ts handoff`.
    SkillOutdated,
}

impl MessageRefusal {
    pub fn name(self) -> &'static str {
        match self {
            Self::NotPairedUser => "not_paired_user",
            Self::NotLaunchedHere => "not_launched_here",
            Self::UnknownSession => "unknown_session",
            Self::TooLong => "too_long",
            Self::NoPane => "no_pane",
            Self::SkillOutdated => "skill_outdated",
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
    /// `POST` with no body: the owner stops their own session (ADR 0059). 403 for anyone else,
    /// 409 `companion_offline` when its companion is not live: a stop is never queued.
    pub const LAUNCH_STOP: &str = "/api/launches/{launch_id}/stop";
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

#[cfg(test)]
mod handoff_prompt_tests {
    use super::*;

    #[test]
    fn the_handoff_prompt_carries_the_record_as_arguments() {
        let kind = SessionMessageKind::PlanApproved {
            approval_id: Uuid::from_u128(5),
            approved_by: "m0x6A".into(),
            approved_at: DateTime::from_timestamp(1_794_700_800, 0).unwrap(),
            approved_sha: "22e89bdc981e97d34dc7057747332a8e23cdf0e9".into(),
            bundle_hash: "0123abcd".into(),
            branch: "issue/122-plan-in-rubato".into(),
            plan_path: "docs/plans/122.md".into(),
            self_approved: true,
            url: "https://rubato.example/approvals/5".into(),
        };
        let p = handoff_prompt(&IssueRef::new("m0x6A/rubato", 122), &kind).unwrap();
        assert!(p.contains("plan.ts handoff --approval 00000000-0000-0000-0000-000000000005 --sha 22e89bdc981e97d34dc7057747332a8e23cdf0e9 --bundle 0123abcd --by m0x6A --at 2026-11-15T00:00:00Z --self --url https://rubato.example/approvals/5"), "{p}");
        assert!(!p.contains('\n'), "one line, so the paste is one prompt");
        assert_eq!(
            kind.text(),
            "Plan approved by m0x6A (plan 22e89bd): hand it off"
        );
        let chat = SessionMessageKind::Chat {
            text: "hi".into(),
            author: "m0x6A".into(),
        };
        assert_eq!(
            handoff_prompt(&IssueRef::new("m0x6A/rubato", 122), &chat),
            None
        );
    }
}
