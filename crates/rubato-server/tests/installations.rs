//! Plan 345 task 3 (ADR 0087, design §3.3, §5): one GitHub client per App installation, each
//! attached to one tenant. Two tenants here: `acme` with installation 11 covering
//! `m0x6A/rubato` (the map's own repo) and `bcorp` with installation 22 covering `bcorp/web`.
//! Workspace `main` is acme's; its map also names `bcorp/web`, another tenant's repository,
//! and `stranger/repo`, which no installation covers. Neither is acme's to read.

use rubato_server::kernel::config::{Config, WebhookSecret};
use rubato_server::kernel::db;
use rubato_server::kernel::github::{Client, FakeGitHub, GitHubApps};
use rubato_server::kernel::state::{App, AppState};
use rubato_server::kernel::tenancy::TenantId;
use rubato_server::{admin, app, map, tasks, tenants, webhooks};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const SECRET: &str = "test-webhook-secret";
const REPO: &str = "m0x6A/rubato";
const BCORP: &str = "bcorp/web";
const STRANGER: &str = "stranger/repo";

struct Two {
    url: String,
    app: App,
    acme: Arc<FakeGitHub>,
    bcorp: Arc<FakeGitHub>,
    person: Arc<FakeGitHub>,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

fn map_yaml() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let yaml = std::fs::read_to_string(root.join("docs/design/rubato-map.yml")).unwrap();
    yaml.replacen(
        "\nnodes:\n",
        "\nnodes:\n  - id: bcorp-web\n    kind: context\n    repo: bcorp/web\n    description: another tenant's repository\n\n  - id: stranger-app\n    kind: context\n    repo: stranger/repo\n    description: a repository no installation covers\n\n",
        1,
    )
}

fn t(s: &str) -> TenantId {
    TenantId::parse(s).unwrap()
}

