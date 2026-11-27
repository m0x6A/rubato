//! The lead's boundary (ADR 0072, plan 134 design §3). Alice runs lead L, worker A1 that L
//! launched, and A2 she started by hand; bob runs worker B. A lead acts only on the workers it
//! launched for its own owner: another person's session is `other_owner` (and pages), its owner's
//! other sessions are `not_led`, a request on another pairing is `identity`. A repeat never acts
//! twice, the flag off changes nothing, and notices tell a lead what its workers did.

use super::handle::{SKIP_TARGET_CHECK, on_request};
use super::{leads, live_workers, on_replay, on_sessions, store};
use crate::kernel::config::Config;
use crate::kernel::db::{connect, ts};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState, CompanionConn, LoadedMap};
use crate::kernel::telemetry::metric_reader;
use chrono::{Duration, TimeZone, Utc};
use rubato_protocol::fleet::reason;
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::{
    FleetAction, FleetOutcome, FleetRequest, FleetResult, IssueRef, LaunchCommand, LaunchRole,
    ServerMessage, SessionMessageKind, SessionState, SessionStatus, StateSource, SystemMap,
    capability,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

const REPO: &str = "m0x6A/rubato";

struct H {
    app: App,
    reader: metric_reader::Reader,
    /// Alice's and bob's companions.
    ca: Uuid,
    cb: Uuid,
    rx_a: mpsc::UnboundedReceiver<ServerMessage>,
    rx_b: mpsc::UnboundedReceiver<ServerMessage>,
    lead: Uuid,
    a1: Uuid,
    a2: Uuid,
    b: Uuid,
    _tmp: tempfile::TempDir,
}

fn t0() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap()
}

async fn launch_row(
    app: &App,
    companion: Uuid,
    user: &str,
    n: u64,
    role: LaunchRole,
    lead: Option<Uuid>,
) -> Uuid {
    let id = Uuid::new_v4();
    let cmd = LaunchCommand {
        id,
        user: user.into(),
        issue: IssueRef::new(REPO, n),
        repo_url: "https://github.com/m0x6A/rubato.git".into(),
        branch: format!("issue/{n}-x"),
        base_ref: "origin/main".into(),
        node_ids: vec![],
        spend_cap_usd: 10.0,
        attempt_limit: 3,
        attempt: 1,
        session_id: Uuid::new_v4(),
        prompt: "p".into(),
        otel_resource: Default::default(),
        map_version: None,
        plan_path: None,
        draft: None,
        planning: Default::default(),
        role,
        lead_launch_id: lead,
    };
    crate::launch::store_launch(
        &app.db,
        &crate::kernel::tenancy::WsKey::of_default("default"),
        companion,
        &cmd,
    )
    .await
    .unwrap();
    id
}

