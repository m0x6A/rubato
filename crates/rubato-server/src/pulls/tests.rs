//! A pull request's view (design 82 §2): the shape, team reviews only, the 30 s cache, one fetch
//! in flight, no token, a merged pull request, and 404 outside the map or the installation. The
//! gate through the overview is in `tests/integration.rs` (it needs the poll).

use super::view::{PullPath, pull_view};
use crate::auth::{CurrentUser, Granted, keep_token};
use crate::kernel::config::{Config, GitHubApp, TokenKey};
use crate::kernel::db::connect;
use crate::kernel::github::{FakeGitHub, PullFull, PullInfo, ReviewInfo};
use crate::kernel::http::Ws;
use crate::kernel::state::{App, AppState, LoadedMap};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{TimeZone, Utc};
use rubato_protocol::pulls::{
    ChangedFile, CheckRun, MergeMethod, PlanGate, PullDetail, PullState, reason,
};
use rubato_protocol::{ChecksStatus, SystemMap};
use serde_json::Value;
use std::sync::Arc;

const REPO: &str = "m0x6A/rubato";
const BRANCH: &str = "issue/82-x";

struct H {
    app: App,
    github: Arc<FakeGitHub>,
    _tmp: tempfile::TempDir,
}

async fn harness() -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = connect(&url).await.unwrap();
    let mut cfg = Config::for_tests(url, "m0x6A");
    cfg.github_app = Some(GitHubApp {
        app_id: 1,
        private_key: "unused".into(),
        installation_id: 34,
    });
    cfg.token_key = Some(TokenKey::from_hex(&"07".repeat(32)).unwrap());
    let github = Arc::new(FakeGitHub::new());
    let app = AppState::new(pool, cfg, github.clone());
    let text = include_str!("../../../../docs/design/rubato-map.yml");
    let map: SystemMap = serde_yaml_ng::from_str(text).unwrap();
    app.set_map(
        &crate::kernel::tenancy::WsKey::of_default(rubato_protocol::DEFAULT_WORKSPACE),
        LoadedMap {
            map,
            version: "v".into(),
            loaded_at: Utc::now(),
            yaml: text.into(),
        },
    )
    .await;
    github.with(|g| {
        g.pulls.insert(
            (REPO.into(), BRANCH.into()),
            PullInfo {
                number: 5,
                open: true,
                draft: false,
                head_sha: "h5".into(),
                merged_at: None,
                updated_at: Utc::now(),
                url: "https://github.com/m0x6A/rubato/pull/5".into(),
                base: Some("main".into()),
                title: "Pulls".into(),
                author: "ann".into(),
            },
        );
        g.check_runs.insert(
            "h5".into(),
            vec![CheckRun {
                name: "ci".into(),
                status: "completed".into(),
                conclusion: Some("success".into()),
                url: "https://github.com/run/1".into(),
            }],
        );
    });
    H {
        app,
        github,
        _tmp: tmp,
    }
}

