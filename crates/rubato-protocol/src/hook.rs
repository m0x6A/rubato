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

/// Payload keys kept in `HookEvent::payload`; every other key stays on the developer's machine
/// (#166). Tool input and output, prompts and messages are never forwarded.
pub const PAYLOAD_ALLOWLIST: [&str; 3] = ["hook_event_name", "notification_type", "tool_name"];

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
    // Only the keys in `PAYLOAD_ALLOWLIST` (#166). The doc comment below is the schema
    // description on the companion-server seam; it is left as it was (plan 166, Q1).
    /// The raw hook payload, for traces and for fields this version does not lift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl HookEvent {
    /// Lift the common fields out of a raw Claude Code hook payload, and keep only the
    /// `PAYLOAD_ALLOWLIST` keys of it in `payload`. A payload that is not an object keeps none.
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
            payload: allowlisted(&payload),
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

    /// Keep only the `PAYLOAD_ALLOWLIST` keys of `payload`, as a companion from before #166
    /// did not (#190). Returns how many it removed.
    pub fn scrub(&mut self) -> usize {
        scrub_payload(&mut self.payload)
    }
}

/// Cut a received or stored hook payload down to the `PAYLOAD_ALLOWLIST` keys, in place, with
/// the same rule as `HookEvent::from_payload`. Returns how many keys it removed; a payload that
/// is not an object becomes `None` and counts as one.
pub fn scrub_payload(payload: &mut Option<serde_json::Value>) -> usize {
    let Some(p) = payload.as_ref() else {
        return 0;
    };
    let Some(obj) = p.as_object() else {
        *payload = None;
        return 1;
    };
    let removed = obj
        .keys()
        .filter(|k| !PAYLOAD_ALLOWLIST.contains(&k.as_str()))
        .count();
    if removed > 0 {
        *payload = allowlisted(p);
    }
    removed
}

/// The `PAYLOAD_ALLOWLIST` keys of a hook payload, or `None` when it is not an object.
fn allowlisted(payload: &serde_json::Value) -> Option<serde_json::Value> {
    let obj = payload.as_object()?;
    let kept: serde_json::Map<String, serde_json::Value> = PAYLOAD_ALLOWLIST
        .iter()
        .filter_map(|k| obj.get(*k).map(|v| ((*k).to_owned(), v.clone())))
        .collect();
    Some(serde_json::Value::Object(kept))
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
    fn lifts_common_fields_and_keeps_the_allowlisted_payload() {
        let p = json!({"session_id":"abc","hook_event_name":"Notification","cwd":"/w","notification_type":"permission_prompt","transcript_path":"/t.jsonl","message":"Claude needs your permission"});
        let e = HookEvent::from_payload(p, fixed_now()).unwrap();
        assert_eq!(e.hook_event_name, HookEventKind::Notification);
        assert_eq!(e.implied_status(), Some(SessionStatus::Blocked));
        assert_eq!(e.transcript_path.as_deref(), Some("/t.jsonl"));
        assert_eq!(
            e.payload,
            Some(json!({"hook_event_name":"Notification","notification_type":"permission_prompt"}))
        );
    }

    /// Every key of a payload is kept exactly when it is on the allowlist.
    #[test]
    fn payload_keeps_only_the_allowlist() {
        let p = json!({
            "session_id": "abc",
            "hook_event_name": "PreToolUse",
            "cwd": "/w",
            "transcript_path": "/t.jsonl",
            "permission_mode": "default",
            "notification_type": "idle_prompt",
            "tool_name": "Read",
            "tool_input": {"file_path": "/w/.env"},
            "tool_response": {"content": "TOKEN=x"},
            "prompt": "p",
            "message": "m",
            "something_new_2027": "?",
        });
        let e = HookEvent::from_payload(p.clone(), fixed_now()).unwrap();
        let kept = e.payload.unwrap();
        let kept = kept.as_object().unwrap();
        let mut checked = 0;
        for (k, v) in p.as_object().unwrap() {
            if PAYLOAD_ALLOWLIST.contains(&k.as_str()) {
                assert_eq!(kept.get(k), Some(v), "{k} dropped");
            } else {
                assert!(!kept.contains_key(k), "{k} kept");
            }
            checked += 1;
        }
        assert_eq!(kept.len(), PAYLOAD_ALLOWLIST.len());
        println!("checked {checked} payload keys");
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

    /// #166: tool input, tool output and prompts never leave the companion.
    #[test]
    fn post_tool_use_secret_does_not_leave_the_companion() {
        let p = json!({
            "session_id": "abc",
            "hook_event_name": "PostToolUse",
            "cwd": "/w",
            "tool_name": "Bash",
            "tool_input": {"command": "cat .env"},
            "tool_response": {"stdout": "SECRET", "stderr": ""},
            "prompt": "my private prompt",
        });
        let e = HookEvent::from_payload(p, fixed_now()).unwrap();
        let msg = crate::CompanionMessage::Hook(e);
        let s = serde_json::to_string(&msg).unwrap();
        for leaked in [
            "SECRET",
            "cat .env",
            "my private prompt",
            "tool_input",
            "tool_response",
        ] {
            assert!(!s.contains(leaked), "{leaked} in {s}");
        }
        assert!(s.contains("\"Bash\""), "{s}");
    }

    /// #190: the server applies the same allowlist to an old companion's payload, and counts.
    #[test]
    fn scrub_payload_counts_what_it_removes() {
        let full = json!({
            "session_id": "abc",
            "hook_event_name": "PostToolUse",
            "cwd": "/w",
            "tool_name": "Bash",
            "tool_input": {"command": "cat .env"},
            "tool_response": {"stdout": "SECRET"},
            "prompt": "p",
        });
        let mut p = Some(full);
        assert_eq!(
            scrub_payload(&mut p),
            5,
            "session_id, cwd, tool_input, tool_response, prompt"
        );
        assert_eq!(
            p,
            Some(json!({"hook_event_name":"PostToolUse","tool_name":"Bash"}))
        );
        assert_eq!(
            scrub_payload(&mut p),
            0,
            "an allowlisted payload is left as it is"
        );

        let mut not_object = Some(json!("SECRET"));
        assert_eq!(scrub_payload(&mut not_object), 1);
        assert_eq!(not_object, None);
        let mut none = None;
        assert_eq!(scrub_payload(&mut none), 0);

        let mut e = HookEvent::from_payload(json!({"session_id":"abc"}), fixed_now()).unwrap();
        e.payload = Some(json!({"hook_event_name":"Stop","message":"m"}));
        assert_eq!(e.scrub(), 1);
        assert_eq!(e.payload, Some(json!({"hook_event_name":"Stop"})));
    }

    #[test]
    fn missing_session_id_is_rejected() {
        assert!(HookEvent::from_payload(json!({"hook_event_name":"Stop"}), fixed_now()).is_none());
    }
}
