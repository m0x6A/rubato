//! Steps for `features/session-messages.feature` (and the message steps `restart.feature`
//! reuses).

use crate::world::*;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use rubato_companion::session_input::{Deliveries, Delivery};
use rubato_companion::sessions::load_launches;
use rubato_protocol::{
    CompanionMessage, HookEventKind, IssueRef, MessageRefusal, ServerMessage, SessionMessage,
    SessionMessageKind, SessionMessageOutcome,
};
use std::time::Duration;
use uuid::Uuid;

/// A docstring as written, without the indentation of its block and the surrounding newlines.
pub(crate) fn docstring(step: &Step) -> String {
    let raw = step.docstring.as_deref().expect("a docstring");
    let lines: Vec<&str> = raw.trim_matches('\n').lines().collect();
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| l.get(indent..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn hooks_for(session: &str, event: HookEventKind) -> impl Fn(&CompanionMessage) -> bool {
    let session = session.to_owned();
    move |m| matches!(m, CompanionMessage::Hook(h) if h.session_id == session && h.hook_event_name == event)
}

/// The launched session's current id: the one it moved to, else the launch's.
pub(crate) fn launched_session(w: &RubatoWorld) -> String {
    w.new_session_id
        .clone()
        .unwrap_or_else(|| w.last_launch.as_ref().unwrap().session_id.to_string())
}

/// Fire the hook and wait until the server has it: the companion applies a hook to the
/// registry before it forwards it, so a message pushed after this sees it.
pub(crate) async fn post_and_wait(w: &RubatoWorld, event: &str, session: &str) {
    post_and_wait_with(w, event, session, serde_json::json!({})).await;
}

/// As `post_and_wait`, with `extra` payload keys.
pub(crate) async fn post_and_wait_with(
    w: &RubatoWorld,
    event: &str,
    session: &str,
    extra: serde_json::Value,
) {
    let kind: HookEventKind = serde_json::from_value(serde_json::json!(event)).unwrap();
    let pred = hooks_for(session, kind);
    let before = w.server().count(&pred).await;
    w.fire_launched_hook(event, session, extra).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while w.server().count(&pred).await <= before {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server never received the {event} hook for {session}"
        );
        let _ =
            tokio::time::timeout(Duration::from_millis(200), w.server().notify.notified()).await;
    }
}

#[when(expr = "the launched session posts a {string} hook")]
async fn when_launched_hook(w: &mut RubatoWorld, event: String) {
    let session = launched_session(w);
    post_and_wait(w, &event, &session).await;
}

#[when(expr = "the launched session posts a {string} hook as a new session id")]
async fn when_launched_hook_new_id(w: &mut RubatoWorld, event: String) {
    let session = Uuid::new_v4().to_string();
    w.new_session_id = Some(session.clone());
    post_and_wait(w, &event, &session).await;
}

fn message_for(w: &RubatoWorld, id: Uuid, text: &str) -> SessionMessage {
    let cmd = w.last_launch.as_ref().expect("a launch");
    SessionMessage {
        id,
        user: cmd.user.clone(),
        launch_id: cmd.id,
        session_id: Some(launched_session(w)),
        issue: cmd.issue.clone(),
        kind: SessionMessageKind::Chat {
            text: text.to_owned(),
            author: cmd.user.clone(),
        },
        created_at: chrono::Utc::now(),
    }
}

pub(crate) fn send(w: &mut RubatoWorld, m: SessionMessage) {
    w.last_message = Some(m.clone());
    w.server().push(ServerMessage::SessionMessage(m));
}

#[when("the server sends the launched session the message:")]
async fn when_send_docstring(w: &mut RubatoWorld, step: &Step) {
    let m = message_for(w, Uuid::new_v4(), &docstring(step));
    send(w, m);
}

#[when(expr = "the server sends the launched session the message {string}")]
async fn when_send(w: &mut RubatoWorld, text: String) {
    let m = message_for(w, Uuid::new_v4(), &text);
    send(w, m);
}

#[when("the server sends that message again")]
async fn when_send_again(w: &mut RubatoWorld) {
    let m = w.last_message.clone().expect("a message sent before");
    send(w, m);
}

#[when("the server sends that message to the launched session")]
async fn when_send_typing(w: &mut RubatoWorld) {
    let id = w.typing_id.expect("an id recorded as typing");
    let m = message_for(w, id, "typed while the companion stopped");
    send(w, m);
}

#[when(expr = "the server sends a message for user {string} to a launch")]
async fn when_send_unknown_launch(w: &mut RubatoWorld, user: String) {
    let m = SessionMessage {
        id: Uuid::new_v4(),
        user: user.clone(),
        launch_id: Uuid::new_v4(),
        session_id: None,
        issue: IssueRef::new("acme/pantry", 7),
        kind: SessionMessageKind::Chat {
            text: "hi".into(),
            author: user,
        },
        created_at: chrono::Utc::now(),
    };
    send(w, m);
}

/// Every outcome the server received for the last message, in order.
async fn outcomes(w: &RubatoWorld) -> Vec<SessionMessageOutcome> {
    let id = w.last_message.as_ref().unwrap().id;
    w.server()
        .received
        .lock()
        .await
        .iter()
        .filter_map(|e| match &e.message {
            CompanionMessage::SessionMessageResult(r) if r.message_id == id => {
                Some(r.outcome.clone())
            }
            _ => None,
        })
        .collect()
}

/// Wait until the last message has `n` results named `name`.
async fn wait_outcomes(w: &RubatoWorld, name: &str, n: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let got = outcomes(w).await;
        let count = got.iter().filter(|o| o.name() == name).count();
        if count >= n {
            assert_eq!(count, n, "{got:?}");
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected {n} {name:?} for the message, got {got:?}"
        );
        let _ =
            tokio::time::timeout(Duration::from_millis(200), w.server().notify.notified()).await;
    }
}

