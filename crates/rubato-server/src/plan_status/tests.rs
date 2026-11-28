//! Plan 226 §4: what a launched session's `plan_status` is told, and when.

use super::*;
use crate::kernel::config::Config;
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{AppState, CompanionConn};
use crate::kernel::telemetry::metric_reader;
use chrono::TimeZone;
use rubato_protocol::otel::metric;
use rubato_protocol::{
    ApprovalRecord, ApprovalSource, ApprovalState, ApprovalVersion, DraftSummary, IssueRef, Plan,
    PlanApprovalRecord, SessionState,
};
use std::sync::Arc;
use tokio::sync::mpsc;

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap()
}

fn issue() -> IssueRef {
    IssueRef::new("m0x6A/rubato", 7)
}

fn summary() -> IssueSummary {
    serde_json::from_value(serde_json::json!({
        "issue": issue(),
        "pins": [],
        "progress": {"issue": issue(), "state": "designing", "last_seen": at()},
    }))
    .unwrap()
}

fn draft(branch: &str, fits: Option<bool>) -> DraftSummary {
    DraftSummary {
        draft_id: Uuid::from_u128(5),
        branch: branch.into(),
        owner: "m0x6A".into(),
        rev: 3,
        at: at(),
        plan_sha: "abc1234".into(),
        bundle_hash: "def5678".into(),
        missing: vec![],
        parse_error: None,
        live: true,
        imported_from: None,
        original_author: None,
        fits,
    }
}

fn approval(source: ApprovalSource, version: ApprovalVersion) -> ApprovalState {
    ApprovalState {
        record: ApprovalRecord {
            issue: issue(),
            source,
            comment_id: source.is_comment().then_some(42),
            approval_id: (!source.is_comment()).then_some(Uuid::from_u128(9)),
            bundle_hash: None,
            author: "m0x6A".into(),
            sha: Some("abc1234".into()),
            branch: None,
            note: None,
            created_at: at(),
            url: String::new(),
        },
        version,
        current_sha: Some("abc1234".into()),
    }
}

fn branch_plan(approved: bool) -> Plan {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/fixtures/plans/data-models.json");
    let mut plan: Plan = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    plan.approval = approved.then(|| PlanApprovalRecord {
        approved_by: Some("m0x6A".into()),
        approved_at: Some("2026-11-16T12:00:00Z".into()),
        approved_sha: Some("abc1234".into()),
    });
    plan
}

const L: Uuid = Uuid::from_u128(10);
const BRANCH: &str = "issue/7-agent";

#[test]
fn a_counted_comment_approval_before_the_cutover_reads_as_a_comment_approval() {
    let mut s = summary();
    s.approval = Some(approval(ApprovalSource::Comment, ApprovalVersion::Current));
    let st = derive(&s, L, Some(BRANCH), at());
    let a = st.approval.expect("approved");
    assert_eq!(a.record.source, ApprovalSource::Comment);
    assert_eq!(a.version, ApprovalVersion::Current);
}

#[test]
fn a_rubato_approval_and_one_of_a_plan_that_does_not_fit_keep_their_source_and_version() {
    let mut s = summary();
    s.approval = Some(approval(ApprovalSource::Rubato, ApprovalVersion::Current));
    assert_eq!(
        derive(&s, L, Some(BRANCH), at())
            .approval
            .unwrap()
            .record
            .source,
        ApprovalSource::Rubato
    );
    s.approval = Some(approval(
        ApprovalSource::Rubato,
        ApprovalVersion::DoesNotFit,
    ));
    assert_eq!(
        derive(&s, L, Some(BRANCH), at()).approval.unwrap().version,
        ApprovalVersion::DoesNotFit
    );
}

#[test]
fn the_draft_is_this_branchs_with_its_own_fit() {
    let mut s = summary();
    s.draft = Some(draft(BRANCH, Some(true)));
    let d = derive(&s, L, Some(BRANCH), at())
        .draft
        .expect("this branch's draft");
    assert_eq!((d.rev, d.plan_sha.as_str(), d.fits), (3, "abc1234", true));
    s.draft = Some(draft(BRANCH, None));
    assert!(
        !derive(&s, L, Some(BRANCH), at()).draft.unwrap().fits,
        "unknown fit is not a fit"
    );
    s.draft = Some(draft("issue/7-other", Some(true)));
    assert_eq!(
        derive(&s, L, Some(BRANCH), at()).draft,
        None,
        "another branch's draft"
    );
}

#[test]
fn the_branch_plan_says_whether_it_carries_the_approval_record() {
    let mut s = summary();
    s.plan = Some(branch_plan(true));
    s.plan_sha = Some("abc1234".into());
    let b = derive(&s, L, Some(BRANCH), at()).branch.unwrap();
    assert_eq!((b.sha.as_str(), b.carries_approval), ("abc1234", true));
    s.plan = Some(branch_plan(false));
    assert!(
        !derive(&s, L, Some(BRANCH), at())
            .branch
            .unwrap()
            .carries_approval
    );
}

// ----- pushes ---------------------------------------------------------------------------------

struct H {
    app: App,
    reader: metric_reader::Reader,
    rx: mpsc::UnboundedReceiver<ServerMessage>,
    _tmp: tempfile::TempDir,
}

const C: Uuid = Uuid::from_u128(1);

