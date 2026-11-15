//! The handoff (ADR 0055, plan 122 §D6–§D7): `plan_approved` queued at priority 0 for the
//! owner's live launched session and pushed before a chat queued earlier; a handoff report taken
//! only from the launch's own companion for its issue; `handoff_overdue` counting only approvals
//! still current; the record match, the link body, ADRs left.

use super::handoff::refused;
use super::link::{Due, LinkFacts, assess, keys_match, link_body};
use super::links::{LinkRow, LinkState};
use super::records::{self, HandoffFacts, RubatoApproval};
use super::{approval_url, approve_draft, on_handoff_report, reconcile_links};
use crate::auth::CurrentUser;
use crate::kernel::config::Config;
use crate::kernel::db::{connect, ts};
use crate::kernel::github::FakeGitHub;
use crate::kernel::http::Ws;
use crate::kernel::state::{App, AppState, CompanionConn, LoadedMap};
use crate::kernel::telemetry::{capture, metric_reader};
use crate::plans::PlanRow;
use crate::{github_poll, plan_drafts, session_messages};
use axum::Json;
use axum::extract::{Path, State};
use chrono::{DateTime, Duration, SubsecRound, TimeZone, Utc};
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::{
    DraftApproveRequest, DraftBundle, DraftFile, Finding, HandoffLine, HandoffOutcome,
    HandoffReport, IssueRef, LaunchCommand, PlanApprovalRecord, ServerMessage, SessionMessageKind,
    SessionMessageOutcome, SessionMessageRequest, SessionMessageResult, SessionState,
    SessionStatus, StateSource, SystemMap, approval_hash, bundle_hash, capability,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

const OWNER: &str = "m0x6A";
const BRANCH: &str = "issue/122-x";
const PLAN: &str = "docs/plans/122.md";

fn issue() -> IssueRef {
    IssueRef::new("m0x6A/rubato", 122)
}

fn plan_text(change: &str) -> String {
    format!(
        "---\nissue: m0x6A/rubato#122\n---\n# Plan\n\n## Affected parts\n\n| Node | Status | What changes | Seams |\n| --- | --- | --- | --- |\n| rubato-server | affected | {change} | web-server |\n\n## O-rows\n\n| Seam | Breaks like | Signal | Panel | Owner |\n| --- | --- | --- | --- | --- |\n| web-server | x | y | z | rubato-server |\n"
    )
}

struct H {
    app: App,
    reader: metric_reader::Reader,
    launch: Uuid,
    companion: Uuid,
    _tmp: tempfile::TempDir,
}

/// A cutover an hour ago, the map loaded, and one launch of #122 on `issue/122-x` by the owner.
async fn harness() -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = connect(&url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut cfg = Config::for_tests(url, OWNER);
    cfg.comment_approvals_until = Some(Utc::now() - Duration::hours(1));
    cfg.public_url = "https://rubato.example/".into();
    let mut app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let text = include_str!("../../../../docs/design/rubato-map.yml");
    let map: SystemMap = serde_yaml_ng::from_str(text).unwrap();
    app.set_map(
        rubato_protocol::DEFAULT_WORKSPACE,
        LoadedMap {
            map,
            version: "v".into(),
            loaded_at: Utc::now(),
            yaml: text.into(),
        },
    )
    .await;
    let launch = Uuid::from_u128(1);
    let companion = Uuid::from_u128(2);
    insert_launch(&app, launch, companion, &issue()).await;
    H {
        app,
        reader,
        launch,
        companion,
        _tmp: tmp,
    }
}

async fn insert_launch(app: &App, launch: Uuid, companion: Uuid, issue: &IssueRef) {
    let cmd = LaunchCommand {
        id: launch,
        user: OWNER.into(),
        issue: issue.clone(),
        repo_url: "https://github.com/m0x6A/rubato.git".into(),
        branch: BRANCH.into(),
        base_ref: "origin/main".into(),
        node_ids: vec![],
        spend_cap_usd: 10.0,
        attempt_limit: 3,
        attempt: 1,
        session_id: Uuid::nil(),
        prompt: "p".into(),
        otel_resource: Default::default(),
        map_version: None,
        plan_path: Some(PLAN.into()),
        draft: None,
        planning: Default::default(),
    };
    sqlx::query("INSERT INTO launches (workspace_id, id, companion_id, user, repo, number, command, created_at) VALUES ('default', ?, ?, ?, ?, ?, ?, ?)")
        .bind(launch.to_string())
        .bind(companion.to_string())
        .bind(OWNER)
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .bind(serde_json::to_string(&cmd).unwrap())
        .bind(ts(Utc::now()))
        .execute(&app.db)
        .await
        .unwrap();
}

/// The owner's launched session, waiting for input.
fn idle_session(h: &H) -> SessionState {
    SessionState {
        session_id: Some(Uuid::nil().to_string()),
        companion_id: h.companion,
        user: OWNER.into(),
        tmux: None,
        cwd: "/w".into(),
        worktree: None,
        branch: Some(BRANCH.into()),
        repo: Some("m0x6A/rubato".into()),
        issue: Some(issue()),
        status: SessionStatus::Idle,
        source: StateSource::Hook,
        last_seen: Utc::now(),
        spend_usd: None,
        branch_drift: None,
        blocked_by: None,
        launch_id: Some(h.launch),
        hooks_lost: false,
    }
}

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

fn pushed(rx: &mut mpsc::UnboundedReceiver<ServerMessage>) -> Vec<(Uuid, SessionMessageKind)> {
    let mut out = Vec::new();
    while let Ok(m) = rx.try_recv() {
        if let ServerMessage::SessionMessage(m) = m {
            out.push((m.id, m.kind));
        }
    }
    out
}

async fn mirror(h: &H, text: &str) {
    let files = vec![DraftFile {
        path: PLAN.into(),
        text: text.into(),
    }];
    plan_drafts::on_draft(
        &h.app,
        &DraftBundle {
            launch_id: h.launch,
            session_id: None,
            issue: issue(),
            branch: BRANCH.into(),
            bundle_hash: bundle_hash(PLAN, &[(PLAN.into(), text.into())]),
            files,
            missing: vec![],
            refused: None,
            at: Utc::now(),
        },
        OWNER,
    )
    .await;
}

async fn approve_as(h: &H, user: &str, text: &str) -> rubato_protocol::DraftApproveResponse {
    approve_draft(
        State(h.app.clone()),
        Ws(rubato_protocol::DEFAULT_WORKSPACE.into()),
        CurrentUser(user.into()),
        Json(DraftApproveRequest {
            repo: "m0x6A/rubato".into(),
            number: 122,
            branch: BRANCH.into(),
            plan_sha: approval_hash(text),
            rev: None,
            summary: String::new(),
        }),
    )
    .await
    .map_err(|(s, j)| format!("{s} {}", j.0))
    .unwrap()
    .1
    .0
}

async fn chat(h: &H, text: &str) -> Uuid {
    session_messages::send(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Path(h.launch),
        Json(SessionMessageRequest { text: text.into() }),
    )
    .await
    .map_err(|(s, j)| format!("{s} {}", j.0))
    .unwrap()
    .0
    .id
}

/// ADR 0055: a new approval queues `plan_approved` at priority 0 for the owner's live launched
/// session, carrying every field the companion types; it is pushed before a chat queued earlier.
#[tokio::test]
async fn plan_approved_is_queued_first_for_the_owners_live_session() {
    let h = harness().await;
    let text = plan_text("one");
    mirror(&h, &text).await;
    h.app.set_sessions(h.companion, vec![idle_session(&h)]);
    // The companion is away: the chat waits in the queue.
    let earlier = chat(&h, "and keep the table").await;
    let a = approve_as(&h, "teammate", &text).await;
    // Back, able to hand off: the approval goes first, then the chat after the next idle hook.
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT, capability::PLAN_HANDOFF]);
    session_messages::dispatch(&h.app, h.launch).await;
    let first = pushed(&mut rx);
    assert_eq!(first.len(), 1, "one in flight");
    let (id, kind) = &first[0];
    assert_ne!(*id, earlier, "the chat was queued first, but goes second");
    assert_eq!(
        *kind,
        SessionMessageKind::PlanApproved {
            approval_id: a.approval_id,
            approved_by: "teammate".into(),
            approved_at: a.approved_at,
            approved_sha: approval_hash(&text),
            bundle_hash: a.bundle_hash.clone(),
            branch: BRANCH.into(),
            plan_path: PLAN.into(),
            self_approved: false,
            url: format!("https://rubato.example/approvals/{}", a.approval_id),
        }
    );
    assert_eq!(
        approval_url(&h.app, a.approval_id),
        format!("https://rubato.example/approvals/{}", a.approval_id)
    );
    assert_eq!(
        a.approved_at,
        a.approved_at.trunc_subsecs(0),
        "whole seconds, as the handoff writes it"
    );
    session_messages::on_result(
        &h.app,
        h.companion,
        &SessionMessageResult {
            message_id: *id,
            outcome: SessionMessageOutcome::Delivered,
            at: Utc::now(),
        },
    )
    .await;
    session_messages::dispatch(&h.app, h.launch).await;
    assert_eq!(
        pushed(&mut rx).iter().map(|p| p.0).collect::<Vec<_>>(),
        [earlier]
    );
}