async fn harness_with(orchestrator: bool, fleet_max: u32) -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = connect(&url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut config = Config::for_tests(url, "alice");
    config.orchestrator = orchestrator;
    config.fleet_max = fleet_max;
    let mut app = AppState::new(pool, config, Arc::new(FakeGitHub::new()));
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    app.clock.set(t0());
    let text = include_str!("../../../../docs/design/rubato-map.yml");
    let map: SystemMap = serde_yaml_ng::from_str(text).unwrap();
    app.set_map(
        &crate::kernel::tenancy::WsKey::of_default(rubato_protocol::DEFAULT_WORKSPACE),
        LoadedMap {
            map,
            version: "v".into(),
            loaded_at: t0(),
            yaml: text.into(),
        },
    )
    .await;
    let (ca, cb) = (Uuid::new_v4(), Uuid::new_v4());
    let mut rx = Vec::new();
    for (c, user) in [(ca, "alice"), (cb, "bob")] {
        sqlx::query("INSERT INTO pairings (companion_id, user, token_hash, hostname, created_at) VALUES (?, ?, ?, 'h', '2026-11-01T00:00:00Z')")
            .bind(c.to_string())
            .bind(user)
            .bind(format!("hash-{user}"))
            .execute(&app.db)
            .await
            .unwrap();
        let (tx, r) = mpsc::unbounded_channel();
        app.companions
            .lock()
            .unwrap()
            .insert(c, CompanionConn::new(tx, t0()));
        app.set_capabilities(
            c,
            [
                capability::FLEET,
                capability::SESSION_INPUT,
                capability::SESSION_STOP,
                capability::HOOK_ALLOWLIST,
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        );
        app.touch_companion(c, t0());
        rx.push(r);
    }
    let rx_b = rx.pop().unwrap();
    let rx_a = rx.pop().unwrap();
    let lead = launch_row(&app, ca, "alice", 100, LaunchRole::Lead, None).await;
    let a1 = launch_row(&app, ca, "alice", 101, LaunchRole::Worker, Some(lead)).await;
    let a2 = launch_row(&app, ca, "alice", 102, LaunchRole::Worker, None).await;
    let b = launch_row(&app, cb, "bob", 103, LaunchRole::Worker, None).await;
    H {
        app,
        reader,
        ca,
        cb,
        rx_a,
        rx_b,
        lead,
        a1,
        a2,
        b,
        _tmp: tmp,
    }
}

async fn harness() -> H {
    harness_with(true, 4).await
}

fn req(lead: Uuid, action: FleetAction) -> FleetRequest {
    FleetRequest {
        request_id: Uuid::new_v4(),
        lead_launch_id: lead,
        action,
    }
}

fn message(to: Uuid, text: &str) -> FleetAction {
    FleetAction::Message {
        launch_id: to,
        text: text.into(),
    }
}

fn launch_of(n: u64, companion: Option<Uuid>) -> FleetAction {
    FleetAction::Launch {
        issue: IssueRef::new(REPO, n),
        companion_id: companion,
        slug: Some("worker".into()),
    }
}

async fn ask(h: &H, companion: Uuid, user: &str, r: &FleetRequest) -> FleetResult {
    on_request(&h.app, companion, user, r)
        .await
        .expect("a result")
}

#[track_caller]
fn refused(r: &FleetResult, why: &str) {
    assert_eq!(r.outcome, FleetOutcome::Refused, "{r:?}");
    assert_eq!(r.reason.as_deref(), Some(why), "{r:?}");
}

async fn messages_of(h: &H, launch: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM session_messages WHERE launch_id = ?")
        .bind(launch.to_string())
        .fetch_one(&h.app.db)
        .await
        .unwrap()
}

fn requests(h: &H, why: &str) -> u64 {
    h.reader
        .points(metric::FLEET_REQUESTS)
        .iter()
        .filter(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(why))
        .map(|p| p.value)
        .sum()
}

fn session(launch: Uuid, user: &str, status: SessionStatus) -> SessionState {
    SessionState {
        session_id: Some(launch.to_string()),
        companion_id: Uuid::nil(),
        user: user.into(),
        tmux: None,
        cwd: "/w".into(),
        worktree: None,
        branch: None,
        repo: Some(REPO.into()),
        issue: None,
        status,
        source: StateSource::Hook,
        last_seen: t0(),
        spend_usd: None,
        branch_drift: None,
        blocked_by: None,
        launch_id: Some(launch),
        hooks_lost: false,
    }
}

/// The line between people: a lead's message or stop to bob's session is refused `other_owner`,
/// nothing is queued or sent, and the refusal is counted.
async fn other_owner_is_refused(h: &mut H) {
    let m = ask(h, h.ca, "alice", &req(h.lead, message(h.b, "rm -rf"))).await;
    assert_eq!(
        m.reason.as_deref(),
        Some(reason::OTHER_OWNER),
        "other_owner: {m:?}"
    );
    let s = ask(
        h,
        h.ca,
        "alice",
        &req(h.lead, FleetAction::Stop { launch_id: h.b }),
    )
    .await;
    refused(&s, reason::OTHER_OWNER);
    assert_eq!(messages_of(h, h.b).await, 0, "nothing queued for bob");
    assert!(h.rx_b.try_recv().is_err(), "bob's companion got nothing");
    assert_eq!(requests(h, reason::OTHER_OWNER), 2);
    println!("checked 2 cross-owner requests");
}

#[tokio::test]
async fn a_lead_never_touches_another_persons_session() {
    let mut h = harness().await;
    other_owner_is_refused(&mut h).await;
}

/// Must fail on purpose: with the target check skipped, the same requests reach bob's session.
#[tokio::test]
#[should_panic(expected = "other_owner")]
async fn the_boundary_test_fails_without_the_target_check() {
    let mut h = harness().await;
    SKIP_TARGET_CHECK.with(|s| s.set(true));
    other_owner_is_refused(&mut h).await;
}

#[tokio::test]
async fn a_lead_acts_only_on_the_workers_it_launched() {
    let h = harness().await;
    // The owner's own hand-started session, and the lead itself: not_led, counted, no page.
    refused(
        &ask(&h, h.ca, "alice", &req(h.lead, message(h.a2, "hi"))).await,
        reason::NOT_LED,
    );
    refused(
        &ask(
            &h,
            h.ca,
            "alice",
            &req(h.lead, FleetAction::Stop { launch_id: h.lead }),
        )
        .await,
        reason::NOT_LED,
    );
    refused(
        &ask(
            &h,
            h.ca,
            "alice",
            &req(h.lead, message(Uuid::new_v4(), "hi")),
        )
        .await,
        reason::UNKNOWN_LAUNCH,
    );
    assert_eq!(messages_of(&h, h.a2).await, 0);
    // Its own worker: queued as alice, framed, via the lead.
    let r = ask(
        &h,
        h.ca,
        "alice",
        &req(h.lead, message(h.a1, "run the tests")),
    )
    .await;
    assert_eq!(r.outcome, FleetOutcome::Ok, "{r:?}");
    let (author, body, via): (String, String, Option<String>) =
        sqlx::query_as("SELECT author, body, via_launch FROM session_messages WHERE id = ?")
            .bind(r.message_id.unwrap().to_string())
            .fetch_one(&h.app.db)
            .await
            .unwrap();
    assert_eq!(author, "alice");
    assert_eq!(via, Some(h.lead.to_string()));
    let kind: SessionMessageKind = serde_json::from_str(&body).unwrap();
    let text = kind.text();
    assert!(
        text.starts_with("From alice's lead agent (launch "),
        "{text}"
    );
    assert!(text.ends_with("run the tests"), "{text}");
    // Empty, and too long with the framing line counted.
    refused(
        &ask(&h, h.ca, "alice", &req(h.lead, message(h.a1, " "))).await,
        reason::EMPTY,
    );
    let max = h.app.config.limits.session_message_max_chars as usize;
    let near = "x".repeat(max - 10);
    refused(
        &ask(&h, h.ca, "alice", &req(h.lead, message(h.a1, &near))).await,
        reason::TOO_LONG,
    );
}

#[tokio::test]
async fn a_lead_stops_its_own_worker_through_the_stop_path() {
    let mut h = harness().await;
    let r = ask(
        &h,
        h.ca,
        "alice",
        &req(h.lead, FleetAction::Stop { launch_id: h.a1 }),
    )
    .await;
    assert_eq!(r.outcome, FleetOutcome::Ok, "{r:?}");
    match h.rx_a.try_recv() {
        Ok(ServerMessage::SessionMessage(m)) => {
            assert!(matches!(m.kind, SessionMessageKind::Stop { .. }), "{m:?}")
        }
        other => panic!("a stop to alice's companion: {other:?}"),
    }
    let via: Option<String> =
        sqlx::query_scalar("SELECT via_launch FROM session_messages WHERE launch_id = ?")
            .bind(h.a1.to_string())
            .fetch_one(&h.app.db)
            .await
            .unwrap();
    assert_eq!(via, Some(h.lead.to_string()));
}

#[tokio::test]
async fn identity_comes_from_the_pairing_and_the_stored_launch() {
    let h = harness().await;
    // Bob's socket naming alice's lead: identity, which pages.
    refused(
        &ask(&h, h.cb, "bob", &req(h.lead, FleetAction::List)).await,
        reason::IDENTITY,
    );
    // A worker naming itself: not a lead.
    refused(
        &ask(&h, h.ca, "alice", &req(h.a1, FleetAction::List)).await,
        reason::NOT_A_LEAD,
    );
    // A launch nobody made.
    refused(
        &ask(&h, h.ca, "alice", &req(Uuid::new_v4(), FleetAction::List)).await,
        reason::IDENTITY,
    );
    assert_eq!(requests(&h, reason::IDENTITY), 2);
}

#[tokio::test]
async fn the_list_is_the_owners_own_launches_and_never_anyone_elses() {
    let h = harness().await;
    h.app
        .set_sessions(h.ca, vec![session(h.a1, "alice", SessionStatus::Working)]);
    h.app
        .set_sessions(h.cb, vec![session(h.b, "bob", SessionStatus::Working)]);
    let r = ask(&h, h.ca, "alice", &req(h.lead, FleetAction::List)).await;
    let fleet = r.fleet.unwrap();
    let ids: Vec<Uuid> = fleet.iter().map(|e| e.launch_id).collect();
    for want in [h.lead, h.a1, h.a2] {
        assert!(ids.contains(&want), "{want} in {ids:?}");
    }
    assert!(!ids.contains(&h.b), "bob's launch must never be listed");
    let a1 = fleet.iter().find(|e| e.launch_id == h.a1).unwrap();
    assert_eq!(a1.state, Some(SessionStatus::Working));
    assert_eq!(a1.started_by, format!("lead:{}", h.lead));
    let a2 = fleet.iter().find(|e| e.launch_id == h.a2).unwrap();
    assert_eq!(a2.started_by, "person");
    assert_eq!(a2.state, None, "not in a snapshot: unknown");
    println!("listed {} launches", fleet.len());
}

#[tokio::test]
async fn a_launch_stays_on_the_owners_companions_and_off_other_peoples_issues() {
    let mut h = harness().await;
    // Bob's companion: refused by the ordinary launch path, nothing sent to bob.
    let r = ask(&h, h.ca, "alice", &req(h.lead, launch_of(200, Some(h.cb)))).await;
    refused(&r, reason::LAUNCH_REFUSED);
    // An issue bob launched: someone else's.
    let r = ask(&h, h.ca, "alice", &req(h.lead, launch_of(103, Some(h.ca)))).await;
    refused(&r, reason::SOMEONE_ELSES_ISSUE);
    assert!(h.rx_b.try_recv().is_err());
}

#[tokio::test]
async fn a_pinned_issue_launches_a_worker_that_names_its_lead() {
    let mut h = harness().await;
    sqlx::query("INSERT INTO pins (tenant_id, workspace_id, repo, number, kind, target, pinned_by, pinned_at) VALUES ('default', 'default', ?, 300, 'node', 'rubato-server', 'alice', ?)")
        .bind(REPO)
        .bind(ts(t0()))
        .execute(&h.app.db)
        .await
        .unwrap();
    let r = ask(&h, h.ca, "alice", &req(h.lead, launch_of(300, Some(h.ca)))).await;
    assert_eq!(r.outcome, FleetOutcome::Ok, "{r:?}");
    match h.rx_a.try_recv() {
        Ok(ServerMessage::Launch(cmd)) => {
            assert_eq!(Some(cmd.id), r.launch_id);
            assert_eq!(cmd.user, "alice");
            assert_eq!(cmd.role, LaunchRole::Worker);
            assert_eq!(cmd.lead_launch_id, Some(h.lead));
            assert!(
                cmd.prompt.contains("started by alice's lead agent"),
                "{}",
                cmd.prompt
            );
        }
        other => panic!("a launch to alice's companion: {other:?}"),
    }
    assert_eq!(
        live_workers(&h.app, h.lead).await.unwrap(),
        2,
        "A1 and the new one, both pending"
    );
}

#[tokio::test]
async fn a_full_fleet_launches_no_more() {
    let h = harness_with(true, 1).await;
    // A1 has no result yet: it counts as live.
    let r = ask(&h, h.ca, "alice", &req(h.lead, launch_of(300, None))).await;
    refused(&r, reason::FLEET_FULL);
}

#[tokio::test]
async fn a_repeat_never_acts_twice() {
    let mut h = harness().await;
    let first = req(h.lead, message(h.a1, "once"));
    let a = ask(&h, h.ca, "alice", &first).await;
    let b = ask(&h, h.ca, "alice", &first).await;
    assert_eq!(a, b, "the stored answer");
    assert_eq!(
        messages_of(&h, h.a1).await,
        1,
        "one message for two requests"
    );
    // The same id with another action.
    let other = FleetRequest {
        action: FleetAction::Stop { launch_id: h.a1 },
        ..first.clone()
    };
    refused(
        &ask(&h, h.ca, "alice", &other).await,
        reason::REQUEST_ID_REUSED,
    );
    // A replayed envelope gets the stored answer and does nothing.
    assert_eq!(on_replay(&h.app, h.ca, &first).await, Some(a));
    assert_eq!(messages_of(&h, h.a1).await, 1);
    // A claim never finished: in progress, then unknown after a minute; never acted on.
    let stuck = req(h.lead, FleetAction::Stop { launch_id: h.a1 });
    store::claim(&h.app.db, h.ca, "alice", &stuck, h.app.clock.now())
        .await
        .unwrap();
    assert_eq!(
        ask(&h, h.ca, "alice", &stuck).await.outcome,
        FleetOutcome::InProgress
    );
    h.app.clock.set(t0() + Duration::seconds(61));
    assert_eq!(
        ask(&h, h.ca, "alice", &stuck).await.outcome,
        FleetOutcome::Unknown
    );
    // Alice's companion got the one chat, and no stop.
    let mut sent = Vec::new();
    while let Ok(m) = h.rx_a.try_recv() {
        sent.push(m);
    }
    assert!(
        !sent.iter().any(|m| matches!(m, ServerMessage::SessionMessage(s) if matches!(s.kind, SessionMessageKind::Stop { .. }))),
        "the stuck stop was never sent: {sent:?}"
    );
    assert!(store::count(&h.app.db).await >= 2);
}

#[tokio::test]
async fn with_the_flag_off_nothing_happens() {
    let h = harness_with(false, 4).await;
    let r = ask(&h, h.ca, "alice", &req(h.lead, message(h.a1, "hi"))).await;
    refused(&r, reason::ORCHESTRATOR_OFF);
    assert_eq!(messages_of(&h, h.a1).await, 0);
    assert_eq!(store::count(&h.app.db).await, 0, "not even a claim");
    assert!(
        leads(
            &h.app,
            &crate::kernel::tenancy::WsKey::of_default("default")
        )
        .await
        .is_empty()
    );
    // A companion that did not announce `fleet` is sent nothing at all.
    let h = harness().await;
    h.app
        .set_capabilities(h.ca, vec![capability::SESSION_INPUT.to_owned()]);
    assert_eq!(
        on_request(&h.app, h.ca, "alice", &req(h.lead, FleetAction::List)).await,
        None
    );
}

async fn notices(h: &H) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT body FROM session_messages WHERE launch_id = ? AND author = 'rubato' ORDER BY created_at",
    )
    .bind(h.lead.to_string())
    .fetch_all(&h.app.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_lead_hears_when_its_workers_end_block_or_go_idle() {
    let h = harness().await;
    let w = |l, s| session(l, "alice", s);
    on_sessions(
        &h.app,
        &[
            w(h.a1, SessionStatus::Working),
            w(h.a2, SessionStatus::Working),
        ],
    )
    .await;
    assert!(notices(&h).await.is_empty(), "working is not news");
    on_sessions(
        &h.app,
        &[w(h.a1, SessionStatus::Idle), w(h.a2, SessionStatus::Idle)],
    )
    .await;
    let got = notices(&h).await;
    assert_eq!(got.len(), 1, "only A1 is this lead's: {got:?}");
    assert!(
        got[0].contains("is idle") && got[0].contains("#101"),
        "{}",
        got[0]
    );
    // Two more changes within a minute: held, then one notice listing both.
    on_sessions(&h.app, &[w(h.a1, SessionStatus::Blocked)]).await;
    on_sessions(&h.app, &[w(h.a1, SessionStatus::Ended)]).await;
    assert_eq!(notices(&h).await.len(), 1, "held for the minute");
    h.app.clock.set(t0() + Duration::seconds(61));
    on_sessions(&h.app, &[]).await;
    let got = notices(&h).await;
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        got[1].contains("is blocked") && got[1].contains("ended"),
        "{}",
        got[1]
    );
    // The leads view: L with its live worker.
    h.app.set_sessions(
        h.ca,
        vec![
            w(h.lead, SessionStatus::Idle),
            w(h.a1, SessionStatus::Working),
        ],
    );
    let l = leads(
        &h.app,
        &crate::kernel::tenancy::WsKey::of_default("default"),
    )
    .await;
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].launch_id, h.lead);
    assert_eq!(l[0].workers, vec![h.a1]);
    // With the flag off: no notice.
    let off = harness_with(false, 4).await;
    on_sessions(
        &off.app,
        &[session(off.a1, "alice", SessionStatus::Working)],
    )
    .await;
    on_sessions(&off.app, &[session(off.a1, "alice", SessionStatus::Ended)]).await;
    assert!(notices(&off).await.is_empty());
}

