//! What a hook event carries off the machine (ADR 0052, plan 122). The lifted fields travel as
//! they are. The raw payload is cut to an allowlist of keys, so tool input and output and
//! anything Claude Code adds later stay on the machine. Prompts and replies stay too, except for
//! a session launched from Rubato: its `UserPromptSubmit` prompt and its `Stop` reply travel,
//! cut to `Limits.session_log_max_chars`, for the session log.

use rubato_protocol::{HookEvent, HookEventKind};
use serde_json::Value;
use uuid::Uuid;

/// Payload keys that may leave the machine: what identifies the hook and its session, nothing a
/// tool read or wrote.
pub const PAYLOAD_ALLOWLIST: &[&str] = &[
    "session_id",
    "hook_event_name",
    "cwd",
    "transcript_path",
    "permission_mode",
    "notification_type",
    "tool_name",
    "source",
    "reason",
    "prompt_id",
    "stop_hook_active",
];

/// A session launched from Rubato, as the registry knows it: its prompts and replies feed the
/// session log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launched {
    pub launch_id: Uuid,
    /// `Limits.session_log_max_chars`: longer text is cut and the event marked `truncated`.
    pub max_chars: u64,
}

/// The one payload key with log text an event may carry, for a launched session.
fn log_key(kind: HookEventKind) -> Option<&'static str> {
    match kind {
        HookEventKind::UserPromptSubmit => Some("prompt"),
        HookEventKind::Stop => Some("last_assistant_message"),
        _ => None,
    }
}

/// `text` cut to `max` characters, and whether it was cut.
fn cut(text: &str, max: u64) -> (String, bool) {
    let max = usize::try_from(max).unwrap_or(usize::MAX);
    match text.char_indices().nth(max) {
        Some((at, _)) => (text[..at].to_owned(), true),
        None => (text.to_owned(), false),
    }
}