/// A companion that cannot hand off is never sent `plan_approved`: it waits in the queue and the
/// chat behind it goes. With no live launched session nothing is queued: the approval waits.
#[tokio::test]
async fn plan_approved_waits_for_a_companion_that_can_type_it_and_for_a_live_session() {
    let h = harness().await;
    let one = plan_text("one");
    mirror(&h, &one).await;
    h.app.set_sessions(h.companion, vec![idle_session(&h)]);
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT]);
    approve_as(&h, "teammate", &one).await;
    assert!(
        pushed(&mut rx).is_empty(),
        "held for a companion with plan_handoff"
    );
    let c = chat(&h, "hello").await;
    assert_eq!(pushed(&mut rx).iter().map(|p| p.0).collect::<Vec<_>>(), [c]);

    // No live session of the draft: the approval is recorded and nothing is queued.
    let h = harness().await;
    let two = plan_text("two");
    mirror(&h, &two).await;
    let mut ended = idle_session(&h);
    ended.status = SessionStatus::Ended;
    let mut elsewhere = idle_session(&h);
    elsewhere.branch = Some("issue/122-y".into());
    h.app.set_sessions(h.companion, vec![ended, elsewhere]);
    let (events, _guard) = capture::start();
    approve_as(&h, "teammate", &two).await;
    let queued = session_messages::list(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Path(h.launch),
    )
    .await
    .unwrap()
    .0
    .messages;
    assert!(queued.is_empty(), "{queued:?}");
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.message == "plan approval waits for a launch of the draft")
    );
}

