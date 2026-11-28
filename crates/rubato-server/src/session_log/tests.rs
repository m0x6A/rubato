//! The session log from hooks (ADR 0052): one prompt and one reply per `prompt_id`, the session
//! message a prompt carried, reviews and summaries by their first line, and nothing from an
//! older companion or a session Rubato did not launch.

use super::{handlers, intake};
use crate::auth::CurrentUser;
use crate::kernel::config::Config;
use crate::kernel::db::{connect, ts};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState};
use crate::kernel::telemetry::metric_reader;
use axum::extract::{Path, State};
use chrono::{TimeZone, Utc};
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::{HookEvent, LogKind, LogRole, capability};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

struct H {
    app: App,
    reader: metric_reader::Reader,
    launch: Uuid,
    companion: Uuid,
    _tmp: tempfile::TempDir,
}

async fn harness(caps: &[&str]) -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = connect(&url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut app = AppState::new(
        pool,
        Config::for_tests(url, "m0x6A"),
        Arc::new(FakeGitHub::new()),
    );
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let (launch, companion) = (Uuid::new_v4(), Uuid::new_v4());
    sqlx::query("INSERT INTO launches (tenant_id, workspace_id, id, companion_id, user, repo, number, command, created_at) VALUES ('default', 'default', ?, ?, 'm0x6A', 'm0x6A/rubato', 122, '{}', ?)")
        .bind(launch.to_string())
        .bind(companion.to_string())
        .bind(ts(Utc::now()))
        .execute(&app.db)
        .await
        .unwrap();
    app.set_capabilities(companion, caps.iter().map(|c| (*c).to_owned()).collect());
    H {
        app,
        reader,
        launch,
        companion,
        _tmp: tmp,
    }
}

/// A hook shaped as Claude Code sends it (design §Spike), as the companion forwards it.
fn hook(
    h: &H,
    name: &str,
    prompt_id: Option<&str>,
    field: Option<(&str, &str)>,
    secs: u32,
) -> HookEvent {
    let mut p = json!({"session_id": "s1", "hook_event_name": name, "cwd": "/w"});
    if let Some(id) = prompt_id {
        p["prompt_id"] = json!(id);
    }
    if let Some((k, v)) = field {
        p[k] = json!(v);
    }
    let at = Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, secs).unwrap();
    let mut ev = HookEvent::from_payload(p, at).unwrap();
    ev.launch_id = Some(h.launch);
    ev
}

async fn entries(h: &H) -> Vec<rubato_protocol::SessionLogEntry> {
    handlers::log(
        State(h.app.clone()),
        crate::kernel::tenancy::default_scope(),
        CurrentUser("alice".into()),
        Path(h.launch.into()),
    )
    .await
    .unwrap()
    .0
    .entries
}

fn dropped(h: &H, reason: &str) -> u64 {
    h.reader
        .points(metric::SESSION_LOG_DROPPED)
        .iter()
        .filter(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(reason))
        .map(|p| p.value)
        .sum()
}

#[tokio::test]
async fn a_turn_is_its_prompt_and_its_last_reply_joined_by_prompt_id() {
    let h = harness(&[capability::HOOK_ALLOWLIST]).await;
    let msg = Uuid::new_v4();
    let mut prompt = hook(
        &h,
        "UserPromptSubmit",
        Some("p1"),
        Some(("prompt", "half Split D7, please")),
        0,
    );
    prompt.message_id = Some(msg);
    prompt.mixed = true;
    intake::on_hook(&h.app, h.companion, &prompt).await;
    let reply = hook(
        &h,
        "Stop",
        Some("p1"),
        Some(("last_assistant_message", "Done: D7 is two sections.")),
        5,
    );
    intake::on_hook(&h.app, h.companion, &reply).await;
    // A replayed hook writes nothing new.
    intake::on_hook(&h.app, h.companion, &reply).await;
    let e = entries(&h).await;
    assert_eq!(e.len(), 2, "{e:?}");
    assert_eq!(
        (e[0].role, e[0].kind, e[0].message_id, e[0].mixed),
        (LogRole::User, LogKind::Prompt, Some(msg), true)
    );
    assert_eq!(
        (e[1].role, e[1].kind, e[1].text.as_str()),
        (
            LogRole::Assistant,
            LogKind::Reply,
            "Done: D7 is two sections."
        )
    );
    assert_eq!(e[0].prompt_id, e[1].prompt_id);
    let stored: u64 = h
        .reader
        .points(metric::SESSION_LOG_ENTRIES)
        .iter()
        .map(|p| p.value)
        .sum();
    assert_eq!(stored, 2);
}

#[tokio::test]
async fn reviews_and_summaries_are_known_by_their_first_line() {
    let h = harness(&[capability::HOOK_ALLOWLIST]).await;
    for (i, text) in [
        "Review: full — passed after fixes\n\n## Plan",
        "Summary: we split D7",
    ]
    .iter()
    .enumerate()
    {
        let id = format!("p{i}");
        intake::on_hook(
            &h.app,
            h.companion,
            &hook(
                &h,
                "Stop",
                Some(&id),
                Some(("last_assistant_message", text)),
                i as u32,
            ),
        )
        .await;
    }
    let kinds: Vec<LogKind> = entries(&h).await.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [LogKind::Review, LogKind::Summary]);
}

#[tokio::test]
async fn a_stop_without_text_or_prompt_id_writes_nothing_and_is_counted() {
    let h = harness(&[capability::HOOK_ALLOWLIST]).await;
    intake::on_hook(&h.app, h.companion, &hook(&h, "Stop", Some("p1"), None, 0)).await;
    intake::on_hook(
        &h.app,
        h.companion,
        &hook(&h, "Stop", None, Some(("last_assistant_message", "x")), 1),
    )
    .await;
    assert!(entries(&h).await.is_empty());
    assert_eq!(dropped(&h, "no_text"), 1);
    assert_eq!(dropped(&h, "no_prompt_id"), 1);
}

#[tokio::test]
async fn nothing_from_an_older_companion_another_companion_or_an_unlaunched_session() {
    // Fails on purpose without the capability check: an older companion forwards full payloads.
    let h = harness(&[]).await;
    intake::on_hook(
        &h.app,
        h.companion,
        &hook(
            &h,
            "UserPromptSubmit",
            Some("p1"),
            Some(("prompt", "secret")),
            0,
        ),
    )
    .await;
    assert!(
        entries(&h).await.is_empty(),
        "an older companion's prompt entered the log"
    );
    let h = harness(&[capability::HOOK_ALLOWLIST]).await;
    let mut unlaunched = hook(
        &h,
        "UserPromptSubmit",
        Some("p1"),
        Some(("prompt", "mine")),
        0,
    );
    unlaunched.launch_id = None;
    intake::on_hook(&h.app, h.companion, &unlaunched).await;
    let other = Uuid::new_v4();
    h.app
        .set_capabilities(other, vec![capability::HOOK_ALLOWLIST.to_owned()]);
    intake::on_hook(
        &h.app,
        other,
        &hook(
            &h,
            "UserPromptSubmit",
            Some("p2"),
            Some(("prompt", "theirs")),
            1,
        ),
    )
    .await;
    assert!(entries(&h).await.is_empty());
    assert_eq!(
        dropped(&h, "unknown_launch"),
        1,
        "another companion's claim on this launch"
    );
}