/// The event as it is forwarded, and whether anything was cut from its payload by the
/// allowlist. `launched` is `Some` only when the registry knows the session as a launch: then
/// its launch id travels on the event, and so does its prompt or reply.
pub fn for_wire(ev: &HookEvent, launched: Option<Launched>) -> (HookEvent, bool) {
    let mut out = ev.clone();
    let log = launched.and_then(|_| log_key(ev.hook_event_name));
    let trimmed = match &ev.payload {
        None => false,
        Some(Value::Object(map)) => {
            let mut kept = serde_json::Map::new();
            for (k, v) in map {
                if PAYLOAD_ALLOWLIST.contains(&k.as_str()) {
                    kept.insert(k.clone(), v.clone());
                } else if Some(k.as_str()) == log
                    && let (Some(text), Some(l)) = (v.as_str(), launched)
                {
                    let (text, was_cut) = cut(text, l.max_chars);
                    out.truncated |= was_cut;
                    kept.insert(k.clone(), Value::String(text));
                }
            }
            let cut = kept.len() != map.len();
            out.payload = Some(Value::Object(kept));
            cut
        }
        // Not an object: nothing in it is known to be safe.
        Some(_) => {
            out.payload = None;
            true
        }
    };
    if let Some(l) = launched {
        out.launch_id = Some(l.launch_id);
    }
    (out, trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(payload: Value) -> HookEvent {
        HookEvent::from_payload(payload, chrono::Utc::now()).unwrap()
    }

    fn for_wire_none(e: &HookEvent) -> (HookEvent, bool) {
        for_wire(e, None)
    }

    fn launched(max_chars: u64) -> Option<Launched> {
        Some(Launched {
            launch_id: Uuid::from_u128(7),
            max_chars,
        })
    }

    /// ADR 0052: a launched session's prompt and reply leave, cut to the limit; nothing else
    /// outside the allowlist does, launched or not.
    #[test]
    fn a_launched_session_forwards_its_prompt_and_reply_cut_to_the_limit() {
        let prompt = ev(json!({
            "session_id": "s", "hook_event_name": "UserPromptSubmit", "cwd": "/w",
            "prompt": "héllo wörld", "prompt_id": "p1", "secret_x": "never"
        }));
        let (w, trimmed) = for_wire(&prompt, launched(100));
        assert!(trimmed, "secret_x was cut");
        let p = w.payload.clone().unwrap();
        assert_eq!(p["prompt"], "héllo wörld");
        assert!(p.get("secret_x").is_none());
        assert!(!w.truncated);
        assert_eq!(w.launch_id, Some(Uuid::from_u128(7)));

        // Cut in characters, not bytes, and marked.
        let (w, _) = for_wire(&prompt, launched(4));
        assert_eq!(w.payload.unwrap()["prompt"], "héll");
        assert!(w.truncated);

        let stop = ev(json!({
            "session_id": "s", "hook_event_name": "Stop", "cwd": "/w",
            "last_assistant_message": "0123456789", "prompt": "not on a Stop"
        }));
        let (w, _) = for_wire(&stop, launched(5));
        let p = w.payload.unwrap();
        assert_eq!(p["last_assistant_message"], "01234");
        assert!(p.get("prompt").is_none(), "only the event's own text");
        assert!(w.truncated);

        // Tool input and output never leave, launched or not.
        let tool = ev(json!({
            "session_id": "s", "hook_event_name": "PostToolUse", "cwd": "/w",
            "tool_input": {"command": "cat secrets.env"}, "tool_response": "TOKEN=abc"
        }));
        let p = for_wire(&tool, launched(100)).0.payload.unwrap();
        assert!(p.get("tool_input").is_none() && p.get("tool_response").is_none());

        // A prompt that is not text is not forwarded.
        let odd = ev(
            json!({"session_id": "s", "hook_event_name": "UserPromptSubmit", "cwd": "/w",
            "prompt": {"text": "x"}}),
        );
        assert!(
            for_wire(&odd, launched(100))
                .0
                .payload
                .unwrap()
                .get("prompt")
                .is_none()
        );

        // Not launched: the same prompt stays on the machine and no launch is claimed.
        let (w, _) = for_wire(&prompt, None);
        assert!(w.payload.unwrap().get("prompt").is_none());
        assert_eq!(w.launch_id, None);
    }

    #[test]
    fn tool_input_and_output_are_cut_and_the_metadata_kept() {
        let e = ev(json!({
            "session_id": "s", "hook_event_name": "PostToolUse", "cwd": "/w",
            "transcript_path": "/t.jsonl", "permission_mode": "acceptEdits", "tool_name": "Bash",
            "tool_input": {"command": "cat secrets.env"},
            "tool_response": {"stdout": "TOKEN=abc"},
            "tool_use_id": "toolu_1", "secret_x": "never"
        }));
        let (w, trimmed) = for_wire(&e, None);
        assert!(trimmed);
        let p = w.payload.unwrap();
        for k in ["tool_input", "tool_response", "tool_use_id", "secret_x"] {
            assert!(p.get(k).is_none(), "{k} left the machine");
        }
        for k in [
            "session_id",
            "hook_event_name",
            "cwd",
            "tool_name",
            "permission_mode",
        ] {
            assert!(p.get(k).is_some(), "{k} was cut");
        }
        // The lifted fields are untouched.
        assert_eq!(w.tool_name.as_deref(), Some("Bash"));
        assert_eq!(w.transcript_path.as_deref(), Some("/t.jsonl"));
    }

    #[test]
    fn prompts_and_replies_stay_on_the_machine() {
        let (w, trimmed) = for_wire_none(&ev(json!({
            "session_id": "s", "hook_event_name": "UserPromptSubmit", "cwd": "/w",
            "prompt": "the secret plan", "prompt_id": "p1"
        })));
        assert!(trimmed);
        let p = w.payload.unwrap();
        assert!(p.get("prompt").is_none());
        assert_eq!(p.get("prompt_id"), Some(&json!("p1")));
        let (w, _) = for_wire_none(&ev(json!({
            "session_id": "s", "hook_event_name": "Stop", "cwd": "/w",
            "last_assistant_message": "done", "stop_hook_active": false
        })));
        assert!(w.payload.unwrap().get("last_assistant_message").is_none());
    }

    #[test]
    fn an_allowlisted_payload_is_not_counted_as_trimmed() {
        let (_, trimmed) = for_wire_none(&ev(json!({
            "session_id": "s", "hook_event_name": "SessionStart", "cwd": "/w", "source": "startup"
        })));
        assert!(!trimmed);
    }
}