fn report(
    launch: Uuid,
    approval: Uuid,
    outcome: HandoffOutcome,
    adrs_left: &[&str],
) -> HandoffReport {
    HandoffReport {
        launch_id: launch,
        line: HandoffLine {
            event: "rubato.plan.handoff".into(),
            outcome,
            approval,
            commit: Some("c0ffee1".into()),
            adrs_accepted: vec![],
            adrs_left: adrs_left.iter().map(|a| (*a).to_owned()).collect(),
            error: None,
        },
        at: Utc::now(),
    }
}

fn dropped(h: &H, why: &str) -> u64 {
    h.reader
        .points(metric::EVENTS_DROPPED)
        .iter()
        .filter(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(why))
        .map(|p| p.value)
        .sum()
}

/// ADR 0055: a handoff report is recorded on the approval it names only from the launch's own
/// companion, and only when that launch's issue is the approval's.
#[tokio::test]
async fn a_handoff_report_is_taken_only_from_the_launchs_companion_for_its_issue() {
    let h = harness().await;
    let text = plan_text("one");
    mirror(&h, &text).await;
    let a = approve_as(&h, "teammate", &text).await;
    let db = h.app.db.clone();
    let facts = || {
        let db = db.clone();
        async move {
            records::get(&db, a.approval_id)
                .await
                .unwrap()
                .unwrap()
                .handoff
        }
    };
    let r = report(
        h.launch,
        a.approval_id,
        HandoffOutcome::Committed,
        &["0050"],
    );
    assert!(
        !on_handoff_report(&h.app, Uuid::from_u128(99), &r).await,
        "another companion"
    );
    assert_eq!(dropped(&h, refused::FOREIGN_LAUNCH), 1);
    // The companion's own launch, of another issue.
    let other = Uuid::from_u128(3);
    insert_launch(
        &h.app,
        other,
        h.companion,
        &IssueRef::new("m0x6A/rubato", 71),
    )
    .await;
    let r71 = report(other, a.approval_id, HandoffOutcome::Committed, &[]);
    assert!(!on_handoff_report(&h.app, h.companion, &r71).await);
    assert_eq!(dropped(&h, refused::OTHER_ISSUE), 1);
    let unknown = report(
        h.launch,
        Uuid::from_u128(77),
        HandoffOutcome::Committed,
        &[],
    );
    assert!(!on_handoff_report(&h.app, h.companion, &unknown).await);
    assert_eq!(dropped(&h, refused::UNKNOWN_APPROVAL), 1);
    assert_eq!(facts().await, None, "nothing recorded");

    let (events, _guard) = capture::start();
    assert!(on_handoff_report(&h.app, h.companion, &r).await);
    let got = facts().await.unwrap();
    assert_eq!(
        (
            got.outcome.as_str(),
            got.commit.as_deref(),
            got.adrs_left.as_slice()
        ),
        ("committed", Some("c0ffee1"), ["0050".to_owned()].as_slice())
    );
    let events = events.lock().unwrap();
    let line = events
        .iter()
        .find(|e| e.message == "handoff reported")
        .expect("handoff reported");
    assert_eq!(line.level, tracing::Level::INFO);
    assert_eq!(
        line.fields.get("rubato.outcome").map(String::as_str),
        Some("committed")
    );
}

