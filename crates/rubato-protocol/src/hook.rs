//! Claude Code hook events as forwarded by the companion (seam `companion-claude-code`).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::session::{BlockCause, SessionStatus};

/// Hook event names we forward. Unknown names arrive as `Unknown` and are kept, not dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum HookEventKind {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Notification,
    Stop,
    StopFailure,
    SessionEnd,
    #[serde(other)]
    Unknown,
}

impl HookEventKind {
    /// The session status this event implies, if any. `Notification` depends on its type.
    pub fn implied_status(self, notification_type: Option<&str>) -> Option<SessionStatus> {
        match self {
            Self::SessionStart
            | Self::UserPromptSubmit
            | Self::PreToolUse
            | Self::PostToolUse
            | Self::PostToolUseFailure => Some(SessionStatus::Working),
            Self::Notification => match notification_type {
                Some("permission_prompt") | Some("idle_prompt") => Some(SessionStatus::Blocked),
                _ => None,
            },
            Self::Stop => Some(SessionStatus::Idle),
            Self::StopFailure => Some(SessionStatus::Blocked),
            Self::SessionEnd => Some(SessionStatus::Ended),
            Self::Unknown => None,
        }
    }

    /// Which hook blocked the session, when this event implies *blocked* (ADR 0043). `None`
    /// for every other event, so a hook that implies another status clears the cause.
    pub fn implied_block_cause(self, notification_type: Option<&str>) -> Option<BlockCause> {
        match self {
            Self::Notification => match notification_type {
                Some("permission_prompt") => Some(BlockCause::PermissionPrompt),
                Some("idle_prompt") => Some(BlockCause::IdlePrompt),
                _ => None,
            },
            Self::StopFailure => Some(BlockCause::StopFailure),
            _ => None,
        }
    }
}

/// One hook invocation. Common stdin fields are lifted; the full payload travels in `payload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HookEvent {
    /// Claude Code `session_id`.
    pub session_id: String,
    pub hook_event_name: HookEventKind,
    /// `cwd` from the payload: the worktree, not the original project dir.
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// `notification_type` for `Notification` events, such as `permission_prompt` or `idle_prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// When the companion received it.
    pub received_at: DateTime<Utc>,
    /// The raw hook payload, for traces and for fields this version does not lift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl HookEvent {
    /// Lift the common fields out of a raw Claude Code hook payload.
    pub fn from_payload(payload: serde_json::Value, received_at: DateTime<Utc>) -> Option<Self> {
        let s = |k: &str| payload.get(k).and_then(|v| v.as_str()).map(str::to_owned);
        let hook_event_name = payload
            .get("hook_event_name")
            .cloned()
            .and_then(|v| serde_json::from_value::<HookEventKind>(v).ok())
            .unwrap_or(HookEventKind::Unknown);
        Some(Self {
            session_id: s("session_id")?,
            hook_event_name,
            cwd: s("cwd").unwrap_or_default(),
            transcript_path: s("transcript_path"),
            permission_mode: s("permission_mode"),
            notification_type: s("notification_type"),
            tool_name: s("tool_name"),
            received_at,
            payload: Some(payload),
        })
    }

    pub fn implied_status(&self) -> Option<SessionStatus> {
        self.hook_event_name
            .implied_status(self.notification_type.as_deref())
    }

    pub fn implied_block_cause(&self) -> Option<BlockCause> {
        self.hook_event_name
            .implied_block_cause(self.notification_type.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap()
    }

    #[test]
    fn lifts_common_fields_and_keeps_payload() {
        let p = json!({"session_id":"abc","hook_event_name":"Notification","cwd":"/w","notification_type":"permission_prompt","transcript_path":"/t.jsonl"});
        let e = HookEvent::from_payload(p.clone(), fixed_now()).unwrap();
        assert_eq!(e.hook_event_name, HookEventKind::Notification);
        assert_eq!(e.implied_status(), Some(SessionStatus::Blocked));
        assert_eq!(e.payload, Some(p));
    }

    #[test]
    fn unknown_event_names_survive() {
        let p = json!({"session_id":"abc","hook_event_name":"SomethingNew2027","cwd":"/w"});
        let e = HookEvent::from_payload(p, fixed_now()).unwrap();
        assert_eq!(e.hook_event_name, HookEventKind::Unknown);
        assert_eq!(e.implied_status(), None);
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"Unknown\""));
    }

    /// The cause travels with *blocked* and only with it: every kind and notification type,
    /// a cause exactly when the status is blocked.
    #[test]
    fn implied_block_cause_names_the_blocking_hook() {
        use HookEventKind::*;
        let kinds = [
            SessionStart,
            UserPromptSubmit,
            PreToolUse,
            PostToolUse,
            PostToolUseFailure,
            Notification,
            Stop,
            StopFailure,
            SessionEnd,
            Unknown,
        ];
        let types = [
            None,
            Some("permission_prompt"),
            Some("idle_prompt"),
            Some("auth_success"),
        ];
        let mut checked = 0;
        for k in kinds {
            for t in types {
                let cause = k.implied_block_cause(t);
                assert_eq!(
                    cause.is_some(),
                    k.implied_status(t) == Some(SessionStatus::Blocked),
                    "{k:?} {t:?}"
                );
                checked += 1;
            }
        }
        assert_eq!(
            StopFailure.implied_block_cause(None),
            Some(BlockCause::StopFailure)
        );
        assert_eq!(
            Notification.implied_block_cause(Some("permission_prompt")),
            Some(BlockCause::PermissionPrompt)
        );
        assert_eq!(
            Notification.implied_block_cause(Some("idle_prompt")),
            Some(BlockCause::IdlePrompt)
        );
        // A Notification of another type implies no status and no cause.
        assert_eq!(Notification.implied_status(Some("auth_success")), None);
        assert_eq!(Notification.implied_block_cause(Some("auth_success")), None);
        println!("checked {checked} hook kinds and notification types");
    }

    #[test]
    fn missing_session_id_is_rejected() {
        assert!(HookEvent::from_payload(json!({"hook_event_name":"Stop"}), fixed_now()).is_none());
    }
}
