//! Steps for `features/session-log.feature`.

use super::session_messages::{launched_session, post_and_wait_with};
use crate::world::*;
use cucumber::{given, then, when};
use rubato_protocol::{CompanionMessage, HookEvent, HookEventKind, SessionMessageKind};
use std::time::Duration;

/// The text of the last session message the server sent.
fn message_text(w: &RubatoWorld) -> String {
    match &w.last_message.as_ref().expect("a message sent before").kind {
        SessionMessageKind::Chat { text, .. } => text.clone(),
        other => other.text(),
    }
}

async fn submit(w: &RubatoWorld, prompt: String) {
    let session = launched_session(w);
    let extra =
        serde_json::json!({ "prompt": prompt, "prompt_id": uuid::Uuid::new_v4().to_string() });
    post_and_wait_with(w, "UserPromptSubmit", &session, extra).await;
}

#[when("the launched session submits that message as its prompt")]
async fn when_submit_message(w: &mut RubatoWorld) {
    submit(w, message_text(w)).await;
}

#[when(expr = "the launched session submits {string} and then that message as its prompt")]
async fn when_submit_mixed(w: &mut RubatoWorld, before: String) {
    submit(w, format!("{before}{}", message_text(w))).await;
}

#[when(expr = "the launched session submits {string} as its prompt")]
async fn when_submit_text(w: &mut RubatoWorld, prompt: String) {
    submit(w, prompt).await;
}

#[when(expr = "the launched session stops with a reply of {int} characters")]
async fn when_long_reply(w: &mut RubatoWorld, n: usize) {
    let session = launched_session(w);
    let reply: String = "0123456789".chars().cycle().take(n).collect();
    let extra = serde_json::json!({ "last_assistant_message": reply, "prompt_id": "p-long" });
    post_and_wait_with(w, "Stop", &session, extra).await;
}

#[given(expr = "the server limits session log text to {int} characters")]
async fn given_log_limit(w: &mut RubatoWorld, n: u64) {
    w.server().limits.lock().unwrap().session_log_max_chars = n;
}

/// The last hook of `kind` the server received from the launched session.
async fn last_hook(w: &RubatoWorld, kind: HookEventKind) -> HookEvent {
    let session = launched_session(w);
    let got = w.server().received.lock().await;
    got.iter()
        .rev()
        .find_map(|e| match &e.message {
            CompanionMessage::Hook(h) if h.session_id == session && h.hook_event_name == kind => {
                Some(h.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {kind:?} hook from {session}"))
}

fn text_of(h: &HookEvent, key: &str) -> String {
    h.payload
        .as_ref()
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("no {key} in {:?}", h.payload))
        .to_owned()
}

async fn then_prompt_carries(w: &RubatoWorld, mixed: bool) {
    let h = last_hook(w, HookEventKind::UserPromptSubmit).await;
    let m = w.last_message.as_ref().unwrap();
    assert_eq!(h.message_id, Some(m.id), "{h:?}");
    assert_eq!(h.mixed, mixed, "{h:?}");
    assert_eq!(h.launch_id, Some(m.launch_id));
    let prompt = text_of(&h, "prompt");
    assert!(prompt.ends_with(&message_text(w)), "{prompt:?}");
    assert!(!h.truncated);
}

#[then("the server receives the prompt with that message's id, not mixed")]
async fn then_not_mixed(w: &mut RubatoWorld) {
    then_prompt_carries(w, false).await;
}

#[then("the server receives the prompt with that message's id, mixed")]
async fn then_mixed(w: &mut RubatoWorld) {
    then_prompt_carries(w, true).await;
}

#[then(expr = "the server receives the prompt {string} with no message id")]
async fn then_untagged(w: &mut RubatoWorld, prompt: String) {
    let h = last_hook(w, HookEventKind::UserPromptSubmit).await;
    assert_eq!(text_of(&h, "prompt"), prompt);
    assert_eq!((h.message_id, h.mixed), (None, false));
    assert_eq!(h.launch_id, Some(w.last_launch.as_ref().unwrap().id));
}

#[then(expr = "the server receives the reply cut to {int} characters, marked truncated")]
async fn then_cut(w: &mut RubatoWorld, n: usize) {
    // The hook is already there (`post_and_wait_with`); allow the snapshot that follows it.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let h = last_hook(w, HookEventKind::Stop).await;
    let reply = text_of(&h, "last_assistant_message");
    assert_eq!(reply.chars().count(), n, "{reply:?}");
    assert!("0123456789".repeat(10).starts_with(&reply), "{reply:?}");
    assert!(h.truncated, "{h:?}");
}