fn approval(n: u128, sha: &str, handoff: Option<&str>) -> RubatoApproval {
    RubatoApproval {
        id: Uuid::from_u128(n),
        issue: IssueRef::new("m0x6A/rubato", n as u64),
        branch: format!("issue/{n}-x"),
        draft_id: Uuid::from_u128(1000 + n),
        rev: 1,
        plan_sha: sha.into(),
        bundle_hash: "b".into(),
        approved_by: "teammate".into(),
        approved_at: Utc.with_ymd_and_hms(2026, 11, 20, 10, 0, 0).unwrap(),
        self_approved: false,
        summary: String::new(),
        handoff: handoff.map(|o| HandoffFacts {
            outcome: o.into(),
            at: Utc::now(),
            commit: None,
            adrs_left: vec![],
        }),
    }
}

/// The plan at the head of an approval's branch, with its record written in.
fn handed_off(a: &RubatoApproval, hash: &str) -> PlanRow {
    let mut plan: rubato_protocol::Plan = serde_json::from_value(serde_json::json!({
        "issue": a.issue, "path": "docs/plans/x.md", "affected_parts": [], "o_rows": []
    }))
    .unwrap();
    plan.approval = Some(PlanApprovalRecord {
        approved_by: Some(a.approved_by.clone()),
        approved_at: Some("2026-11-20T10:00:00Z".into()),
        approved_sha: Some(a.plan_sha.clone()),
    });
    PlanRow {
        issue: a.issue.clone(),
        branch: a.branch.clone(),
        path: "docs/plans/x.md".into(),
        plan: Some(plan),
        parse_error: None,
        approval_sha: Some(hash.into()),
        head_sha: Some("h1".into()),
    }
}

