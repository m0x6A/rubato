//! Steps for `features/hooks.feature`.

use crate::world::*;
use cucumber::{given, then, when};
use rubato_companion::kernel::config::{HookEndpoint, ModelPrice};
use rubato_protocol::CompanionMessage;
use std::time::Duration;

#[when(expr = "Claude Code posts the recorded hook {string} to the companion")]
async fn when_post_hook(w: &mut RubatoWorld, fixture: String) {
    w.last_status = Some(w.post_hook(&fixture, true).await);
    assert_eq!(w.last_status, Some(202));
}

#[when(expr = "something posts the recorded hook {string} without the bearer token")]
async fn when_post_hook_no_token(w: &mut RubatoWorld, fixture: String) {
    w.last_status = Some(w.post_hook(&fixture, false).await);
}

#[then(expr = "the server receives a hook event {string} for session {string}")]
async fn then_hook(w: &mut RubatoWorld, event: String, session: String) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Hook(h)
                if h.session_id == session
                    && serde_json::to_value(h.hook_event_name).unwrap().as_str()
                        == Some(event.as_str()) =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(got.is_some(), "no hook {event} for {session}");
}

#[then(expr = "the companion reports that session with status {string} from source {string}")]
async fn then_status(w: &mut RubatoWorld, status: String, source: String) {
    let reg = w.handles.as_ref().unwrap().registry.lock().await;
    let e = reg
        .by_session("0f8fad5b-d9cb-469f-a165-70867728950e")
        .expect("session known");
    assert_eq!(
        serde_json::to_value(e.status).unwrap().as_str(),
        Some(status.as_str())
    );
    assert_eq!(
        serde_json::to_value(e.source).unwrap().as_str(),
        Some(source.as_str())
    );
}

#[then(expr = "the companion answers {int}")]
async fn then_answers(w: &mut RubatoWorld, code: u16) {
    assert_eq!(w.last_status, Some(code));
}

#[then(expr = "the server receives no hook event within {int} second(s)")]
async fn then_no_hook(w: &mut RubatoWorld, secs: u64) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(secs), |m| {
            matches!(m, CompanionMessage::Hook(_)).then_some(())
        })
        .await;
    assert!(got.is_none(), "a hook was forwarded");
}

/// The cause as the snapshot sent to the server carries it (ADR 0043); "" means absent.
#[then(expr = "the companion reports that session blocked by {string}")]
async fn then_blocked_by(w: &mut RubatoWorld, cause: String) {
    let reg = w.handles.as_ref().unwrap().registry.lock().await;
    let s = reg
        .snapshot()
        .into_iter()
        .find(|s| s.session_id.as_deref() == Some("0f8fad5b-d9cb-469f-a165-70867728950e"))
        .expect("session in the snapshot");
    let got = s
        .blocked_by
        .map(|c| c.name().to_owned())
        .unwrap_or_default();
    assert_eq!(got, cause, "blocked_by in {s:?}");
}

/// Spend is summed only with pricing configured; set it before the daemon starts.
#[given(expr = "the companion prices models starting with {string}")]
async fn given_pricing(w: &mut RubatoWorld, prefix: String) {
    w.workspace.companion.pricing.insert(
        prefix,
        ModelPrice {
            input: 1.0,
            output: 1.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
    );
}

#[when(expr = "Claude Code posts the recorded hook {string} naming {string} as its transcript")]
async fn when_post_hook_transcript(w: &mut RubatoWorld, fixture: String, transcript: String) {
    let mut v: serde_json::Value =
        serde_json::from_str(&RubatoWorld::hook_fixture(&fixture)).unwrap();
    v["transcript_path"] = serde_json::Value::String(transcript);
    w.last_status = Some(w.post_body(&v.to_string(), true).await);
    assert_eq!(w.last_status, Some(202));
}

/// ADR 0063: the token is in a 0600 file curl reads, never in the command (argv is public).
#[then("the hook header file is private and no hook command holds the token")]
async fn then_header_private(w: &mut RubatoWorld) {
    use std::os::unix::fs::PermissionsExt;
    let token = w.handles.as_ref().unwrap().hook_endpoint.token.clone();
    let header = HookEndpoint::header_path(&w.state_dir);
    assert_eq!(
        std::fs::metadata(&header).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(std::fs::read_to_string(&header).unwrap().contains(&token));
    let cmd = w.rendered_stop_command();
    assert!(!cmd.contains(&token), "{cmd}");
    assert!(cmd.contains(&format!("@'{}'", header.display())), "{cmd}");
}

/// The payload of the last hook the server received (ADR 0052: only the allowlist leaves).
async fn last_forwarded_payload(w: &mut RubatoWorld) -> serde_json::Value {
    w.server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Hook(h) => Some(h.payload.clone().unwrap_or_default()),
            _ => None,
        })
        .await
        .expect("a forwarded hook")
}

#[then(expr = "the forwarded hook payload has no key {string}")]
async fn then_payload_lacks(w: &mut RubatoWorld, key: String) {
    let p = last_forwarded_payload(w).await;
    assert!(p.get(&key).is_none(), "{key} left the machine: {p}");
}

#[then(expr = "the forwarded hook payload keeps key {string}")]
async fn then_payload_keeps(w: &mut RubatoWorld, key: String) {
    let p = last_forwarded_payload(w).await;
    assert!(p.get(&key).is_some(), "{key} was cut: {p}");
}

#[then("the forwarded hook names no launch")]
async fn then_no_launch(w: &mut RubatoWorld) {
    let h = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Hook(h) => Some(h.clone()),
            _ => None,
        })
        .await
        .expect("a forwarded hook");
    assert_eq!(h.launch_id, None, "{h:?}");
    assert_eq!((h.message_id, h.truncated), (None, false), "{h:?}");
}
