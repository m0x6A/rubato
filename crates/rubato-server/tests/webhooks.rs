//! Issue #208 (ADR 0074): what a webhook delivery makes the poll loop read, the catch-up, the
//! loop's cadence with webhooks on, and the rate-limit read on every path. The handler's own
//! checks are in `src/webhooks/tests.rs`; the route's place in the router, in `src/app.rs`.

use rubato_protocol::pins::routes as pin_routes;
use rubato_protocol::progress::routes as overview_routes;
use rubato_protocol::{ChecksStatus, IssueRef, MapRef, Overview, PinRequest, ProgressState};
use rubato_server::kernel::config::{Config, WebhookSecret};
use rubato_server::kernel::db;
use rubato_server::kernel::github::{Comment, FakeGitHub, OpenIssueInfo, PullInfo};
use rubato_server::kernel::state::{App, AppState};
use rubato_server::{app, map, tasks, webhooks};
use std::path::PathBuf;
use std::sync::Arc;

const SECRET: &str = "test-webhook-secret";
const REPO: &str = "m0x6A/rubato";
const OTHER: &str = "m0x6A/other";
const PLAN: &str = "---\nissue: m0x6A/rubato#1\n---\n## Affected parts\n| Node | Status | What changes | Seams |\n| --- | --- | --- | --- |\n| rubato-server | affected | x | server-github |\n## O-rows\n| Seam | Breaks like | Signal | Panel | Owner |\n| --- | --- | --- | --- | --- |\n| server-github | y | z | p | rubato-server |\n";