/// `handoff_overdue` counts an approval typed into its session over 30 minutes ago that is
/// still the newest revision, has no handoff on its branch, and no report that it committed or
/// that the plan changed. Each other approval differs from the first in one of those.
#[test]
fn handoff_overdue_counts_only_approvals_still_current() {
    let now = Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap();
    let overdue = approval(1, "s1", None);
    let recent = approval(2, "s2", None);
    let replaced = approval(3, "s3", None);
    let committed = approval(4, "s4", Some("committed"));
    let changed = approval(5, "s5", Some("plan_changed"));
    let on_branch = approval(6, "s6", None);
    let undelivered = approval(7, "s7", None);
    let pushed_failed = approval(8, "s8", Some("push_failed"));
    let all = [
        overdue.clone(),
        recent.clone(),
        replaced.clone(),
        committed.clone(),
        changed.clone(),
        on_branch.clone(),
        undelivered.clone(),
        pushed_failed.clone(),
    ];
    let mut newest: HashMap<Uuid, String> = all
        .iter()
        .map(|a| (a.draft_id, a.plan_sha.clone()))
        .collect();
    newest.insert(replaced.draft_id, "a newer revision".into());
    let long_ago = now - Duration::minutes(31);
    let mut delivered: HashMap<Uuid, DateTime<Utc>> = all
        .iter()
        .filter(|a| a.id != undelivered.id)
        .map(|a| (a.id, long_ago))
        .collect();
    delivered.insert(recent.id, now - Duration::minutes(10));
    let plans = vec![handed_off(&on_branch, "s6")];
    let links: HashMap<Uuid, LinkRow> = HashMap::from([(
        on_branch.id,
        LinkRow {
            approval_id: on_branch.id,
            commit_sha: "h1".into(),
            state: LinkState::Posted,
            comment_id: Some(1),
            url: Some("u".into()),
            created_at: now,
            posted_at: Some(now),
        },
    )]);
    let got = assess(&LinkFacts {
        approvals: &all,
        newest: &newest,
        plans: &plans,
        links: &links,
        delivered: &delivered,
        can_act: &HashSet::new(),
        now,
    });
    assert_eq!(
        got.overdue, 2,
        "the overdue one and the one whose push failed"
    );
    assert_eq!((got.link_missing, got.mismatch), (0, 0));
    assert!(got.findings.is_empty(), "{:?}", got.findings);
    // Without the push failure, one: the count is not a coincidence of the list.
    let got = assess(&LinkFacts {
        approvals: &all[..7],
        newest: &newest,
        plans: &plans,
        links: &links,
        delivered: &delivered,
        can_act: &HashSet::new(),
        now,
    });
    assert_eq!(got.overdue, 1);
}

/// The record matches at the second, whatever the case of the login; a link is due only with
/// a token, else the approver is waited for; ADRs left are a finding for the newest approval.
#[test]
fn the_record_the_link_and_the_adrs_left() {
    let now = Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap();
    let a = approval(1, "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9", None);
    let keys = |by: &str, at: &str, sha: &str| PlanApprovalRecord {
        approved_by: Some(by.into()),
        approved_at: Some(at.into()),
        approved_sha: Some(sha.into()),
    };
    assert!(keys_match(
        &keys("TeamMate", "2026-11-20T10:00:00Z", &a.plan_sha),
        &a
    ));
    assert!(keys_match(
        &keys(
            "teammate",
            "2026-11-20T11:00:00+01:00",
            &a.plan_sha.to_uppercase()
        ),
        &a
    ));
    assert!(!keys_match(
        &keys("teammate", "2026-11-20T10:00:01Z", &a.plan_sha),
        &a
    ));
    assert!(!keys_match(
        &keys("other", "2026-11-20T10:00:00Z", &a.plan_sha),
        &a
    ));
    assert!(!keys_match(
        &keys("teammate", "2026-11-20T10:00:00Z", "c438e00"),
        &a
    ));
    assert!(!keys_match(&PlanApprovalRecord::default(), &a));

    let plans = vec![handed_off(&a, &a.plan_sha)];
    let newest = HashMap::from([(a.draft_id, a.plan_sha.clone())]);
    let facts = |can: &HashSet<String>| {
        assess(&LinkFacts {
            approvals: std::slice::from_ref(&a),
            newest: &newest,
            plans: &plans,
            links: &HashMap::new(),
            delivered: &HashMap::new(),
            can_act: can,
            now,
        })
    };
    let with = facts(&HashSet::from(["teammate".to_owned()]));
    assert_eq!(
        with.due,
        vec![Due {
            approval: a.clone(),
            commit: "h1".into(),
            path: "docs/plans/x.md".into(),
        }]
    );
    assert_eq!(with.link_missing, 1, "missing until it is posted");
    let without = facts(&HashSet::new());
    assert!(without.due.is_empty());
    assert_eq!(
        without.findings,
        vec![Finding::LinkWaitingForApprover {
            issue: a.issue.clone(),
            approver: "teammate".into(),
        }]
    );
    // Edited after the handoff: stale, not a mismatch, nothing due.
    let edited = vec![handed_off(&a, "0123456789abcdef")];
    let got = assess(&LinkFacts {
        approvals: std::slice::from_ref(&a),
        newest: &newest,
        plans: &edited,
        links: &HashMap::new(),
        delivered: &HashMap::new(),
        can_act: &HashSet::from(["teammate".to_owned()]),
        now,
    });
    assert!(got.due.is_empty() && got.findings.is_empty() && got.mismatch == 0);

    // ADRs left by the newest approval of the branch only.
    let mut older = approval(1, "s0", None);
    older.id = Uuid::from_u128(50);
    older.approved_at -= Duration::hours(1);
    older.handoff = Some(HandoffFacts {
        outcome: "committed".into(),
        at: now,
        commit: None,
        adrs_left: vec!["0049".into()],
    });
    let mut newer = a.clone();
    newer.handoff = Some(HandoffFacts {
        outcome: "committed".into(),
        at: now,
        commit: None,
        adrs_left: vec!["0050".into()],
    });
    let got = assess(&LinkFacts {
        approvals: &[older, newer],
        newest: &newest,
        plans: &[],
        links: &HashMap::new(),
        delivered: &HashMap::new(),
        can_act: &HashSet::new(),
        now,
    });
    assert_eq!(
        got.findings,
        vec![Finding::AdrsNotAccepted {
            issue: a.issue.clone(),
            adrs: vec!["0050".into()],
        }]
    );
}

