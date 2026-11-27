//! The session-message state machine (ADR 0051, plan 122 §D1): at most one in flight per launch,
//! pushed only to a live companion that can take it, pushed again at Hello with the same id,
//! *delivery unknown* only while the companion stayed live, owner-only sends.

use super::{dispatch, handlers, store};
use crate::auth::CurrentUser;
use crate::kernel::config::Config;
use crate::kernel::db::{connect, ts};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState, CompanionConn};
use crate::kernel::telemetry::metric_reader;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, TimeZone, Utc};
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::{
    BlockCause, HookEvent, HookEventKind, IssueRef, LaunchCommand, ServerMessage,
    SessionMessageOutcome, SessionMessageRequest, SessionMessageResult, SessionMessageState,
    SessionState, SessionStatus, StateSource, capability,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

const OWNER: &str = "m0x6A";

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap()
}

struct H {
    app: App,
    reader: metric_reader::Reader,
    launch: Uuid,
    companion: Uuid,
    _tmp: tempfile::TempDir,
}

async fn harness() -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = connect(&url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut app = AppState::new(
        pool,
        Config::for_tests(url, OWNER),
        Arc::new(FakeGitHub::new()),
    );
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    app.clock.set(t0());
    let launch = Uuid::new_v4();
    let companion = Uuid::new_v4();
    let cmd = LaunchCommand {
        id: launch,
        user: OWNER.into(),
        issue: IssueRef::new("m0x6A/rubato", 122),
        repo_url: "https://github.com/m0x6A/rubato.git".into(),
        branch: "issue/122-x".into(),
        base_ref: "origin/main".into(),
        node_ids: vec![],
        spend_cap_usd: 10.0,
        attempt_limit: 3,
        attempt: 1,
        session_id: Uuid::nil(),
        prompt: "p".into(),
        otel_resource: Default::default(),
        map_version: None,
        plan_path: None,
        draft: None,
        planning: Default::default(),
        role: Default::default(),
        lead_launch_id: None,
    };
    sqlx::query("INSERT INTO launches (workspace_id, id, companion_id, user, repo, number, command, created_at) VALUES ('default', ?, ?, ?, 'm0x6A/rubato', 122, ?, ?)")
        .bind(launch.to_string())
        .bind(companion.to_string())
        .bind(OWNER)
        .bind(serde_json::to_string(&cmd).unwrap())
        .bind(ts(t0()))
        .execute(&app.db)
        .await
        .unwrap();
    H {
        app,
        reader,
        launch,
        companion,
        _tmp: tmp,
    }
}

/// Connect the companion with these capabilities; returns what the server pushes to it.
fn connect_companion(h: &H, caps: &[&str]) -> mpsc::UnboundedReceiver<ServerMessage> {
    let (tx, rx) = mpsc::unbounded_channel();
    h.app
        .companions
        .lock()
        .unwrap()
        .insert(h.companion, CompanionConn::new(tx, h.app.clock.now()));
    h.app
        .set_capabilities(h.companion, caps.iter().map(|c| (*c).to_owned()).collect());
    rx
}

fn pushed(rx: &mut mpsc::UnboundedReceiver<ServerMessage>) -> Vec<(Uuid, String)> {
    let mut out = Vec::new();
    while let Ok(m) = rx.try_recv() {
        if let ServerMessage::SessionMessage(m) = m {
            out.push((m.id, m.kind.text().to_owned()));
        }
    }
    out
}

async fn send(h: &H, user: &str, text: &str) -> Result<Uuid, (StatusCode, String)> {
    handlers::send(
        State(h.app.clone()),
        CurrentUser(user.into()),
        Path(h.launch.into()),
        Json(SessionMessageRequest { text: text.into() }),
    )
    .await
    .map(|Json(v)| v.id)
    .map_err(|(s, Json(b))| (s, b["reason"].as_str().unwrap_or("").to_owned()))
}

