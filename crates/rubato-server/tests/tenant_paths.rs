//! Plan 345 task 4 (ADR 0087, 0088, 0089; design §4): tenant routes on the real router. Two
//! tenants, `acme` (installation 11, repos `acme/map` and `acme/api`) and `bcorp` (installation
//! 22, repos `bcorp/map` and `bcorp/web`), each with a workspace called `main`. No dev user:
//! every request carries a real session.

use rubato_protocol::tenant::routes::in_tenant;
use rubato_protocol::workspace::routes::{WORKSPACES, scoped};
use rubato_protocol::{Overview, Role, WorkspaceInfo};
use rubato_server::kernel::config::{Config, GitHubApp, TokenKey};
use rubato_server::kernel::db;
use rubato_server::kernel::github::{Client, FakeGitHub, GitHubApps};
use rubato_server::kernel::state::{App, AppState};
use rubato_server::{admin, app, tenants, workspace_settings};
use std::collections::HashMap;
use std::sync::Arc;

fn map_yaml(name: &str, code: &str) -> String {
    format!(
        "version: 1\nname: {name}\nrepos: [{{ name: {code} }}]\nnodes:\n  - {{ id: storefront, kind: context, repo: {code} }}\n  - {{ id: backend, kind: context, repo: {code} }}\nseams:\n  - {{ id: storefront-backend, from: storefront, to: backend, owner: backend }}\n"
    )
}

