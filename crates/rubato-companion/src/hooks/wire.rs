//! What a hook event carries off the machine (ADR 0052, plan 122). The lifted fields travel as
//! they are. The raw payload is cut to an allowlist of keys, so tool input and output, prompts,
//! replies and anything Claude Code adds later stay on the machine.

use rubato_protocol::HookEvent;
use serde_json::Value;

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

/// The event as it is forwarded, and whether anything was cut from its payload.
pub fn for_wire(ev: &HookEvent) -> (HookEvent, bool) {
    let mut out = ev.clone();
    let trimmed = match &ev.payload {
        None => false,
        Some(Value::Object(map)) => {
            let kept: serde_json::Map<String, Value> = map
                .iter()
                .filter(|(k, _)| PAYLOAD_ALLOWLIST.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
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
    (out, trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(payload: Value) -> HookEvent {
        HookEvent::from_payload(payload, chrono::Utc::now()).unwrap()
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
        let (w, trimmed) = for_wire(&e);
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
        let (w, trimmed) = for_wire(&ev(json!({
            "session_id": "s", "hook_event_name": "UserPromptSubmit", "cwd": "/w",
            "prompt": "the secret plan", "prompt_id": "p1"
        })));
        assert!(trimmed);
        let p = w.payload.unwrap();
        assert!(p.get("prompt").is_none());
        assert_eq!(p.get("prompt_id"), Some(&json!("p1")));
        let (w, _) = for_wire(&ev(json!({
            "session_id": "s", "hook_event_name": "Stop", "cwd": "/w",
            "last_assistant_message": "done", "stop_hook_active": false
        })));
        assert!(w.payload.unwrap().get("last_assistant_message").is_none());
    }

    #[test]
    fn an_allowlisted_payload_is_not_counted_as_trimmed() {
        let (_, trimmed) = for_wire(&ev(json!({
            "session_id": "s", "hook_event_name": "SessionStart", "cwd": "/w", "source": "startup"
        })));
        assert!(!trimmed);
    }
}