async fn state_of(h: &H, id: Uuid) -> SessionMessageState {
    store::get(&h.app.db, id).await.unwrap().unwrap().state
}

fn result(id: Uuid, outcome: SessionMessageOutcome) -> SessionMessageResult {
    SessionMessageResult {
        message_id: id,
        outcome,
        at: t0(),
    }
}

fn stop(launch: Option<Uuid>) -> HookEvent {
    let mut ev = HookEvent::from_payload(
        serde_json::json!({"session_id": Uuid::nil().to_string(), "hook_event_name": "Stop", "cwd": "/w"}),
        t0(),
    )
    .unwrap();
    ev.launch_id = launch;
    ev
}

#[tokio::test]
async fn one_message_in_flight_per_launch_and_the_next_waits_for_an_idle_hook() {
    let h = harness().await;
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT]);
    let a = send(&h, OWNER, "first\nwith two lines").await.unwrap();
    let b = send(&h, OWNER, "second").await.unwrap();
    assert_eq!(
        pushed(&mut rx),
        [(a, "first\nwith two lines".to_owned())],
        "only the first is pushed, newlines intact"
    );
    assert_eq!(state_of(&h, a).await, SessionMessageState::Sent);
    assert_eq!(state_of(&h, b).await, SessionMessageState::Queued);
    // Delivered: the second is not pushed until the session waits again.
    dispatch::on_result(
        &h.app,
        h.companion,
        &result(a, SessionMessageOutcome::Delivered),
    )
    .await;
    assert_eq!(state_of(&h, a).await, SessionMessageState::Delivered);
    assert!(pushed(&mut rx).is_empty());
    // A working hook does not dispatch; a Stop does.
    let mut working = stop(Some(h.launch));
    working.hook_event_name = HookEventKind::PreToolUse;
    dispatch::on_hook(&h.app, h.companion, &working).await;
    assert!(pushed(&mut rx).is_empty());
    dispatch::on_hook(&h.app, h.companion, &stop(Some(h.launch))).await;
    assert_eq!(pushed(&mut rx).len(), 1);
    assert_eq!(state_of(&h, b).await, SessionMessageState::Sent);
    // not_idle: back to the queue, nothing pushed until the next idle hook.
    dispatch::on_result(
        &h.app,
        h.companion,
        &result(b, SessionMessageOutcome::NotIdle),
    )
    .await;
    assert_eq!(state_of(&h, b).await, SessionMessageState::Queued);
    assert!(pushed(&mut rx).is_empty());
    let mut idle = stop(Some(h.launch));
    idle.hook_event_name = HookEventKind::Notification;
    idle.notification_type = Some("idle_prompt".into());
    dispatch::on_hook(&h.app, h.companion, &idle).await;
    assert_eq!(
        pushed(&mut rx).len(),
        1,
        "idle_prompt counts as waiting for input"
    );
}

#[tokio::test]
async fn a_message_for_an_absent_or_old_companion_stays_queued_until_hello() {
    let h = harness().await;
    // Not connected.
    let a = send(&h, OWNER, "while away").await.unwrap();
    assert_eq!(state_of(&h, a).await, SessionMessageState::Queued);
    // Connected, but an older companion without session_input: still queued.
    let mut rx = connect_companion(&h, &[capability::HOOK_ALLOWLIST]);
    dispatch::dispatch(&h.app, h.launch).await;
    assert!(pushed(&mut rx).is_empty());
    assert_eq!(state_of(&h, a).await, SessionMessageState::Queued);
    // Connected and asleep: not heard from within two heartbeats.
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT]);
    h.app.clock.set(t0() + Duration::seconds(61));
    dispatch::dispatch(&h.app, h.launch).await;
    assert!(pushed(&mut rx).is_empty(), "a silent socket is not live");
    // Hello: delivered from the queue.
    h.app.touch_companion(h.companion, h.app.clock.now());
    dispatch::on_hello(&h.app, h.companion).await;
    assert_eq!(pushed(&mut rx), [(a, "while away".to_owned())]);
}