#[then(expr = "the server receives the outcome {string} for that message")]
async fn then_outcome(w: &mut RubatoWorld, name: String) {
    wait_outcomes(w, &name, 1).await;
}

#[then(expr = "the server receives the outcome {string} for that message {int} times")]
async fn then_outcome_times(w: &mut RubatoWorld, name: String, n: usize) {
    wait_outcomes(w, &name, n).await;
}

#[then(expr = "the server receives the refusal {string} for that message")]
async fn then_refusal(w: &mut RubatoWorld, reason: String) {
    wait_outcomes(w, "refused", 1).await;
    let got = outcomes(w).await;
    let reasons: Vec<MessageRefusal> = got
        .iter()
        .filter_map(|o| match o {
            SessionMessageOutcome::Refused { reason } => Some(*reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons.iter().map(|r| r.name()).collect::<Vec<_>>(),
        [reason.as_str()]
    );
}

#[then("the stand-in claude was typed exactly:")]
async fn then_typed(w: &mut RubatoWorld, step: &Step) {
    let want = format!("{}\n", docstring(step));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while typed(w) != want && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(typed(w), want);
    // And nothing more arrives: typed once.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(typed(w), want, "typed again");
}

#[then("nothing was typed into the stand-in claude")]
async fn then_nothing_typed(w: &mut RubatoWorld) {
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(typed(w), "", "something was typed");
}

#[then(expr = "the companion still reports the launched session as {string}")]
async fn then_still_status(w: &mut RubatoWorld, status: String) {
    let id = w.last_launch.as_ref().unwrap().id;
    let reg = w.handles.as_ref().unwrap().registry.lock().await;
    let snap = reg.snapshot();
    let s = snap
        .iter()
        .find(|s| s.launch_id == Some(id))
        .expect("the launch in the snapshot");
    assert_eq!(
        serde_json::to_value(s.status).unwrap().as_str(),
        Some(status.as_str()),
        "{s:?}"
    );
}

#[given("the companion's delivery record says a message was being typed")]
async fn given_typing(w: &mut RubatoWorld) {
    let id = Uuid::new_v4();
    Deliveries::open(&w.state_dir)
        .unwrap()
        .record(id, Delivery::Typing, chrono::Utc::now())
        .unwrap();
    w.typing_id = Some(id);
}

#[then("the companion reports the launch under the new session id and no other session")]
async fn then_moved(w: &mut RubatoWorld) {
    let cmd = w.last_launch.clone().unwrap();
    let new = w.new_session_id.clone().unwrap();
    let reg = w.handles.as_ref().unwrap().registry.lock().await;
    let e = reg.by_launch(cmd.id).expect("the launch");
    assert_eq!(e.session_id.as_deref(), Some(new.as_str()));
    assert!(reg.by_session(&cmd.session_id.to_string()).is_none());
    let snap = reg.snapshot();
    assert_eq!(
        snap.iter()
            .filter(|s| s.session_id.as_deref() == Some(new.as_str()))
            .count(),
        1,
        "{snap:?}"
    );
    assert!(
        snap.iter()
            .all(|s| s.issue != Some(cmd.issue.clone()) || s.launch_id == Some(cmd.id)),
        "no unlaunched session for the issue: {snap:?}"
    );
}

#[then("the server receives a sessions snapshot with the launch under the new session id")]
async fn then_snapshot_moved(w: &mut RubatoWorld) {
    let id = w.last_launch.as_ref().unwrap().id;
    let new = w.new_session_id.clone().unwrap();
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Sessions { sessions } => sessions
                .iter()
                .find(|s| s.launch_id == Some(id) && s.session_id.as_deref() == Some(new.as_str()))
                .map(|_| ()),
            _ => None,
        })
        .await;
    assert!(got.is_some(), "no snapshot with launch {id} under {new}");
}

#[then("launches.json names the new session id")]
async fn then_launches_file(w: &mut RubatoWorld) {
    let id = w.last_launch.as_ref().unwrap().id;
    let new = w.new_session_id.clone().unwrap();
    let records = load_launches(&w.state_dir);
    let r = records
        .iter()
        .find(|r| r.launch_id == id)
        .unwrap_or_else(|| panic!("launch {id} not in {records:?}"));
    assert_eq!(r.session_id, new);
}