/// §D7's body: a permalink at the commit, the login without `@`, the summary, and the line the
/// comments pass finds it by.
#[test]
fn the_link_body_names_the_commit_and_the_approval() {
    let mut a = approval(1, "s", None);
    a.summary = "  We kept the table.  ".into();
    let body = link_body("https://github.com/", &a, "abc1234def", "docs/plans/1.md");
    assert_eq!(
        body,
        format!(
            "Plan approved in Rubato: [docs/plans/1.md at abc1234](https://github.com/m0x6A/rubato/blob/abc1234def/docs/plans/1.md)\nApproved by teammate at 2026-11-20 10:00 UTC.\n\nWe kept the table.\n\nRubato-Approval: {}\n",
            a.id
        )
    );
    assert_eq!(github_poll::link_approval(&body), Some(a.id));
    a.summary = String::new();
    assert!(!link_body("https://github.com", &a, "abc", "p.md").contains("\n\n\n"));
}

/// The three gauges come from the reconcile on every tick: an approval typed into its session
/// over 30 minutes ago with no handoff is overdue until a report says the plan changed.
#[tokio::test]
async fn the_reconcile_records_the_handoff_gauges() {
    let h = harness().await;
    let text = plan_text("one");
    mirror(&h, &text).await;
    h.app.set_sessions(h.companion, vec![idle_session(&h)]);
    let mut rx = connect_companion(&h, &[capability::SESSION_INPUT, capability::PLAN_HANDOFF]);
    let a = approve_as(&h, "teammate", &text).await;
    let (id, _) = pushed(&mut rx).remove(0);
    session_messages::on_result(
        &h.app,
        h.companion,
        &SessionMessageResult {
            message_id: id,
            outcome: SessionMessageOutcome::Delivered,
            at: Utc::now(),
        },
    )
    .await;
    let gauge =
        |h: &H, name: &str| -> Vec<u64> { h.reader.points(name).iter().map(|p| p.value).collect() };
    reconcile_links(&h.app).await;
    assert_eq!(
        gauge(&h, metric::PLANS_HANDOFF_OVERDUE),
        [0],
        "not yet 30 minutes"
    );
    h.app.clock.set(Utc::now() + Duration::minutes(31));
    reconcile_links(&h.app).await;
    assert_eq!(gauge(&h, metric::PLANS_HANDOFF_OVERDUE), [1]);
    assert_eq!(gauge(&h, metric::PLANS_LINK_COMMENT_MISSING), [0]);
    assert_eq!(gauge(&h, metric::PLANS_APPROVAL_RECORD_MISMATCH), [0]);
    on_handoff_report(
        &h.app,
        h.companion,
        &report(h.launch, a.approval_id, HandoffOutcome::PlanChanged, &[]),
    )
    .await;
    reconcile_links(&h.app).await;
    assert_eq!(
        gauge(&h, metric::PLANS_HANDOFF_OVERDUE),
        [0],
        "a plan_changed is not current"
    );
}