#[tokio::test]
async fn an_unanswered_push_is_pushed_again_at_hello_with_the_same_id_and_swept_only_while_live() {
    let h = harness().await;
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT]);
    let a = send(&h, OWNER, "x").await.unwrap();
    assert_eq!(pushed(&mut rx).len(), 1);
    // The companion goes away before answering: the sweep leaves the row alone.
    h.app.companions.lock().unwrap().clear();
    h.app.clock.set(t0() + Duration::seconds(300));
    assert_eq!(dispatch::sweep(&h.app).await, 0);
    assert_eq!(state_of(&h, a).await, SessionMessageState::Sent);
    // It comes back: the same id again, nothing new.
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT]);
    dispatch::on_hello(&h.app, h.companion).await;
    assert_eq!(pushed(&mut rx), [(a, "x".to_owned())]);
    // Live and still no answer after 60 s: delivery unknown, never pushed again.
    h.app.clock.set(t0() + Duration::seconds(300 + 30));
    h.app.touch_companion(h.companion, h.app.clock.now());
    assert_eq!(dispatch::sweep(&h.app).await, 1);
    assert_eq!(state_of(&h, a).await, SessionMessageState::DeliveryUnknown);
    dispatch::on_hello(&h.app, h.companion).await;
    dispatch::dispatch(&h.app, h.launch).await;
    assert!(
        pushed(&mut rx).is_empty(),
        "delivery_unknown is never pushed again"
    );
    // A late answer still settles it, once.
    dispatch::on_result(
        &h.app,
        h.companion,
        &result(a, SessionMessageOutcome::Delivered),
    )
    .await;
    assert_eq!(state_of(&h, a).await, SessionMessageState::Delivered);
    dispatch::on_result(
        &h.app,
        h.companion,
        &result(a, SessionMessageOutcome::Delivered),
    )
    .await;
    let delivered: u64 = h
        .reader
        .points(metric::SESSION_MESSAGES)
        .iter()
        .filter(|p| p.attrs.get(attr::OUTCOME).map(String::as_str) == Some("delivered"))
        .map(|p| p.value)
        .sum();
    assert_eq!(delivered, 1, "a repeated result is not counted twice");
}

#[tokio::test]
async fn interrupted_is_delivery_unknown_and_a_result_from_another_companion_is_ignored() {
    let h = harness().await;
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT]);
    let a = send(&h, OWNER, "x").await.unwrap();
    pushed(&mut rx);
    dispatch::on_result(
        &h.app,
        Uuid::new_v4(),
        &result(a, SessionMessageOutcome::Delivered),
    )
    .await;
    assert_eq!(state_of(&h, a).await, SessionMessageState::Sent);
    dispatch::on_result(
        &h.app,
        h.companion,
        &result(a, SessionMessageOutcome::Interrupted),
    )
    .await;
    assert_eq!(state_of(&h, a).await, SessionMessageState::DeliveryUnknown);
}

#[tokio::test]
async fn two_dispatches_at_once_push_one_message() {
    let h = harness().await;
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT]);
    for text in ["a", "b", "c"] {
        let row = store::MessageRow {
            id: Uuid::new_v4(),
            launch_id: h.launch,
            session_id: None,
            companion_id: h.companion,
            owner: OWNER.into(),
            issue: IssueRef::new("m0x6A/rubato", 122),
            kind: rubato_protocol::SessionMessageKind::Chat {
                text: text.into(),
                author: OWNER.into(),
            },
            author: OWNER.into(),
            priority: if text == "c" { 0 } else { 1 },
            state: SessionMessageState::Queued,
            created_at: t0(),
            pushed_at: None,
            outcome_at: None,
            detail: None,
            via_launch: None,
        };
        store::insert(&h.app.db, &row).await.unwrap();
    }
    tokio::join!(
        dispatch::dispatch(&h.app, h.launch),
        dispatch::dispatch(&h.app, h.launch),
        dispatch::dispatch(&h.app, h.launch),
    );
    let got = pushed(&mut rx);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, "c", "a lower priority number goes first");
}