async fn pin(h: &H, n: u64) {
    sqlx::query("INSERT INTO pins (tenant_id, workspace_id, repo, number, kind, target, pinned_by, pinned_at) VALUES ('default', 'default', ?, ?, 'node', 'rubato-server', 'alice', ?)")
        .bind(REPO)
        .bind(n as i64)
        .bind(ts(t0()))
        .execute(&h.app.db)
        .await
        .unwrap();
}

fn lead_request(n: u64, companion: Uuid) -> rubato_protocol::LaunchRequest {
    rubato_protocol::LaunchRequest {
        issue: IssueRef::new(REPO, n),
        companion_id: Some(companion),
        slug: Some("lead".into()),
        branch: None,
        spend_cap_usd: None,
        attempt_limit: None,
        prompt: None,
        role: LaunchRole::Lead,
    }
}

/// A lead starts only with the server's flag on, only on a companion that announced `fleet`,
/// and never at a lead's own request (ADR 0072).
#[tokio::test]
async fn a_lead_launch_needs_both_flags() {
    let off = harness_with(false, 4).await;
    pin(&off, 400).await;
    let e = crate::launch::start(
        &off.app,
        &crate::kernel::tenancy::WsKey::of_default("default"),
        "alice",
        lead_request(400, off.ca),
        None,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(e.0, axum::http::StatusCode::CONFLICT);
    assert!(e.1.starts_with(reason::ORCHESTRATOR_OFF), "{}", e.1);

    let mut h = harness().await;
    pin(&h, 400).await;
    h.app
        .set_capabilities(h.ca, vec![capability::SESSION_INPUT.to_owned()]);
    let e = crate::launch::start(
        &h.app,
        &crate::kernel::tenancy::WsKey::of_default("default"),
        "alice",
        lead_request(400, h.ca),
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(e.1.starts_with(reason::COMPANION_CANNOT_LEAD), "{}", e.1);
    // A lead asking for a lead: refused.
    h.app.set_capabilities(
        h.ca,
        vec![
            capability::FLEET.to_owned(),
            capability::SESSION_INPUT.to_owned(),
        ],
    );
    let e = crate::launch::start(
        &h.app,
        &crate::kernel::tenancy::WsKey::of_default("default"),
        "alice",
        lead_request(400, h.ca),
        None,
        Some(h.lead),
    )
    .await
    .unwrap_err();
    assert!(e.1.starts_with(reason::ORCHESTRATOR_OFF), "{}", e.1);
    // Both flags on: a lead with the lead prompt.
    let ok = crate::launch::start(
        &h.app,
        &crate::kernel::tenancy::WsKey::of_default("default"),
        "alice",
        lead_request(400, h.ca),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(ok.command.role, LaunchRole::Lead);
    assert!(
        ok.command.prompt.contains("fleet_launch"),
        "{}",
        ok.command.prompt
    );
    assert!(matches!(h.rx_a.try_recv(), Ok(ServerMessage::Launch(_))));
    let t = crate::launch::launch_target(&h.app.db, ok.command.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(t.role, LaunchRole::Lead);
}