async fn two() -> Two {
    let tmp = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}/test.db?mode=rwc", tmp.path().display());
    let pool = db::connect(&db_url).await.unwrap();
    let (acme, bcorp, person) = (
        Arc::new(FakeGitHub::new()),
        Arc::new(FakeGitHub::new()),
        Arc::new(FakeGitHub::new()),
    );
    acme.with(|g| {
        g.installation = Some(vec![REPO.into()]);
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
    });
    bcorp.with(|g| {
        g.installation = Some(vec![BCORP.into()]);
        g.branches.insert(BCORP.into(), vec![]);
    });
    let fakes: HashMap<u64, Client> = [(11, acme.clone() as Client), (22, bcorp.clone() as Client)]
        .into_iter()
        .collect();
    let github = GitHubApps::installations(
        person.clone(),
        Box::new(move |id| {
            fakes
                .get(&id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("installation {id} mints no token"))
        }),
    );
    let mut cfg = Config::for_tests(db_url, "m0x6A");
    cfg.github_webhook_secret = WebhookSecret::from_env_value(Some(SECRET.into()));
    let app = AppState::with_github(pool, cfg, github);
    let now = chrono::Utc::now();
    let mut tx = app.db.begin().await.unwrap();
    for (tenant, id) in [("acme", 11), ("bcorp", 22)] {
        tenants::create_tenant(&mut tx, tenant, tenant, "op", now)
            .await
            .unwrap()
            .unwrap();
        tenants::attach_installation(&mut tx, tenant, id, "op", now)
            .await
            .unwrap()
            .unwrap();
    }
    tx.commit().await.unwrap();
    tenants::load_routes(&app).await.unwrap();
    let r = admin::refresh_installations(&app).await;
    assert_eq!((r.installations, r.failed, r.added), (2, 0, 2), "{r:?}");
    // Workspace `main` of tenant acme, on the map in acme's own repository.
    app.set_workspaces(rubato_server::kernel::workspaces::Registry {
        entries: vec![rubato_server::kernel::workspaces::WorkspaceEntry {
            tenant: t("acme"),
            id: "main".into(),
            name: None,
            map_repo: REPO.into(),
            map_path: "docs/design/rubato-map.yml".into(),
            map_ref: "main".into(),
            map_file: None,
            settings: Default::default(),
        }],
        source: rubato_server::kernel::workspaces::Source::Database,
    });
    map::reload(&app).await.unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app::router(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Two {
        url: format!("http://{addr}"),
        app,
        acme,
        bcorp,
        person,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

impl Two {
    async fn pin(&self, repo: &str, number: i64) {
        sqlx::query(
            "INSERT INTO pins (tenant_id, workspace_id, repo, number, kind, target, pinned_by, pinned_at) VALUES ('acme', 'main', ?, ?, 'node', 'rubato-server', 'm0x6A', ?)",
        )
        .bind(repo)
        .bind(number)
        .bind(rubato_server::kernel::db::ts(chrono::Utc::now()))
        .execute(&self.app.db)
        .await
        .unwrap();
    }

    fn calls_naming(&self, repo: &str) -> Vec<(&'static str, &'static str, String)> {
        [
            ("acme", &self.acme),
            ("bcorp", &self.bcorp),
            ("person", &self.person),
        ]
        .into_iter()
        .flat_map(|(who, f)| {
            f.calls()
                .into_iter()
                .filter(|(_, a)| a.to_ascii_lowercase().contains(repo))
                .map(move |(m, a)| (who, m, a))
        })
        .collect()
    }

    /// Post a signed delivery; (status, body).
    async fn deliver(&self, id: &str, event: &str, body: serde_json::Value) -> (u16, String) {
        let body = body.to_string();
        let r = self
            .http
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
            .unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(&self.app.db)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn each_repository_is_read_only_through_the_installation_of_its_tenant() {
    let h = two().await;
    h.acme.with(|g| {
        g.issues.insert(
            (REPO.into(), 2),
            rubato_server::kernel::github::IssueInfo {
                title: "acme's own issue".into(),
                state: "open".into(),
                url: "https://github.com/m0x6A/rubato/issues/2".into(),
            },
        );
    });
    h.pin(REPO, 2).await;
    h.pin(STRANGER, 1).await;
    tasks::poll_once(&h.app).await.unwrap();

    let stranger = h.calls_naming(STRANGER);
    assert!(
        stranger.is_empty(),
        "no installation covers it, so nothing is called: {stranger:?}"
    );
    let ours = h.calls_naming("m0x6a/rubato");
    assert!(!ours.is_empty());
    assert!(ours.iter().all(|(who, _, _)| *who == "acme"), "{ours:?}");
    // Design §5.2, §11.3: acme's map names bcorp's repository. That is a finding for acme, never
    // a read: not through acme's installation, and not through bcorp's on acme's behalf.
    let theirs = h.calls_naming(BCORP);
    assert!(theirs.is_empty(), "{theirs:?}");
    // Each installation's own budget is read, through itself.
    for f in [&h.acme, &h.bcorp] {
        assert!(f.calls().iter().any(|(m, _)| *m == "rate_limit"));
    }
    assert!(h.person.calls().is_empty(), "{:?}", h.person.calls());
    let uncovered: Vec<(String, String)> =
        sqlx::query_as("SELECT tenant_id, repo FROM uncovered_repos ORDER BY repo")
            .fetch_all(&h.app.db)
            .await
            .unwrap();
    assert_eq!(
        uncovered,
        [
            ("acme".to_owned(), BCORP.to_owned()),
            ("acme".to_owned(), STRANGER.to_owned())
        ],
        "both are uncovered for the tenant whose map names them"
    );
    // What was read about acme's own issue is acme's row.
    let meta: Vec<(String, String)> =
        sqlx::query_as("SELECT tenant_id, repo FROM issue_meta ORDER BY repo")
            .fetch_all(&h.app.db)
            .await
            .unwrap();
    assert_eq!(meta, [("acme".to_owned(), REPO.to_owned())]);
    println!(
        "checked {} calls across 3 repositories and 2 installations",
        ours.len()
    );
}

#[tokio::test]
async fn a_delivery_from_an_installation_no_tenant_attached_changes_nothing() {
    let h = two().await;
    let (status, body) = h
        .deliver(
            "d-1",
            "issues",
            serde_json::json!({
                "action": "opened",
                "issue": {"number": 5},
                "repository": {"full_name": REPO},
                "installation": {"id": 99},
            }),
        )
        .await;
    assert_eq!(
        (status, body.as_str()),
        (200, r#"{"outcome":"unattached"}"#)
    );
    assert_eq!(h.scalar("SELECT COUNT(*) FROM webhook_deliveries").await, 0);
    // From an attached installation the same delivery is stored as before.
    let (status, _) = h
        .deliver(
            "d-2",
            "issues",
            serde_json::json!({
                "action": "opened",
                "issue": {"number": 5},
                "repository": {"full_name": REPO},
                "installation": {"id": 11},
            }),
        )
        .await;
    assert_eq!(status, 202);
}

#[tokio::test]
async fn an_installation_deleted_on_github_is_detached_and_its_rows_kept() {
    let h = two().await;
    let now = rubato_server::kernel::db::ts(chrono::Utc::now());
    for sql in [
        "INSERT INTO room_agents (tenant_id, id, repo, number, branch, launch_id, owner, called_by, joined_at) VALUES ('bcorp', 'a1', 'bcorp/web', 3, 'issue/3-x', 'l1', 'ada', 'ada', ?)",
        "INSERT INTO room_requests (tenant_id, id, repo, number, branch, owner, launch_id, requested_by, at, state) VALUES ('bcorp', 'r1', 'bcorp/web', 3, 'issue/3-x', 'ada', 'l1', 'bob', ?, 'pending')",
        "INSERT INTO companion_queue (companion_id, repo, number, position, queued_by, queued_at) VALUES ('00000000-0000-0000-0000-0000000000aa', 'bcorp/web', 3, 0, 'ada', ?)",
    ] {
        sqlx::query(sql)
            .bind(&now)
            .execute(&h.app.db)
            .await
            .unwrap();
    }
    let (status, _) = h
        .deliver(
            "d-del",
            "installation",
            serde_json::json!({"action": "deleted", "installation": {"id": 22}}),
        )
        .await;
    assert_eq!(status, 202);
    tasks::Poller::default().refresh(&h.app).await.unwrap();

    assert_eq!(h.app.github.tenant_of(BCORP), None);
    assert_eq!(h.app.github.tenant_of(REPO), Some(t("acme")));
    assert_eq!(
        h.scalar(
            "SELECT COUNT(*) FROM tenant_repos WHERE repo = 'bcorp/web' AND detached_at IS NOT NULL"
        )
        .await,
        1,
        "tombstoned, not deleted, until an operator purges"
    );
    assert_eq!(
        h.scalar("SELECT COUNT(*) FROM room_agents WHERE left_at IS NULL")
            .await,
        0
    );
    assert_eq!(
        h.scalar("SELECT COUNT(*) FROM room_requests WHERE state = 'pending'")
            .await,
        0
    );
    assert_eq!(h.scalar("SELECT COUNT(*) FROM companion_queue").await, 0);
    assert_eq!(
        h.scalar("SELECT COUNT(*) FROM webhook_deliveries WHERE refreshed_at IS NULL")
            .await,
        0,
        "the delivery is settled"
    );
    // Deliveries from it are no longer admitted.
    let (status, body) = h
        .deliver(
            "d-after",
            "installation",
            serde_json::json!({"action": "unsuspend", "installation": {"id": 22}}),
        )
        .await;
    assert_eq!(
        (status, body.as_str()),
        (200, r#"{"outcome":"unattached"}"#)
    );
}

#[tokio::test]
async fn a_suspended_installation_is_not_read_until_github_unsuspends_it() {
    let h = two().await;
    h.pin(REPO, 2).await;
    let (status, _) = h
        .deliver(
            "d-s",
            "installation",
            serde_json::json!({"action": "suspend", "installation": {"id": 11}}),
        )
        .await;
    assert_eq!(status, 202);
    tasks::Poller::default().refresh(&h.app).await.unwrap();
    assert_eq!(h.app.github.tenant_of(REPO), None);
    h.acme.with(|g| g.calls.clear());
    tasks::poll_once(&h.app).await.unwrap();
    assert!(h.acme.calls().is_empty(), "{:?}", h.acme.calls());

    let (status, _) = h
        .deliver(
            "d-u",
            "installation",
            serde_json::json!({"action": "unsuspend", "installation": {"id": 11}}),
        )
        .await;
    assert_eq!(status, 202);
    tasks::Poller::default().refresh(&h.app).await.unwrap();
    assert_eq!(h.app.github.tenant_of(REPO), Some(t("acme")));
}

#[tokio::test]
async fn repositories_added_to_an_installation_join_its_tenant() {
    let h = two().await;
    h.acme.with(|g| {
        g.installation = Some(vec![REPO.into(), "m0x6A/other".into()]);
    });
    let (status, _) = h
        .deliver(
            "d-add",
            "installation_repositories",
            serde_json::json!({
                "action": "added",
                "installation": {"id": 11},
                "repositories_added": [{"full_name": "m0x6A/other"}],
            }),
        )
        .await;
    assert_eq!(status, 202);
    tasks::Poller::default().refresh(&h.app).await.unwrap();
    assert_eq!(h.app.github.tenant_of("m0x6A/other"), Some(t("acme")));

    // Removed again: tombstoned with the next listing.
    h.acme.with(|g| g.installation = Some(vec![REPO.into()]));
    let (status, _) = h
        .deliver(
            "d-rm",
            "installation_repositories",
            serde_json::json!({
                "action": "removed",
                "installation": {"id": 11},
                "repositories_removed": [{"full_name": "m0x6A/other"}],
            }),
        )
        .await;
    assert_eq!(status, 202);
    tasks::Poller::default().refresh(&h.app).await.unwrap();
    assert_eq!(h.app.github.tenant_of("m0x6A/other"), None);
}