#[tokio::test]
async fn only_the_owner_sends_and_the_text_must_fit() {
    let h = harness().await;
    assert_eq!(
        send(&h, "alice", "hi").await.unwrap_err(),
        (StatusCode::FORBIDDEN, "not_your_session".into())
    );
    assert_eq!(
        send(&h, "M0X6A", "hi").await.map(|_| ()),
        Ok(()),
        "logins compare without case"
    );
    assert_eq!(
        send(&h, OWNER, "  \n").await.unwrap_err(),
        (StatusCode::UNPROCESSABLE_ENTITY, "empty".into())
    );
    let long = "x".repeat(8001);
    assert_eq!(
        send(&h, OWNER, &long).await.unwrap_err(),
        (StatusCode::UNPROCESSABLE_ENTITY, "too_long".into())
    );
    let other = handlers::send(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Path(Uuid::new_v4().into()),
        Json(SessionMessageRequest { text: "hi".into() }),
    )
    .await
    .map_err(|(s, _)| s);
    assert_eq!(other.err(), Some(StatusCode::NOT_FOUND));
    let rejected: u64 = h
        .reader
        .points(metric::WEB_REJECTED)
        .iter()
        .map(|p| p.value)
        .sum();
    assert_eq!(rejected, 4, "every refusal is counted");
    // Everyone reads.
    let Json(list) = handlers::list(
        State(h.app.clone()),
        CurrentUser("alice".into()),
        Path(h.launch.into()),
    )
    .await
    .unwrap();
    assert_eq!(list.owner, OWNER);
    assert_eq!(list.messages.len(), 1);
    assert!(!list.companion_can_receive);
}

#[tokio::test]
async fn a_clear_moves_queued_rows_to_the_new_session_and_an_old_hook_finds_them() {
    let h = harness().await;
    let a = send(&h, OWNER, "x").await.unwrap();
    let mut s = SessionState {
        session_id: Some("new-session".into()),
        companion_id: h.companion,
        user: OWNER.into(),
        tmux: None,
        cwd: "/w".into(),
        worktree: None,
        branch: None,
        repo: None,
        issue: None,
        status: SessionStatus::Idle,
        source: StateSource::Hook,
        last_seen: t0(),
        spend_usd: None,
        branch_drift: None,
        blocked_by: None,
        launch_id: Some(h.launch),
        hooks_lost: false,
    };
    dispatch::on_sessions(&h.app, std::slice::from_ref(&s)).await;
    assert_eq!(
        store::get(&h.app.db, a)
            .await
            .unwrap()
            .unwrap()
            .session_id
            .as_deref(),
        Some("new-session")
    );
    // A hook with no launch header, from the new session id, still dispatches.
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT]);
    let mut ev = stop(None);
    ev.session_id = "new-session".into();
    dispatch::on_hook(&h.app, h.companion, &ev).await;
    assert_eq!(pushed(&mut rx).len(), 1);
    // The gauge: nothing queued now.
    dispatch::record_gauges(&h.app, std::slice::from_ref(&s)).await;
    let _ = send(&h, OWNER, "y").await.unwrap();
    h.app.clock.set(t0() + Duration::seconds(700));
    s.status = SessionStatus::Blocked;
    s.blocked_by = Some(BlockCause::IdlePrompt);
    dispatch::record_gauges(&h.app, &[s]).await;
    let age = h
        .reader
        .points(metric::SESSION_MESSAGES_OLDEST_IDLE_QUEUED)
        .last()
        .map(|p| p.value);
    assert_eq!(
        age,
        Some(700),
        "a message queued for a session waiting for input is aged"
    );
}