async fn token(h: &H, login: &str) {
    let kept = keep_token(
        &h.app,
        login,
        &Granted {
            access: format!("tok-{login}"),
            expires_in: Some(chrono::Duration::hours(8)),
            refresh: Some("r".into()),
            refresh_expires_in: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(kept, None);
}

async fn view(
    h: &H,
    login: &str,
    repo: &str,
    number: u64,
) -> Result<PullDetail, (StatusCode, Value)> {
    let (owner, name) = repo.split_once('/').unwrap();
    pull_view(
        State(h.app.clone()),
        Ws(crate::kernel::tenancy::WsKey::of_default(
            rubato_protocol::DEFAULT_WORKSPACE,
        )),
        CurrentUser(login.into()),
        Path(PullPath {
            owner: owner.into(),
            repo: name.into(),
            number,
        }),
    )
    .await
    .map(|Json(d)| d)
    .map_err(|(s, Json(v))| (s, v))
}

fn calls(h: &H, kind: &str) -> usize {
    h.github.calls().iter().filter(|(k, _)| *k == kind).count()
}

fn review(author: &str, assoc: &str, bot: bool, state: &str, h: u32) -> ReviewInfo {
    ReviewInfo {
        author: author.into(),
        author_association: assoc.into(),
        author_is_bot: bot,
        state: state.into(),
        submitted_at: Utc.with_ymd_and_hms(2026, 11, 15, h, 0, 0).unwrap(),
    }
}

#[tokio::test]
async fn the_view_reads_once_and_shows_team_reviews_only() {
    let h = harness().await;
    token(&h, "m0x6A").await;
    h.github.with(|g| {
        g.reviews.insert(
            (REPO.into(), 5),
            vec![
                review("m0x6A", "OWNER", false, "CHANGES_REQUESTED", 9),
                review("M0X6A", "OWNER", false, "APPROVED", 10),
                review("stranger", "NONE", false, "APPROVED", 11),
                review("bot", "NONE", true, "COMMENTED", 12),
            ],
        );
        g.files_changed.insert(
            (REPO.into(), 5),
            vec![ChangedFile {
                path: "a.rs".into(),
                additions: 4,
                deletions: 2,
            }],
        );
    });
    let d = view(&h, "m0x6A", REPO, 5).await.unwrap();
    assert_eq!(
        (d.number, d.state, d.branch.as_str()),
        (5, PullState::Open, BRANCH)
    );
    assert_eq!(d.issue.as_ref().map(|i| i.number), Some(82));
    assert_eq!(d.checks, ChecksStatus::Green);
    assert_eq!(d.check_runs.len(), 1);
    assert_eq!(
        d.reviews.len(),
        1,
        "latest per team reviewer: {:?}",
        d.reviews
    );
    assert_eq!(d.reviews[0].state, "APPROVED");
    assert_eq!(d.outside_reviews, 2);
    assert_eq!(
        (d.files.count, d.files.additions, d.files.deletions),
        (1, 4, 2)
    );
    assert_eq!(
        d.merge.plan_gate,
        PlanGate::NoPlan,
        "no plan anywhere for #82 here"
    );
    assert!(d.merge.can_merge, "{:?}", d.merge);
    assert_eq!(d.merge.default_method, Some(MergeMethod::Squash));
    // ADR 0034's per-view budget: pull, reviews, files, runs at the head, the plan file, the
    // repo as the person. No runs at the base: the head has some.
    for (kind, n) in [
        ("pull", 1),
        ("pull_reviews", 1),
        ("pull_files", 1),
        ("check_runs", 1),
        ("file", 1),
        ("repo_as", 1),
    ] {
        assert_eq!(calls(&h, kind), n, "{kind}: {:?}", h.github.calls());
    }
    // Within 30 s: nothing read again.
    let again = view(&h, "m0x6A", REPO, 5).await.unwrap();
    assert_eq!(again.read_at, d.read_at);
    assert_eq!((calls(&h, "pull"), calls(&h, "repo_as")), (1, 1));
}

#[tokio::test]
async fn two_views_at_once_make_one_fetch() {
    let h = harness().await;
    h.github
        .with(|g| g.pull_delay = Some(std::time::Duration::from_millis(200)));
    let (a, b) = tokio::join!(view(&h, "m0x6A", REPO, 5), view(&h, "ann", REPO, 5));
    assert_eq!(a.unwrap().read_at, b.unwrap().read_at);
    assert_eq!(calls(&h, "pull"), 1, "{:?}", h.github.calls());
}

#[tokio::test]
async fn without_a_token_the_view_renders_and_says_why_not() {
    let h = harness().await;
    let d = view(&h, "m0x6A", REPO, 5).await.unwrap();
    assert!(!d.merge.can_merge && d.merge.methods.is_empty());
    assert_eq!(
        d.merge.why_not.as_deref(),
        Some(reason::NEEDS_GITHUB_AUTHORIZATION)
    );
    assert_eq!(calls(&h, "repo_as"), 0);
}

#[tokio::test]
async fn a_merged_pull_request_renders_and_runs_at_the_base_are_read_only_without_runs_at_the_head()
{
    let h = harness().await;
    token(&h, "m0x6A").await;
    h.github.with(|g| {
        let mut info = g.pulls[&(REPO.to_owned(), BRANCH.to_owned())].clone();
        info.open = false;
        info.merged_at = Some(Utc::now());
        info.head_sha = "h6".into();
        info.number = 6;
        g.pull_details.insert(
            (REPO.into(), 6),
            PullFull {
                info,
                branch: "trivial/x".into(),
                body: "done".into(),
                fork: false,
                mergeable: None,
                base_sha: "b6".into(),
            },
        );
    });
    let d = view(&h, "m0x6A", REPO, 6).await.unwrap();
    assert_eq!(d.state, PullState::Merged);
    assert_eq!(d.body, "done");
    assert_eq!(d.merge.why_not.as_deref(), Some(reason::NOT_OPEN));
    assert!(d.issue.is_none());
    assert_eq!(
        calls(&h, "check_runs"),
        2,
        "head h6 has none, so the base b6 is read"
    );
    assert_eq!(calls(&h, "file"), 0, "the branch names no issue");
}

#[tokio::test]
async fn a_repo_outside_the_map_or_the_installation_is_not_found() {
    let h = harness().await;
    let (s, v) = view(&h, "m0x6A", "m0x6A/elsewhere", 5).await.unwrap_err();
    assert_eq!(
        (s, v["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some(reason::UNKNOWN_REPO))
    );
    assert_eq!(
        calls(&h, "pull"),
        0,
        "nothing read for a repo outside the map"
    );
    // Spelled another way, the map's repo is found.
    assert!(view(&h, "m0x6A", "M0X6A/Rubato", 5).await.is_ok());
    let (s, _) = view(&h, "m0x6A", REPO, 99).await.unwrap_err();
    assert_eq!(s, StatusCode::NOT_FOUND);
    // An installation that does not cover the repo.
    h.github.with(|g| g.installation = Some(vec![]));
    crate::github_poll::check_coverage(
        &h.app,
        &crate::kernel::tenancy::TenantId::default_tenant(),
        &[REPO.to_owned()].into(),
    )
    .await;
    let (s, v) = view(&h, "m0x6A", REPO, 5).await.unwrap_err();
    assert_eq!(
        (s, v["reason"].as_str()),
        (StatusCode::NOT_FOUND, Some(reason::UNKNOWN_REPO))
    );
}

/// Plan 82 Decision 9: a contents read that fails closes the gate; it is never "no file". Without
/// the `Unreadable` arm this view would offer Merge (no other plan signal exists here).
#[tokio::test]
async fn an_unreadable_plan_closes_the_gate() {
    let h = harness().await;
    token(&h, "m0x6A").await;
    h.github
        .with(|g| g.file_error = Some("502 Bad Gateway".into()));
    let d = view(&h, "m0x6A", REPO, 5).await.unwrap();
    assert_eq!(
        d.merge.plan_gate,
        PlanGate::NotApproved {
            why: rubato_protocol::pulls::plan_why::UNREADABLE.into()
        }
    );
    assert_eq!(d.merge.why_not.as_deref(), Some(reason::PLAN_NOT_APPROVED));
}

/// Plan 82 Decision 10: with a plan row, the head is read at the row's path, not at the path the
/// head branch's slug would give (a stacked head with another slug).
#[tokio::test]
async fn the_plan_is_read_at_the_plan_rows_path() {
    let h = harness().await;
    sqlx::query("INSERT INTO plans (tenant_id, repo, number, branch, path, plan, parse_error, approval_sha, head_sha, fetched_at) VALUES ('default', ?, 82, 'issue/82-a', 'docs/plans/82-a.md', NULL, 'x', 'abc1234', 's', '2026-11-15T00:00:00Z')")
        .bind(REPO)
        .execute(&h.app.db)
        .await
        .unwrap();
    view(&h, "m0x6A", REPO, 5).await.unwrap();
    let read: Vec<String> = h
        .github
        .calls()
        .into_iter()
        .filter(|(k, _)| *k == "file")
        .map(|(_, a)| a)
        .collect();
    assert_eq!(read, [format!("{REPO} docs/plans/82-a.md h5")]);
}