struct Harness {
    url: String,
    app: App,
    github: Arc<FakeGitHub>,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Rubato's map with one more part in a second repo, so a refresh can be shown to leave it alone.
fn map_yaml() -> String {
    let yaml = std::fs::read_to_string(repo_root().join("docs/design/rubato-map.yml")).unwrap();
    yaml.replacen(
        "\nnodes:\n",
        "\nnodes:\n  - id: other-app\n    kind: context\n    repo: m0x6A/other\n    description: a part in another repo, for issue #208's tests\n\n",
        1,
    )
}

fn pull(number: u64, sha: &str) -> PullInfo {
    PullInfo {
        number,
        open: true,
        draft: false,
        head_sha: sha.into(),
        merged_at: None,
        updated_at: chrono::Utc::now(),
        url: format!("pr{number}"),
        base: None,
        title: String::new(),
        author: String::new(),
    }
}

fn open_issue(n: u64, body: &str) -> OpenIssueInfo {
    OpenIssueInfo {
        number: n,
        title: format!("issue {n}"),
        url: format!("https://github.com/m0x6A/rubato/issues/{n}"),
        body: body.into(),
        author: "m0x6A".into(),
        author_association: "OWNER".into(),
        updated_at: chrono::Utc::now(),
        ..Default::default()
    }
}

async fn harness(secret: bool, load_map: bool) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}/test.db?mode=rwc", tmp.path().display());
    let pool = db::connect(&db_url).await.unwrap();
    let github = Arc::new(FakeGitHub::new());
    github.with(|g| {
        g.files.insert(
            (
                REPO.into(),
                "docs/design/rubato-map.yml".into(),
                "main".into(),
            ),
            map_yaml(),
        );
        g.branches
            .insert(REPO.into(), vec![("main".into(), "mapsha1".into())]);
        g.branches.insert(OTHER.into(), vec![]);
        g.open.insert(OTHER.into(), vec![open_issue(50, "")]);
        g.pulls
            .insert((OTHER.into(), "feature/z".into()), pull(77, "z1"));
    });
    let mut cfg = Config::for_tests(db_url, "m0x6A");
    if secret {
        cfg.github_webhook_secret = WebhookSecret::from_env_value(Some(SECRET.into()));
    }
    let app = AppState::new(pool, cfg, github.clone());
    if load_map {
        map::reload(&app).await.unwrap();
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app::router(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Harness {
        url: format!("http://{addr}"),
        app,
        github,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

impl Harness {
    /// Issue `n` open, with branch `issue/<n>-x` at `s<n>`, a plan on it, and optionally an open
    /// green pull request.
    fn working_issue(&self, n: u64, with_pr: bool) {
        self.github.with(|g| {
            let branch = format!("issue/{n}-x");
            let sha = format!("s{n}");
            g.branches
                .get_mut(REPO)
                .unwrap()
                .push((branch.clone(), sha.clone()));
            g.files.insert(
                (REPO.into(), format!("docs/plans/{n}.md"), branch.clone()),
                PLAN.replace("#1", &format!("#{n}")),
            );
            if with_pr {
                g.pulls.insert((REPO.into(), branch), pull(1000 + n, &sha));
                g.checks.insert(sha, ChecksStatus::Green);
            }
            g.open
                .entry(REPO.into())
                .or_default()
                .push(open_issue(n, ""));
        });
    }

    async fn pin(&self, n: u64) {
        let r = self
            .http
            .post(format!("{}{}", self.url, pin_routes::PINS))
            .json(&PinRequest {
                issue: IssueRef::new(REPO, n),
                target: MapRef::node("rubato-server"),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
    }

    async fn overview(&self) -> Overview {
        self.http
            .get(format!("{}{}", self.url, overview_routes::OVERVIEW))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn state_of(&self, n: u64) -> Option<ProgressState> {
        self.overview()
            .await
            .issues
            .iter()
            .find(|i| i.issue.number == n && i.issue.repo == REPO)
            .map(|i| i.progress.state)
    }

    fn merge(&self, n: u64) {
        self.github.with(|g| {
            let pr = g
                .pulls
                .get_mut(&(REPO.to_string(), format!("issue/{n}-x")))
                .unwrap();
            pr.open = false;
            pr.merged_at = Some(chrono::Utc::now());
        });
    }

    async fn deliver(&self, id: &str, event: &str, body: serde_json::Value) -> u16 {
        let body = body.to_string();
        self.http
            .post(format!("{}{}", self.url, webhooks::PATH))
            .header("x-github-delivery", id)
            .header("x-github-event", event)
            .header(
                "x-hub-signature-256",
                webhooks::sign(SECRET.as_bytes(), body.as_bytes()),
            )
            .body(body)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    fn calls(&self) -> Vec<String> {
        self.github
            .calls()
            .into_iter()
            .map(|(n, a)| format!("{n} {a}"))
            .collect()
    }

    async fn other_repo_rows(&self) -> Vec<String> {
        let mut out = Vec::new();
        for q in [
            "SELECT repo || '|' || COALESCE(listed_at, '') FROM backlog_repos WHERE repo = 'm0x6A/other'",
            "SELECT repo || '|' || number FROM open_issues WHERE repo = 'm0x6A/other'",
            "SELECT repo || '|' || COALESCE(listed_at, '') FROM repo_pulls_status WHERE repo = 'm0x6A/other'",
            "SELECT repo || '|' || number FROM repo_pulls WHERE repo = 'm0x6A/other'",
        ] {
            let rows: Vec<String> = sqlx::query_scalar(q).fetch_all(&self.app.db).await.unwrap();
            out.extend(rows);
        }
        out
    }
}

fn pr_event(action: &str, branch: &str) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "pull_request": {"number": 1, "head": {"ref": branch}},
        "repository": {"full_name": REPO},
    })
}

fn push_event(repo: &str, branch: &str) -> serde_json::Value {
    serde_json::json!({"ref": format!("refs/heads/{branch}"), "repository": {"full_name": repo}})
}

fn poller() -> tasks::Poller {
    tasks::Poller::counting_changes()
}

/// S5: a merge delivery for issue 5 makes it Done with no read of issue 6, and leaves the other
/// repo's backlog and pull rows as they were.
#[tokio::test]
async fn a_delivery_refreshes_only_what_it_names() {
    let h = harness(true, true).await;
    for n in [5, 6] {
        h.working_issue(n, true);
        h.pin(n).await;
    }
    let mut p = poller();
    p.webhook_tick(&h.app, tasks::Lane::Full, true)
        .await
        .unwrap();
    assert_eq!(h.state_of(5).await, Some(ProgressState::InReview));
    let before = h.other_repo_rows().await;
    assert_eq!(before.len(), 4, "the other repo was listed: {before:?}");
    h.merge(5);
    h.github.with(|g| g.calls.clear());
    assert_eq!(
        h.deliver("d-merge", "pull_request", pr_event("closed", "issue/5-x"))
            .await,
        202
    );
    p.refresh(&h.app).await.unwrap();
    assert_eq!(h.state_of(5).await, Some(ProgressState::Done));
    assert_eq!(h.state_of(6).await, Some(ProgressState::InReview));
    let calls = h.calls();
    assert!(
        calls
            .iter()
            .all(|c| !c.contains("#6") && !c.contains("s6") && !c.contains("issue/6")),
        "nothing read for issue 6: {calls:?}"
    );
    assert!(
        calls.iter().all(|c| !c.contains(OTHER)),
        "nothing read in the other repo: {calls:?}"
    );
    assert!(calls.contains(&format!("open_pulls {REPO}")), "{calls:?}");
    assert_eq!(
        h.other_repo_rows().await,
        before,
        "the other repo's rows are unchanged"
    );
    assert!(
        webhooks::pending(&h.app).await.is_empty(),
        "the row is marked"
    );
    println!("checked {} calls of one refresh", calls.len());
}

/// S5: `issues` opened with a `Map:` line pins the issue; `push` to an issue branch reads the new
/// plan; `issue_comment` reads that issue's comments; a push for an untracked issue's branch
/// discovers its plan; a push to the map branch reloads the map.
#[tokio::test]
async fn each_event_reads_what_it_names() {
    let h = harness(true, true).await;
    for n in [7, 8] {
        h.working_issue(n, false);
        h.pin(n).await;
    }
    let mut p = poller();
    p.webhook_tick(&h.app, tasks::Lane::Full, true)
        .await
        .unwrap();

    // issues opened, with a member's Map: line.
    h.github.with(|g| {
        g.open
            .get_mut(REPO)
            .unwrap()
            .push(open_issue(60, "## Why\nx\n\nMap: node web\n"));
        g.calls.clear();
    });
    let opened = serde_json::json!({"action":"opened","issue":{"number":60},"repository":{"full_name":REPO}});
    assert_eq!(h.deliver("d-60", "issues", opened).await, 202);
    p.refresh(&h.app).await.unwrap();
    assert_eq!(
        h.state_of(60).await,
        Some(ProgressState::Planned),
        "pinned by its Map: line"
    );
    assert!(
        h.calls().iter().all(|c| !c.contains(OTHER)),
        "only this repo listed"
    );

    // push to issue/7-x: the new head's plan is read.
    h.github.with(|g| {
        let b = g.branches.get_mut(REPO).unwrap();
        b.iter_mut().find(|(n, _)| n == "issue/7-x").unwrap().1 = "s7b".into();
        g.calls.clear();
    });
    assert_eq!(
        h.deliver("d-7", "push", push_event(REPO, "issue/7-x"))
            .await,
        202
    );
    p.refresh(&h.app).await.unwrap();
    assert!(
        h.calls()
            .iter()
            .any(|c| c.starts_with("file m0x6A/rubato docs/plans/7.md")),
        "{:?}",
        h.calls()
    );

    // issue_comment on 8: its comments are read and the decision stored.
    h.github.with(|g| {
        g.comments.insert(
            (REPO.into(), 8),
            vec![Comment {
                id: 801,
                author: "m0x6A".into(),
                author_association: "OWNER".into(),
                author_is_bot: false,
                body: "Decision: Keep it\n\n## Chosen\nyes".into(),
                created_at: chrono::Utc::now(),
                url: "c801".into(),
            }],
        );
        g.calls.clear();
    });
    let comment = serde_json::json!({"action":"created","issue":{"number":8},"repository":{"full_name":REPO}});
    assert_eq!(h.deliver("d-8", "issue_comment", comment).await, 202);
    p.refresh(&h.app).await.unwrap();
    assert_eq!(
        h.calls()
            .iter()
            .filter(|c| c.starts_with("issue_comments"))
            .count(),
        1
    );
    let ov = h.overview().await;
    let eight = ov.issues.iter().find(|i| i.issue.number == 8).unwrap();
    assert_eq!(eight.decisions.len(), 1, "the Decision: comment is read");

    // push for untracked issue 9, open with a plan on its branch: discovered.
    h.working_issue(9, false);
    h.github.with(|g| g.calls.clear());
    assert_eq!(
        h.deliver("d-9", "push", push_event(REPO, "issue/9-x"))
            .await,
        202
    );
    p.refresh(&h.app).await.unwrap();
    assert!(
        h.calls()
            .iter()
            .any(|c| c.starts_with("file m0x6A/rubato docs/plans/9.md")),
        "{:?}",
        h.calls()
    );

    // push to the map branch: the map is reloaded.
    h.github.with(|g| g.calls.clear());
    assert_eq!(
        h.deliver("d-map", "push", push_event(REPO, "main")).await,
        202
    );
    p.refresh(&h.app).await.unwrap();
    assert!(
        h.calls()
            .contains(&"branch_sha m0x6A/rubato main".to_owned()),
        "{:?}",
        h.calls()
    );
    println!("checked 5 events");
}

/// S6: with webhooks on, an issue pinned between catch-ups gets its first read on the next tick.
#[tokio::test]
async fn a_newly_pinned_issue_is_read_on_the_next_tick() {
    let h = harness(true, true).await;
    let mut p = poller();
    p.webhook_tick(&h.app, tasks::Lane::Full, true)
        .await
        .unwrap();
    h.working_issue(11, true);
    h.pin(11).await;
    p.webhook_tick(&h.app, tasks::Lane::Fast, false)
        .await
        .unwrap();
    assert_eq!(h.state_of(11).await, Some(ProgressState::InReview));
    // Read once: the next tick reads nothing of it.
    h.github.with(|g| g.calls.clear());
    p.webhook_tick(&h.app, tasks::Lane::Fast, false)
        .await
        .unwrap();
    assert_eq!(h.calls(), ["rate_limit "], "only the rate limit");
}

/// S6: a merge with no delivery stays In review until the catch-up, which counts the change; a
/// change in a repo the installation does not cover is not counted.
#[tokio::test]
async fn a_dropped_delivery_is_fixed_by_the_catch_up() {
    let h = harness(true, true).await;
    h.working_issue(5, true);
    h.pin(5).await;
    let mut p = poller();
    p.webhook_tick(&h.app, tasks::Lane::Full, true)
        .await
        .unwrap();
    h.merge(5);
    p.webhook_tick(&h.app, tasks::Lane::Fast, false)
        .await
        .unwrap();
    p.webhook_tick(&h.app, tasks::Lane::Full, false)
        .await
        .unwrap();
    p.refresh(&h.app).await.unwrap();
    assert_eq!(
        h.state_of(5).await,
        Some(ProgressState::InReview),
        "no delivery, no read"
    );
    assert_eq!(p.last_change_at(), None);
    p.webhook_tick(&h.app, tasks::Lane::Full, true)
        .await
        .unwrap();
    assert_eq!(h.state_of(5).await, Some(ProgressState::Done));
    assert!(p.last_change_at().is_some(), "the catch-up counted it");

    // An installation that covers no repo: GitHub sends it no webhooks, so nothing is counted.
    let h = harness(true, true).await;
    h.github.with(|g| g.installation = Some(vec![]));
    h.working_issue(5, true);
    h.pin(5).await;
    let mut p = poller();
    p.webhook_tick(&h.app, tasks::Lane::Full, true)
        .await
        .unwrap();
    h.merge(5);
    p.webhook_tick(&h.app, tasks::Lane::Full, true)
        .await
        .unwrap();
    assert_eq!(h.state_of(5).await, Some(ProgressState::Done));
    assert_eq!(p.last_change_at(), None, "uncovered repos are not counted");
}

/// The issue's acceptance: at 30 tracked issues, an hour of the production cadence with
/// webhooks on uses well under 10% of the 5,000-call budget.
#[tokio::test]
async fn the_catch_up_stays_well_inside_the_budget() {
    let h = harness(true, true).await;
    for n in 1..=30 {
        h.working_issue(n, true);
        h.pin(n).await;
    }
    h.github.with(|g| g.calls.clear());
    let mut p = poller();
    let mut cadence = tasks::Cadence::new(60, Some(900));
    let (mut ticks, mut catchups) = (0, 0);
    for now in 0..3600u64 {
        if let Some(due) = cadence.at(now) {
            ticks += 1;
            catchups += usize::from(due.catchup);
            p.webhook_tick(&h.app, due.lane, due.catchup).await.unwrap();
        }
    }
    let calls = h.github.calls();
    let rate = calls.iter().filter(|(n, _)| *n == "rate_limit").count();
    let counted = calls.len() - rate;
    println!(
        "checked {ticks} ticks ({catchups} catch-ups) over 30 issues: {counted} counted GitHub calls, {rate} rate-limit reads"
    );
    assert_eq!(catchups, 4);
    assert!(
        counted < 500,
        "{counted} calls in an hour is not under 10% of 5,000"
    );
    assert_eq!(h.state_of(7).await, Some(ProgressState::InReview));
}

/// D9: a refresh that fails leaves its row pending; the third failure leaves it to the catch-up.
#[tokio::test]
async fn marking_follows_the_pass() {
    let h = harness(true, true).await;
    h.working_issue(8, false);
    h.pin(8).await;
    let mut p = poller();
    p.webhook_tick(&h.app, tasks::Lane::Full, true)
        .await
        .unwrap();
    h.github.with(|g| g.comments_error = Some("boom".into()));
    let comment = serde_json::json!({"action":"created","issue":{"number":8},"repository":{"full_name":REPO}});
    assert_eq!(h.deliver("d-8", "issue_comment", comment).await, 202);
    for attempt in 1..=3 {
        assert_eq!(
            webhooks::pending(&h.app).await.len(),
            1,
            "pending before attempt {attempt}"
        );
        p.refresh(&h.app).await.unwrap();
    }
    assert!(
        webhooks::pending(&h.app).await.is_empty(),
        "given up after 3"
    );
    // A pass marks only the rows it started from: one stored after it waits for the next.
    h.github.with(|g| g.comments_error = None);
    let later =
        serde_json::json!({"action":"edited","issue":{"number":8},"repository":{"full_name":REPO}});
    assert_eq!(h.deliver("d-8b", "issue_comment", later).await, 202);
    assert_eq!(webhooks::pending(&h.app).await.len(), 1);
    p.refresh(&h.app).await.unwrap();
    assert!(webhooks::pending(&h.app).await.is_empty());
}

/// S6: with `tasks::run` running and webhooks on, a delivery changes the overview within 10 s.
#[tokio::test]
async fn the_loop_refreshes_within_seconds() {
    let h = harness(true, true).await;
    h.working_issue(5, true);
    h.pin(5).await;
    let (tx, rx) = tokio::sync::watch::channel(false);
    let running = tokio::spawn(tasks::run(h.app.clone(), rx));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while h.state_of(5).await != Some(ProgressState::InReview) {
        assert!(
            std::time::Instant::now() < deadline,
            "the first tick did not run"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    h.merge(5);
    let sent = std::time::Instant::now();
    assert_eq!(
        h.deliver("d-loop", "pull_request", pr_event("closed", "issue/5-x"))
            .await,
        202
    );
    while h.state_of(5).await != Some(ProgressState::Done) {
        assert!(
            sent.elapsed() < std::time::Duration::from_secs(10),
            "not refreshed within 10 s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    println!(
        "refreshed {} ms after the delivery",
        sent.elapsed().as_millis()
    );
    tx.send(true).unwrap();
    running.await.unwrap();
}

/// S9: the rate limit is read when every other read fails, and when no map is loaded.
#[tokio::test]
async fn the_rate_limit_is_read_on_every_path() {
    let h = harness(false, true).await;
    h.working_issue(5, true);
    h.pin(5).await;
    h.github.with(|g| {
        g.exhausted = true;
        g.calls.clear();
    });
    tasks::Poller::default()
        .tick(&h.app, tasks::Lane::Full)
        .await
        .unwrap();
    let calls = h.github.calls();
    assert!(calls.len() > 1, "the tick tried its reads: {calls:?}");
    assert_eq!(calls.last().map(|c| c.0), Some("rate_limit"));

    let h = harness(false, false).await;
    tasks::Poller::default()
        .tick(&h.app, tasks::Lane::Full)
        .await
        .unwrap();
    assert_eq!(
        h.github.calls().iter().map(|c| c.0).collect::<Vec<_>>(),
        ["rate_limit"],
        "no map loaded: the rate limit still"
    );
}