async fn stop_as(h: &H, user: &str) -> Result<Uuid, (StatusCode, String)> {
    handlers::stop(
        State(h.app.clone()),
        CurrentUser(user.into()),
        Path(h.launch.into()),
    )
    .await
    .map(|Json(v)| v.id)
    .map_err(|(s, Json(b))| (s, b["reason"].as_str().unwrap_or("").to_owned()))
}

/// ADR 0062: only the owner stops; a companion that cannot take it or is not live gets nothing,
/// so a stop never waits for a later Hello and never reaches a later session.
#[tokio::test]
async fn stop_is_owner_only_and_never_queued() {
    let h = harness().await;
    assert_eq!(
        stop_as(&h, OWNER).await.unwrap_err(),
        (StatusCode::CONFLICT, "companion_cannot_stop".into()),
        "a companion that did not announce session_stop"
    );
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT, capability::SESSION_STOP]);
    assert_eq!(
        stop_as(&h, "alice").await.unwrap_err(),
        (StatusCode::FORBIDDEN, "not_your_session".into())
    );
    assert!(pushed(&mut rx).is_empty(), "nothing pushed for alice");
    // Gone quiet: announced, but no envelope within two heartbeats.
    h.app.clock.set(t0() + Duration::minutes(10));
    assert_eq!(
        stop_as(&h, OWNER).await.unwrap_err(),
        (StatusCode::CONFLICT, "companion_offline".into())
    );
    let queued = store::queued(&h.app.db).await.unwrap();
    assert!(
        queued.is_empty(),
        "a refused stop is not queued: {}",
        queued.len()
    );
}

/// A stop goes out at once, even while a chat message is in flight, and is never pushed again.
#[tokio::test]
async fn stop_goes_past_a_message_in_flight() {
    let h = harness().await;
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT, capability::SESSION_STOP]);
    let chat = send(&h, OWNER, "working on it?").await.unwrap();
    assert_eq!(pushed(&mut rx).len(), 1);
    assert_eq!(state_of(&h, chat).await, SessionMessageState::Sent);
    let s = stop_as(&h, OWNER).await.unwrap();
    let got = pushed(&mut rx);
    assert_eq!(
        got,
        [(s, format!("Stopped by {OWNER}"))],
        "the stop, not the queue"
    );
    assert_eq!(state_of(&h, s).await, SessionMessageState::Sent);
    // The next dispatch does not push the stop a second time.
    dispatch::dispatch(&h.app, h.launch).await;
    assert!(pushed(&mut rx).iter().all(|(id, _)| *id != s));
}

/// The "Companions by capability" panel shows what the cutover waits for (QUICKSTART §3, #274):
/// `session_input`, `draft_mirror` and `plan_handoff`, not only the first. A capability nobody
/// announced reads 0, so the panel says it is missing rather than leaving it out.
#[tokio::test]
async fn the_capability_gauge_counts_what_planning_in_rubato_needs() {
    let planning = [
        capability::SESSION_INPUT,
        capability::DRAFT_MIRROR,
        capability::PLAN_HANDOFF,
    ];
    let h = harness().await;
    let _rx = connect_companion(&h, &planning);
    dispatch::record_gauges(&h.app, &[]).await;
    let by_cap: std::collections::BTreeMap<String, u64> = h
        .reader
        .points(metric::COMPANIONS_BY_CAPABILITY)
        .into_iter()
        .map(|p| (p.attrs[attr::CAPABILITY].clone(), p.value))
        .collect();
    for cap in planning {
        assert_eq!(by_cap.get(cap), Some(&1), "{cap} in {by_cap:?}");
    }
    let others: Vec<_> = by_cap
        .iter()
        .filter(|(c, _)| !planning.contains(&c.as_str()))
        .collect();
    assert!(others.iter().all(|(_, n)| **n == 0), "{others:?}");
    assert!(
        by_cap.contains_key(capability::ROOM_TURNS) && by_cap.contains_key(capability::HEADLESS),
        "every capability a companion announces is on the gauge: {by_cap:?}"
    );
    println!("checked {} capabilities on the gauge", by_cap.len());
}
