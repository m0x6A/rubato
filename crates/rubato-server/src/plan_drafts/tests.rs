//! Drafts from bundles (ADR 0053): one revision per changed bundle, owned by the first launch's
//! user, another person's bundle refused, a plan that does not parse kept with its error.

use super::{handlers, intake, latest_drafts};
use crate::auth::CurrentUser;
use crate::kernel::config::Config;
use crate::kernel::db::connect;
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState};
use crate::kernel::telemetry::metric_reader;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{TimeZone, Utc};
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::{DraftBundle, DraftFile, DraftRefusal, IssueRef, approval_hash, bundle_hash};
use std::sync::Arc;
use uuid::Uuid;

const PLAN: &str = "docs/plans/122.md";

fn plan_text(change: &str) -> String {
    format!(
        "---\nissue: m0x6A/rubato#122\n---\n# Plan\n\n## Affected parts\n\n| Node | Status | What changes | Seams |\n| --- | --- | --- | --- |\n| rubato-server | affected | {change} | web-server |\n\n## O-rows\n\n| Seam | Breaks like | Signal | Panel | Owner |\n| --- | --- | --- | --- | --- |\n| web-server | x | y | z | rubato-server |\n\n- [ ] a task\n"
    )
}

fn bundle(text: &str, design: &str) -> DraftBundle {
    let files = vec![
        DraftFile {
            path: PLAN.into(),
            text: text.into(),
        },
        DraftFile {
            path: "docs/plans/122-x/design.md".into(),
            text: design.into(),
        },
    ];
    let pairs: Vec<(String, String)> = files
        .iter()
        .map(|f| (f.path.clone(), f.text.clone()))
        .collect();
    DraftBundle {
        launch_id: Uuid::from_u128(1),
        session_id: Some("s1".into()),
        issue: IssueRef::new("m0x6A/rubato", 122),
        branch: "issue/122-x".into(),
        bundle_hash: bundle_hash(PLAN, &pairs),
        files,
        missing: vec![],
        refused: None,
        at: Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap(),
    }
}

async fn harness() -> (App, metric_reader::Reader, tempfile::TempDir) {
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
    (app, reader, tmp)
}

fn refused(reader: &metric_reader::Reader, reason: &str) -> u64 {
    reader
        .points(metric::DRAFTS_REFUSED)
        .iter()
        .filter(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(reason))
        .map(|p| p.value)
        .sum()
}

#[tokio::test]
async fn a_changed_bundle_is_the_next_revision_and_an_unchanged_one_is_not_stored() {
    let (app, _reader, _tmp) = harness().await;
    intake::on_draft(&app, &bundle(&plan_text("one"), "design\n"), "m0x6A").await;
    intake::on_draft(&app, &bundle(&plan_text("one"), "design\n"), "m0x6A").await;
    // Ticking a task is not a new version of the plan, but it is a new bundle file text: the
    // bundle hash leaves ticks out too (the plan's blob is its approval hash).
    intake::on_draft(
        &app,
        &bundle(&plan_text("one").replace("- [ ]", "- [x]"), "design\n"),
        "m0x6A",
    )
    .await;
    let latest = latest_drafts(&app, &crate::kernel::tenancy::TenantId::default_tenant())
        .await
        .unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].summary.rev, 1, "unchanged bundles store nothing");
    intake::on_draft(
        &app,
        &bundle(&plan_text("one"), "design, revised\n"),
        "m0x6A",
    )
    .await;
    let latest = latest_drafts(&app, &crate::kernel::tenancy::TenantId::default_tenant())
        .await
        .unwrap();
    assert_eq!(
        latest[0].summary.rev, 2,
        "an edited design note is a new revision"
    );
    assert_eq!(latest[0].summary.plan_sha, approval_hash(&plan_text("one")));
    assert!(latest[0].plan.is_some(), "the plan parses");
    assert_eq!(latest[0].summary.owner, "m0x6A");
}

#[tokio::test]
async fn another_persons_bundle_is_not_stored_and_is_a_finding_input() {
    let (app, reader, _tmp) = harness().await;
    intake::on_draft(&app, &bundle(&plan_text("one"), "d\n"), "m0x6A").await;
    intake::on_draft(&app, &bundle(&plan_text("theirs"), "d\n"), "alice").await;
    let latest = latest_drafts(&app, &crate::kernel::tenancy::TenantId::default_tenant())
        .await
        .unwrap();
    assert_eq!(latest[0].summary.rev, 1);
    assert_eq!(
        latest[0].conflict.as_ref().map(|c| c.0.as_str()),
        Some("alice")
    );
    assert_eq!(refused(&reader, "owner_conflict"), 1);
}

#[tokio::test]
async fn a_bundle_refused_on_the_machine_and_a_plan_that_does_not_parse() {
    let (app, reader, _tmp) = harness().await;
    let mut big = bundle(&plan_text("one"), "d\n");
    big.files.clear();
    big.refused = Some(DraftRefusal::TooLarge);
    intake::on_draft(&app, &big, "m0x6A").await;
    assert!(
        latest_drafts(&app, &crate::kernel::tenancy::TenantId::default_tenant())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(refused(&reader, "too_large"), 1);
    intake::on_draft(&app, &bundle("---\ntitle: no issue\n---\n", "d\n"), "m0x6A").await;
    let latest = latest_drafts(&app, &crate::kernel::tenancy::TenantId::default_tenant())
        .await
        .unwrap();
    assert!(latest[0].plan.is_none());
    assert!(
        latest[0].summary.parse_error.is_some(),
        "kept with its error, shown on the draft"
    );
}

#[tokio::test]
async fn everyone_reads_a_draft_and_its_revisions() {
    let (app, reader, _tmp) = harness().await;
    intake::on_draft(&app, &bundle(&plan_text("one"), "d\n"), "m0x6A").await;
    intake::on_draft(&app, &bundle(&plan_text("two"), "d\n"), "m0x6A").await;
    let id = latest_drafts(&app, &crate::kernel::tenancy::TenantId::default_tenant())
        .await
        .unwrap()[0]
        .summary
        .draft_id;
    let view = handlers::draft(
        State(app.clone()),
        crate::kernel::tenancy::default_scope(),
        CurrentUser("alice".into()),
        Path(id.into()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(
        view.revisions.iter().map(|r| r.rev).collect::<Vec<_>>(),
        [1, 2]
    );
    let r1 = handlers::revision(
        State(app.clone()),
        crate::kernel::tenancy::default_scope(),
        CurrentUser("alice".into()),
        Path((id, 1).into()),
    )
    .await
    .unwrap()
    .0;
    assert!(r1.files[0].text.contains("| one |"));
    let missing = handlers::revision(
        State(app.clone()),
        crate::kernel::tenancy::default_scope(),
        CurrentUser("alice".into()),
        Path((id, 9).into()),
    )
    .await;
    assert_eq!(missing.err().map(|e| e.0), Some(StatusCode::NOT_FOUND));
    let unknown = handlers::draft(
        State(app.clone()),
        crate::kernel::tenancy::default_scope(),
        CurrentUser("alice".into()),
        Path(Uuid::new_v4().into()),
    )
    .await;
    assert_eq!(unknown.err().map(|e| e.0), Some(StatusCode::NOT_FOUND));
    let rejected: u64 = reader
        .points(metric::WEB_REJECTED)
        .iter()
        .map(|p| p.value)
        .sum();
    assert_eq!(rejected, 2);
}