async fn h(caps: &[&str]) -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = crate::kernel::db::connect(&url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut app = AppState::new(
        pool,
        Config::for_tests(url, "m0x6A"),
        Arc::new(FakeGitHub::new()),
    );
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let (tx, rx) = mpsc::unbounded_channel();
    app.companions
        .lock()
        .unwrap()
        .insert(C, CompanionConn::new(tx, Utc::now()));
    app.set_capabilities(C, caps.iter().map(|c| (*c).to_owned()).collect());
    sqlx::query("INSERT INTO launches (tenant_id, workspace_id, id, companion_id, user, repo, number, command, created_at) VALUES ('default', 'default', ?, ?, 'm0x6A', 'm0x6A/rubato', 7, '{}', '2026-11-16T12:00:00Z')")
        .bind(L.to_string())
        .bind(C.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let session: SessionState = serde_json::from_value(serde_json::json!({
        "session_id": "s", "companion_id": C, "user": "m0x6A", "cwd": "/w", "branch": BRANCH,
        "status": "working", "source": "hook", "last_seen": Utc::now(), "launch_id": L,
    }))
    .unwrap();
    app.set_sessions(C, vec![session]);
    H {
        app,
        reader,
        rx,
        _tmp: tmp,
    }
}

fn built(s: IssueSummary) -> Vec<(crate::kernel::tenancy::WsKey, Vec<IssueSummary>)> {
    vec![(
        crate::kernel::tenancy::WsKey::of_default("default"),
        vec![s],
    )]
}

impl H {
    fn pushed(&mut self) -> Vec<PlanStatus> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            if let ServerMessage::PlanStatus(s) = m {
                out.push(*s);
            }
        }
        out
    }

    fn sent(&self, reason: &str, outcome: &str) -> u64 {
        self.reader
            .points(metric::PLAN_STATUS_SENT)
            .iter()
            .filter(|p| p.attrs[attr::DROP_REASON] == reason && p.attrs[attr::OUTCOME] == outcome)
            .map(|p| p.value)
            .sum()
    }
}

#[tokio::test]
async fn a_tick_sends_once_per_change_and_only_to_a_companion_that_takes_it() {
    let mut h = h(&[capability::AGENT_MCP]).await;
    let mut s = summary();
    s.draft = Some(draft(BRANCH, Some(true)));
    on_tick(&h.app, &built(s.clone())).await;
    let first = h.pushed();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].launch_id, L);
    assert_eq!(h.sent("welcome", "sent"), 1, "the first push on a socket");
    on_tick(&h.app, &built(s.clone())).await;
    assert!(h.pushed().is_empty(), "nothing changed");
    s.approval = Some(approval(ApprovalSource::Comment, ApprovalVersion::Current));
    on_tick(&h.app, &built(s)).await;
    assert_eq!(h.pushed().len(), 1);
    assert_eq!(h.sent("change", "sent"), 1);

    let mut without = self::h(&[]).await;
    on_tick(&without.app, &built(summary())).await;
    assert!(
        without.pushed().is_empty(),
        "a companion without agent_mcp gets nothing"
    );
}

#[tokio::test]
async fn a_new_socket_gets_a_fresh_push_and_a_welcome_sends_from_the_cache() {
    let mut h = h(&[capability::AGENT_MCP]).await;
    on_tick(&h.app, &built(summary())).await;
    assert_eq!(h.pushed().len(), 1);
    // The companion reconnects: a new socket, so the cached status goes out on it at once.
    let (tx, rx) = mpsc::unbounded_channel();
    h.app
        .companions
        .lock()
        .unwrap()
        .insert(C, CompanionConn::new(tx, Utc::now()));
    h.rx = rx;
    on_welcome(&h.app, C).await;
    assert_eq!(h.pushed().len(), 1, "from the last tick's cache");
    assert_eq!(h.sent("welcome", "sent"), 2);
    on_tick(&h.app, &built(summary())).await;
    assert!(h.pushed().is_empty(), "already sent on this socket");
}

#[tokio::test]
async fn a_launch_without_a_summary_is_counted_and_a_disconnected_companion_skipped() {
    let mut h = h(&[capability::AGENT_MCP]).await;
    on_tick(
        &h.app,
        &[(crate::kernel::tenancy::WsKey::of_default("default"), vec![])],
    )
    .await;
    assert!(h.pushed().is_empty());
    assert_eq!(h.sent("change", "no_summary"), 1);
    h.app.companions.lock().unwrap().remove(&C);
    on_tick(&h.app, &built(summary())).await;
    assert_eq!(
        h.sent("welcome", "offline") + h.sent("change", "offline"),
        0,
        "not asked at all"
    );
}

#[tokio::test]
async fn an_issue_in_two_workspaces_is_read_from_the_launchs_own() {
    let mut h = h(&[capability::AGENT_MCP]).await;
    let mut here = summary();
    here.draft = Some(draft(BRANCH, Some(true)));
    let mut elsewhere = summary();
    elsewhere.draft = Some(draft(BRANCH, Some(false)));
    on_tick(
        &h.app,
        &[
            (
                crate::kernel::tenancy::WsKey::of_default("other"),
                vec![elsewhere],
            ),
            (
                crate::kernel::tenancy::WsKey::of_default("default"),
                vec![here],
            ),
        ],
    )
    .await;
    let got = h.pushed();
    assert!(
        got[0].draft.as_ref().unwrap().fits,
        "the launch's workspace is default"
    );
}
