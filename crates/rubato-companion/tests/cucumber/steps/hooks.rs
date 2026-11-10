//! Steps for `features/hooks.feature`.

use crate::world::*;
use cucumber::{then, when};
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