struct H {
    url: String,
    app: App,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

/// People: `ada` admin of acme, `mia` member of acme, `bob` admin of bcorp, `zed` in no tenant.
async fn two_tenants() -> H {
    let tmp = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = db::connect(&db_url).await.unwrap();
    let installation = |map_repo: &str, name: &str, code: &str| {
        let f = Arc::new(FakeGitHub::new());
        f.with(|g| {
            g.installation = Some(vec![map_repo.into(), code.into()]);
            g.files.insert(
                (map_repo.into(), "system-map.yml".into(), "main".into()),
                map_yaml(name, code),
            );
            g.branches
                .insert(map_repo.into(), vec![("main".into(), "sha1".into())]);
        });
        f
    };
    let fakes: HashMap<u64, Client> = [
        (
            11,
            installation("acme/map", "Acme shop", "acme/api") as Client,
        ),
        (
            22,
            installation("bcorp/map", "Bcorp site", "bcorp/web") as Client,
        ),
    ]
    .into_iter()
    .collect();
    let github = GitHubApps::installations(
        Arc::new(FakeGitHub::new()),
        Box::new(move |id| {
            fakes
                .get(&id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no installation {id}"))
        }),
    );
    let mut cfg = Config::for_tests(db_url, "unused");
    cfg.dev_user = None;
    cfg.github_app = Some(GitHubApp {
        app_id: 1,
        private_key: "unused".into(),
        installation_id: 11,
    });
    cfg.token_key = Some(TokenKey::from_hex(&"07".repeat(32)).unwrap());
    let app = AppState::with_github(pool, cfg, github);
    app.set_admission(Arc::new(admin::Admission));
    workspace_settings::start(&app).await.unwrap();
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
    for (tenant, login, id, role) in [
        ("acme", "ada", 1, Role::Admin),
        ("acme", "mia", 3, Role::Member),
        ("bcorp", "bob", 2, Role::Admin),
    ] {
        tenants::add_member(&mut tx, tenant, id, login, role, "op", now)
            .await
            .unwrap()
            .unwrap();
    }
    tx.commit().await.unwrap();
    for (login, id) in [("ada", 1), ("bob", 2), ("mia", 3), ("zed", 4)] {
        sqlx::query("INSERT INTO users (login, github_id, created_at) VALUES (?, ?, ?)")
            .bind(login)
            .bind(id)
            .bind(now.to_rfc3339())
            .execute(&app.db)
            .await
            .unwrap();
    }
    tenants::load_routes(&app).await.unwrap();
    let r = admin::refresh_installations(&app).await;
    assert_eq!((r.installations, r.failed, r.added), (2, 0, 4), "{r:?}");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    // The router the binary serves: no path of before tenants is mapped.
    let router = app::router(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    H {
        url: format!("http://{addr}"),
        app,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

impl H {
    async fn cookie(&self, login: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO web_sessions (id, login, created_at) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(login)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&self.app.db)
            .await
            .unwrap();
        format!("rubato_session={}", self.app.signer.sign(&id))
    }

    /// (status, body) of a request as `login`.
    async fn call(
        &self,
        login: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (u16, String) {
        let mut r = self
            .http
            .request(method, format!("{}{path}", self.url))
            .header("cookie", self.cookie(login).await);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let r = r.send().await.unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    /// Workspace `main` in each tenant, made through the API by that tenant's admin.
    async fn with_main(&self) {
        for (admin, tenant, repo) in [("ada", "acme", "acme/map"), ("bob", "bcorp", "bcorp/map")] {
            let (status, body) = self
                .call(
                    admin,
                    reqwest::Method::POST,
                    &in_tenant(tenant, WORKSPACES),
                    Some(serde_json::json!({
                        "id": "main", "map_repo": repo, "map_path": "system-map.yml"
                    })),
                )
                .await;
            assert_eq!(status, 201, "{tenant}: {body}");
        }
    }
}

const GET: reqwest::Method = reqwest::Method::GET;
const POST: reqwest::Method = reqwest::Method::POST;
const PUT: reqwest::Method = reqwest::Method::PUT;
const DELETE: reqwest::Method = reqwest::Method::DELETE;

#[tokio::test]
async fn the_paths_of_before_tenants_are_gone() {
    let h = two_tenants().await;
    h.with_main().await;
    let gone = [
        (GET, "/api/overview"),
        (GET, "/api/w/main/overview"),
        (GET, "/api/map"),
        (POST, "/api/pins"),
        (POST, "/api/queue"),
        (GET, "/api/workspaces"),
        (GET, "/api/settings"),
        (GET, "/api/room?repo=acme/api&number=1&branch=issue/1-x"),
        (DELETE, "/api/w/main"),
    ];
    for (method, path) in &gone {
        let (status, body) = h.call("ada", method.clone(), path, None).await;
        assert_eq!(
            (status, body.as_str()),
            (404, app::UNKNOWN_API_PATH),
            "{method} {path}"
        );
    }
    // The same routes, in the person's tenant.
    let (status, _) = h
        .call("ada", GET, &scoped("acme", "main", "/api/overview"), None)
        .await;
    assert_eq!(status, 200);
    println!("checked {} paths of before tenants", gone.len());
}

#[tokio::test]
async fn a_person_reaches_only_the_tenants_they_are_in() {
    let h = two_tenants().await;
    h.with_main().await;
    let unknown = r#"{"reason":"unknown_tenant"}"#;
    // (who, tenant, expected status of GET overview)
    let cases = [
        ("ada", "acme", 200),
        ("mia", "acme", 200),
        ("bob", "bcorp", 200),
        ("ada", "bcorp", 404),
        ("bob", "acme", 404),
        ("zed", "acme", 404),
        ("ada", "nosuch", 404),
        ("ada", "Not-An-Id", 404),
    ];
    for (who, tenant, want) in cases {
        let (status, body) = h
            .call(who, GET, &scoped(tenant, "main", "/api/overview"), None)
            .await;
        assert_eq!(status, want, "{who} in {tenant}: {body}");
        if want == 404 {
            assert_eq!(
                body, unknown,
                "{who} in {tenant}: a tenant that does not exist and one the person is not in answer alike"
            );
        }
    }
    // No session at all is 401, before any tenant is looked at.
    let r = h
        .http
        .get(format!(
            "{}{}",
            h.url,
            scoped("acme", "main", "/api/overview")
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    // A write is refused the same way, and nothing is written.
    let (status, body) = h
        .call(
            "bob",
            POST,
            &scoped("acme", "main", "/api/pins"),
            Some(serde_json::json!({
                "issue": {"repo": "acme/api", "number": 1},
                "target": {"kind": "node", "id": "backend"}
            })),
        )
        .await;
    assert_eq!((status, body.as_str()), (404, unknown));
    let pins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pins")
        .fetch_one(&h.app.db)
        .await
        .unwrap();
    assert_eq!(pins, 0);
    println!("checked {} person and tenant pairs", cases.len());
}

#[tokio::test]
async fn two_tenants_with_a_workspace_of_the_same_id_stay_apart() {
    let h = two_tenants().await;
    h.with_main().await;
    let overview = |who: &'static str, tenant: &'static str| {
        let h = &h;
        async move {
            let (status, body) = h
                .call(who, GET, &scoped(tenant, "main", "/api/overview"), None)
                .await;
            assert_eq!(status, 200, "{body}");
            serde_json::from_str::<Overview>(&body).unwrap()
        }
    };
    assert_eq!(overview("ada", "acme").await.map.map.name, "Acme shop");
    assert_eq!(overview("bob", "bcorp").await.map.map.name, "Bcorp site");

    // Each tenant lists its own workspace only.
    for (who, tenant, repo) in [("ada", "acme", "acme/map"), ("bob", "bcorp", "bcorp/map")] {
        let (_, body) = h.call(who, GET, &in_tenant(tenant, WORKSPACES), None).await;
        let list: Vec<WorkspaceInfo> = serde_json::from_str(&body).unwrap();
        assert_eq!(
            list.iter()
                .map(|w| (w.id.as_str(), w.map_repo.as_str()))
                .collect::<Vec<_>>(),
            [("main", repo)],
            "{tenant}"
        );
    }

    // A pin and a layout position in acme's `main` do not show in bcorp's `main`.
    let (status, body) = h
        .call(
            "ada",
            POST,
            &scoped("acme", "main", "/api/pins"),
            Some(serde_json::json!({
                "issue": {"repo": "acme/api", "number": 1},
                "target": {"kind": "node", "id": "backend"}
            })),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let (status, body) = h
        .call(
            "ada",
            PUT,
            &scoped("acme", "main", "/api/layout"),
            Some(serde_json::json!({ "positions": { "backend": { "x": 7.0, "y": 9.0 } } })),
        )
        .await;
    assert!((200..300).contains(&status), "{status} {body}");
    let theirs = overview("bob", "bcorp").await;
    assert!(theirs.issues.is_empty(), "{:?}", theirs.issues);
    assert!(theirs.layout.positions.is_empty());
    let ours = overview("ada", "acme").await;
    assert_eq!(ours.issues.len(), 1);
    assert!(ours.layout.positions.contains_key("backend"));
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT tenant_id, workspace_id FROM pins UNION ALL SELECT tenant_id, workspace_id FROM map_layout")
            .fetch_all(&h.app.db)
            .await
            .unwrap();
    assert_eq!(
        rows,
        [
            ("acme".into(), "main".into()),
            ("acme".into(), "main".into())
        ]
    );
}

#[tokio::test]
async fn workspaces_are_an_admins_to_make_and_only_on_the_tenants_own_repositories() {
    let h = two_tenants().await;
    h.with_main().await;
    let new = |id: &str, repo: &str| {
        Some(serde_json::json!({ "id": id, "map_repo": repo, "map_path": "system-map.yml" }))
    };
    // A member is not an admin (ADR 0088).
    let (status, body) = h
        .call(
            "mia",
            POST,
            &in_tenant("acme", WORKSPACES),
            new("second", "acme/map"),
        )
        .await;
    assert_eq!((status, body.as_str()), (403, r#"{"reason":"not_admin"}"#));
    let (status, _) = h.call("mia", DELETE, "/api/t/acme/w/main", None).await;
    assert_eq!(status, 403);
    // An admin of acme cannot point a workspace at bcorp's repository (ADR 0087), and is told
    // nothing about it but that it is not their tenant's.
    let (status, body) = h
        .call(
            "ada",
            POST,
            &in_tenant("acme", WORKSPACES),
            new("theirs", "bcorp/map"),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert!(body.contains("repo_unreadable"), "{body}");
    assert!(!body.contains("Bcorp site"), "{body}");
    // Nor at a repository nobody covers.
    let (status, body) = h
        .call(
            "ada",
            POST,
            &in_tenant("acme", WORKSPACES),
            new("nobody", "stranger/map"),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    // Nor delete another tenant's workspace.
    let (status, _) = h.call("ada", DELETE, "/api/t/bcorp/w/main", None).await;
    assert_eq!(status, 404);
    let held: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workspace_settings WHERE workspace_id = 'main'")
            .fetch_one(&h.app.db)
            .await
            .unwrap();
    assert_eq!(held, 2, "both tenants still hold their `main`");
    // An admin deletes their own.
    let (status, body) = h.call("ada", DELETE, "/api/t/acme/w/main", None).await;
    assert_eq!(status, 204, "{body}");
    let (status, _) = h
        .call("bob", GET, &scoped("bcorp", "main", "/api/overview"), None)
        .await;
    assert_eq!(status, 200, "bcorp's `main` is untouched");
}

/// Every tenant route with an id in its path reads it: the path also carries `{tenant}` (and
/// `{ws}`), so a handler that takes its parameters by position answers 500. None may.
#[tokio::test]
async fn every_tenant_route_with_an_id_in_its_path_reads_it() {
    let h = two_tenants().await;
    h.with_main().await;
    let id = uuid::Uuid::new_v4();
    let t = |p: &str| in_tenant("acme", p);
    let w = |p: &str| scoped("acme", "main", p);
    let routes: Vec<(reqwest::Method, String, Option<serde_json::Value>)> = vec![
        (GET, t(&format!("/api/launches/{id}/messages")), None),
        (
            POST,
            t(&format!("/api/launches/{id}/messages")),
            Some(serde_json::json!({"id": id, "kind": "prompt", "text": "x"})),
        ),
        (GET, t(&format!("/api/launches/{id}/log")), None),
        (POST, t(&format!("/api/launches/{id}/stop")), None),
        (GET, t(&format!("/api/drafts/{id}")), None),
        (GET, t(&format!("/api/drafts/{id}/revisions/1")), None),
        (
            POST,
            t(&format!("/api/room/requests/{id}")),
            Some(serde_json::json!({"accept": true})),
        ),
        (GET, t(&format!("/api/room/voice/speech/{id}")), None),
        (DELETE, w("/api/map/draft/ops/1"), None),
        (DELETE, w("/api/traffic/dismissals/1"), None),
        (GET, w("/api/pulls/acme/api/1"), None),
    ];
    for (method, path, body) in &routes {
        let (status, text) = h.call("ada", method.clone(), path, body.clone()).await;
        assert!(
            status != 500 && text != app::UNKNOWN_API_PATH,
            "{method} {path}: {status} {text}"
        );
    }
    println!("checked {} routes with an id in their path", routes.len());
}