async fn messages_of(h: &H, launch: Uuid) -> Vec<rubato_protocol::SessionMessageView> {
    session_messages::list(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Path(launch),
    )
    .await
    .unwrap()
    .0
    .messages
}

/// ADR 0053, plan 122 §D5: an approval given while no session of the draft was live waits; the
/// next launch of the draft gets it first when it reports Started, and a later launch gets no
/// second one while that one is open. A handed-off approval waits for nothing.
#[tokio::test]
async fn a_waiting_approval_goes_to_the_next_launch_of_the_draft() {
    let h = harness().await;
    let text = plan_text("one");
    mirror(&h, &text).await;
    let a = approve_as(&h, "teammate", &text).await;
    assert!(
        messages_of(&h, h.launch).await.is_empty(),
        "no live session: it waits"
    );

    let next = Uuid::from_u128(9);
    insert_launch(&h.app, next, h.companion, &issue()).await;
    super::on_launch_started(&h.app, next).await;
    let bound = messages_of(&h, next).await;
    assert_eq!(bound.len(), 1, "{bound:?}");
    assert_eq!(bound[0].kind, "plan_approved");
    assert_eq!(bound[0].author, "teammate");

    let third = Uuid::from_u128(10);
    insert_launch(&h.app, third, h.companion, &issue()).await;
    super::on_launch_started(&h.app, third).await;
    assert!(
        messages_of(&h, third).await.is_empty(),
        "one plan_approved per approval while it is open"
    );

    // Handed off: a later launch has nothing waiting, even with the first message gone.
    sqlx::query("UPDATE session_messages SET state = 'failed'")
        .execute(&h.app.db)
        .await
        .unwrap();
    on_handoff_report(
        &h.app,
        h.companion,
        &report(next, a.approval_id, HandoffOutcome::Committed, &[]),
    )
    .await;
    let fourth = Uuid::from_u128(11);
    insert_launch(&h.app, fourth, h.companion, &issue()).await;
    super::on_launch_started(&h.app, fourth).await;
    assert!(
        messages_of(&h, fourth).await.is_empty(),
        "committed: nothing to hand off"
    );
}

/// A handoff that refused because the bundle or a pinned ADR changed wrote nothing and leaves an
/// approval of an earlier version: not overdue, nothing to page. An approval superseded by a
/// newer one of the same branch is never overdue either; only the newer one can be.
#[test]
fn a_refused_or_superseded_approval_is_not_overdue() {
    let now = Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap();
    let bundle = approval(1, "s1", Some("bundle_changed"));
    let adr = approval(2, "s2", Some("adr_changed"));
    // Two approvals of one branch: the first, then a newer one of the same plan.
    let older = approval(3, "s3", None);
    let mut newer = approval(3, "s3", None);
    newer.id = Uuid::from_u128(99);
    newer.approved_at = older.approved_at + Duration::minutes(5);
    let all = [bundle.clone(), adr.clone(), older.clone(), newer.clone()];
    let newest: HashMap<Uuid, String> = all
        .iter()
        .map(|a| (a.draft_id, a.plan_sha.clone()))
        .collect();
    let delivered: HashMap<Uuid, DateTime<Utc>> = all
        .iter()
        .map(|a| (a.id, now - Duration::minutes(31)))
        .collect();
    let got = assess(&LinkFacts {
        approvals: &all,
        newest: &newest,
        plans: &[],
        links: &HashMap::new(),
        delivered: &delivered,
        can_act: &HashSet::new(),
        now,
    });
    assert_eq!(got.overdue, 1, "only the newer approval of branch 3");
    // Without the newer one, the older one is the newest of its branch, and overdue.
    let got = assess(&LinkFacts {
        approvals: &all[..3],
        newest: &newest,
        plans: &[],
        links: &HashMap::new(),
        delivered: &delivered,
        can_act: &HashSet::new(),
        now,
    });
    assert_eq!(got.overdue, 1);
}
