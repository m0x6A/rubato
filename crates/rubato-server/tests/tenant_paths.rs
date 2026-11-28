//! Plan 345 task 4 (ADR 0087, 0088, 0089; design §4): tenant routes on the real router. Two
//! tenants, `acme` (installation 11, repos `acme/map` and `acme/api`) and `bcorp` (installation
//! 22, repos `bcorp/map` and `bcorp/web`), each with a workspace called `main`. No dev user:
//! every request carries a real session.

use axum::http::Method;
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
    two_tenants_with(|_| {}).await
}

async fn two_tenants_with(configure: impl FnOnce(&mut Config)) -> H {
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
    configure(&mut cfg);
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

// ---- The cross-tenant sweep (plan 345, design §11.2) ----

/// What only bcorp's rows hold. No answer to a member of acme may carry it.
const MARKER: &str = "bcorp-secret";
const B_LAUNCH: &str = "00000000-0000-0000-0000-0000000000b1";
const B_DRAFT: &str = "00000000-0000-0000-0000-0000000000b2";
const B_MESSAGE: &str = "00000000-0000-0000-0000-0000000000b3";
const B_REQUEST: &str = "00000000-0000-0000-0000-0000000000b4";
const B_COMPANION: &str = "00000000-0000-0000-0000-0000000000b5";
const A_COMPANION: &str = "00000000-0000-0000-0000-0000000000a5";
const B_BRANCH: &str = "issue/1-x";

/// One of every resource in bcorp, each carrying [`MARKER`] where it has text; and a companion
/// of ada's own, so owner-only checks pass and the tenant check is what answers.
async fn seed_bcorp(h: &H) {
    let now = chrono::Utc::now().to_rfc3339();
    let stmts: Vec<String> = vec![
        format!(
            "INSERT INTO launches (tenant_id, id, companion_id, user, repo, number, command, created_at, workspace_id) VALUES ('bcorp', '{B_LAUNCH}', '{B_COMPANION}', 'bob', 'bcorp/web', 1, '{{}}', '{now}', 'main')"
        ),
        format!(
            "INSERT INTO plans (tenant_id, repo, number, branch, path, plan, fetched_at) VALUES ('bcorp', 'bcorp/web', 1, '{B_BRANCH}', 'docs/plans/1.md', NULL, '{now}')"
        ),
        format!(
            "INSERT INTO plan_drafts (tenant_id, id, repo, number, branch, owner, created_at) VALUES ('bcorp', '{B_DRAFT}', 'bcorp/web', 1, '{B_BRANCH}', 'bob', '{now}')"
        ),
        format!(
            "INSERT INTO plan_draft_revisions (tenant_id, draft_id, rev, files, missing, plan_sha, bundle_hash, at) VALUES ('bcorp', '{B_DRAFT}', 1, '[{{\"path\":\"docs/plans/1.md\",\"text\":\"{MARKER}\"}}]', '[]', 'sha', 'hash', '{now}')"
        ),
        format!(
            "INSERT INTO room_messages (tenant_id, id, repo, number, branch, author_kind, author, text, at) VALUES ('bcorp', '{B_MESSAGE}', 'bcorp/web', 1, '{B_BRANCH}', 'person', 'bob', '{MARKER}', '{now}')"
        ),
        format!(
            "INSERT INTO room_requests (tenant_id, id, repo, number, branch, owner, launch_id, requested_by, at, state) VALUES ('bcorp', '{B_REQUEST}', 'bcorp/web', 1, '{B_BRANCH}', 'bob', '{B_LAUNCH}', 'bob', '{now}', 'pending')"
        ),
        format!(
            "INSERT INTO session_messages (tenant_id, id, launch_id, companion_id, owner, repo, number, kind, body, author, priority, state, created_at) VALUES ('bcorp', '{B_MESSAGE}', '{B_LAUNCH}', '{B_COMPANION}', 'bob', 'bcorp/web', 1, 'chat', '{{\"kind\":\"chat\",\"text\":\"{MARKER}\"}}', 'bob', 1, 'delivered', '{now}')"
        ),
        format!(
            "INSERT INTO session_log_entries (tenant_id, session_id, prompt_id, role, launch_id, repo, number, kind, text, truncated, mixed, at) VALUES ('bcorp', 's', 'p', 'user', '{B_LAUNCH}', 'bcorp/web', 1, 'prompt', '{MARKER}', 0, 0, '{now}')"
        ),
        format!(
            "INSERT INTO issue_meta (tenant_id, repo, number, title, github_state, url, fetched_at) VALUES ('bcorp', 'bcorp/web', 1, '{MARKER}', 'open', 'u', '{now}')"
        ),
        format!(
            "INSERT INTO open_issues (tenant_id, repo, number, title, url, labels, updated_at) VALUES ('bcorp', 'bcorp/web', 2, '{MARKER}', 'u', '[]', '{now}')"
        ),
        format!(
            "INSERT INTO pairings (companion_id, user, token_hash, hostname, created_at) VALUES ('{B_COMPANION}', 'bob', 'hb', '{MARKER}-laptop', '{now}')"
        ),
        format!(
            "INSERT INTO pairings (companion_id, user, token_hash, hostname, created_at) VALUES ('{A_COMPANION}', 'ada', 'ha', 'ada-laptop', '{now}')"
        ),
        format!(
            "INSERT INTO companion_queue (tenant_id, companion_id, repo, number, position, queued_by, queued_at) VALUES ('bcorp', '{B_COMPANION}', 'bcorp/web', 1, 0, 'bob', '{now}')"
        ),
    ];
    for s in &stmts {
        sqlx::query(s)
            .execute(&h.app.db)
            .await
            .unwrap_or_else(|e| panic!("{e}: {s}"));
    }
    let (status, body) = h
        .call(
            "bob",
            POST,
            &scoped("bcorp", "main", "/api/pins"),
            Some(serde_json::json!({
                "issue": {"repo": "bcorp/web", "number": 1},
                "target": {"kind": "node", "id": "backend"}
            })),
        )
        .await;
    assert_eq!(status, 201, "{body}");
}

/// What a route is called with when a member of acme names bcorp's things inside acme's own
/// paths, and what must come back.
enum Case {
    /// The route names a resource or repository: with bcorp's, it answers 404.
    Foreign(Option<serde_json::Value>),
    /// The route names nothing of another tenant's: it serves acme, and none of bcorp's data.
    Own(Option<serde_json::Value>),
}

/// The fixture of one route, by its mounted template. `None` fails the sweep: a route added to
/// the table without a fixture here is a route nobody swept.
fn fixture(method: &Method, template: &str) -> Option<(String, Case)> {
    use Case::{Foreign, Own};
    let issue = serde_json::json!({"repo": "bcorp/web", "number": 1});
    let room = |extra: serde_json::Value| {
        let mut v = serde_json::json!({"repo": "bcorp/web", "number": 1, "branch": B_BRANCH});
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        Some(v)
    };
    let rest = template
        .strip_prefix("/api/t/{tenant}")
        .expect("a tenant route");
    let m = method.as_str();
    let (path, case): (String, Case) = match (m, rest) {
        ("GET", "/w/{ws}/overview") | ("GET", "/w/{ws}/map") | ("GET", "/w/{ws}/settings") => {
            (rest.into(), Own(None))
        }
        ("GET", "/w/{ws}/map/draft/yaml")
        | ("GET", "/workspaces")
        | ("GET", "/traffic/sources") => (rest.into(), Own(None)),
        ("POST", "/w/{ws}/map/reload")
        | ("POST", "/w/{ws}/map/draft/discard")
        | ("POST", "/w/{ws}/map/draft/restore")
        | ("POST", "/w/{ws}/map/draft/propose")
        | ("POST", "/w/{ws}/map/draft/withdraw")
        | ("DELETE", "/w/{ws}/layout")
        | ("DELETE", "/w/{ws}/layout/grouped") => (rest.into(), Own(None)),
        ("POST", "/w/{ws}/map/draft/ops") => (
            rest.into(),
            Own(Some(
                serde_json::json!({"op": "add_node", "op_id": uuid::Uuid::new_v4(), "node": {"id": "ledger", "kind": "context"}}),
            )),
        ),
        ("DELETE", "/w/{ws}/map/draft/ops/{seq}") => {
            ("/w/{ws}/map/draft/ops/999".into(), Own(None))
        }
        ("PUT", "/w/{ws}/layout") => (
            rest.into(),
            Own(Some(
                serde_json::json!({"positions": {"backend": {"x": 1.0, "y": 2.0}}}),
            )),
        ),
        ("POST", "/w/{ws}/traffic/dismissals") => (
            rest.into(),
            Own(Some(
                serde_json::json!({"target": {"kind": "quiet_seam", "seam": "storefront-backend"}}),
            )),
        ),
        ("DELETE", "/w/{ws}/traffic/dismissals/{id}") => {
            ("/w/{ws}/traffic/dismissals/999".into(), Own(None))
        }
        ("PUT", "/w/{ws}/settings") => (
            rest.into(),
            Own(Some(
                serde_json::json!({"revision": 999, "map_repo": "bcorp/map", "map_path": "system-map.yml", "map_ref": "main"}),
            )),
        ),
        // Deleting acme's own workspace would end the sweep: a workspace it does not hold.
        ("DELETE", "/w/{ws}") => ("/w/nosuch".into(), Foreign(None)),
        ("POST", "/workspaces") => (
            rest.into(),
            Own(Some(
                serde_json::json!({"id": "theirs", "map_repo": "bcorp/map", "map_path": "system-map.yml"}),
            )),
        ),
        ("POST", "/w/{ws}/pins") | ("DELETE", "/w/{ws}/pins") => (
            rest.into(),
            Foreign(Some(
                serde_json::json!({"issue": issue, "target": {"kind": "node", "id": "backend"}}),
            )),
        ),
        ("POST", "/w/{ws}/launch") => (
            rest.into(),
            Foreign(Some(
                serde_json::json!({"issue": issue, "companion_id": A_COMPANION}),
            )),
        ),
        ("POST", "/w/{ws}/plans/import") => (rest.into(), Foreign(room(serde_json::json!({})))),
        ("POST", "/w/{ws}/issues") => (
            rest.into(),
            Foreign(Some(serde_json::json!({
                "request_id": uuid::Uuid::new_v4(), "repo": "bcorp/web",
                "target": {"kind": "node", "id": "backend"}, "title": "t", "summary": "s",
                "does": [], "know": [], "must_not_break": []
            }))),
        ),
        ("POST", "/w/{ws}/plans/approve") => (
            rest.into(),
            Foreign(Some(serde_json::json!({"issue": issue, "sha": "sha"}))),
        ),
        ("POST", "/w/{ws}/plans/drafts/approve") => (
            rest.into(),
            Foreign(room(serde_json::json!({"plan_sha": "sha"}))),
        ),
        ("GET", "/w/{ws}/pulls/{owner}/{repo}/{number}") => {
            ("/w/{ws}/pulls/bcorp/web/1".into(), Foreign(None))
        }
        ("POST", "/w/{ws}/pulls/merge") => (
            rest.into(),
            Foreign(Some(
                serde_json::json!({"repo": "bcorp/web", "number": 1, "head_sha": "sha", "method": "squash"}),
            )),
        ),
        ("POST", "/queue") | ("DELETE", "/queue") => (
            rest.into(),
            Foreign(Some(
                serde_json::json!({"issue": issue, "companion_id": A_COMPANION}),
            )),
        ),
        ("PUT", "/queue") => (
            rest.into(),
            Foreign(Some(
                serde_json::json!({"companion_id": B_COMPANION, "issues": [issue]}),
            )),
        ),
        ("GET", "/launches/{launch_id}/messages") | ("GET", "/launches/{launch_id}/log") => {
            (rest.replace("{launch_id}", B_LAUNCH), Foreign(None))
        }
        ("POST", "/launches/{launch_id}/messages") => (
            rest.replace("{launch_id}", B_LAUNCH),
            Foreign(Some(serde_json::json!({"text": "hello"}))),
        ),
        ("POST", "/launches/{launch_id}/stop") => {
            (rest.replace("{launch_id}", B_LAUNCH), Foreign(None))
        }
        ("GET", "/drafts/{draft_id}") => (rest.replace("{draft_id}", B_DRAFT), Foreign(None)),
        ("GET", "/drafts/{draft_id}/revisions/{rev}") => (
            rest.replace("{draft_id}", B_DRAFT).replace("{rev}", "1"),
            Foreign(None),
        ),
        ("GET", "/room") => (
            format!("/room?repo=bcorp/web&number=1&branch={B_BRANCH}"),
            Foreign(None),
        ),
        ("POST", "/room/messages") => (
            rest.into(),
            Foreign(room(serde_json::json!({"text": "hi"}))),
        ),
        ("POST", "/room/agents") => (
            rest.into(),
            Foreign(room(serde_json::json!({"launch_id": B_LAUNCH}))),
        ),
        ("DELETE", "/room/agents") => (
            rest.into(),
            Foreign(room(serde_json::json!({"handle": "bob-agent"}))),
        ),
        ("POST", "/room/requests/{request_id}") => (
            rest.replace("{request_id}", B_REQUEST),
            Foreign(Some(serde_json::json!({"answer": "accept"}))),
        ),
        ("POST", "/room/voice/token") => (rest.into(), Foreign(room(serde_json::json!({})))),
        ("POST", "/room/voice/utterance") => (
            format!(
                "/room/voice/utterance?repo=bcorp/web&number=1&branch={B_BRANCH}&id={}",
                uuid::Uuid::new_v4()
            ),
            Foreign(None),
        ),
        ("GET", "/room/voice/speech/{message_id}") => {
            (rest.replace("{message_id}", B_MESSAGE), Foreign(None))
        }
        ("POST", "/room/voice/join-failed") => (
            rest.into(),
            Foreign(room(serde_json::json!({"reason": "other"}))),
        ),
        _ => return None,
    };
    Some((path, case))
}

/// Whether an answer to a member of acme is as it must be. `Err` says what is wrong.
fn judge(case: &Case, status: u16, body: &str) -> Result<(), String> {
    if body.contains(MARKER) {
        return Err(format!("the answer carries bcorp's data: {status} {body}"));
    }
    if status >= 500 {
        return Err(format!("a server error: {status} {body}"));
    }
    if body == app::UNKNOWN_API_PATH {
        return Err("the route was not reached".into());
    }
    match case {
        Case::Foreign(_) if status != 404 => Err(format!(
            "bcorp's resource inside acme's path answered {status}, not 404: {body}"
        )),
        _ => Ok(()),
    }
}

/// Design §11.2: every tenant route, called by a member of acme only, with bcorp's tenant, and
/// with bcorp's ids and repositories inside acme's own paths. Voice and the orchestrator are on,
/// so no route hides behind a flag.
#[tokio::test]
async fn the_cross_tenant_sweep_finds_nothing() {
    let h = two_tenants_with(|c| {
        c.orchestrator = true;
        c.voice = Some(rubato_server::kernel::config::VoiceConfig {
            livekit: None,
            speech: Some(rubato_server::kernel::config::SpeechConfig::Fake),
        });
    })
    .await;
    h.with_main().await;
    seed_bcorp(&h).await;
    let counts = |h: &H| {
        let db = h.app.db.clone();
        async move {
            let mut out = Vec::new();
            for t in [
                "launches",
                "plans",
                "plan_drafts",
                "room_messages",
                "room_requests",
                "room_agents",
                "session_messages",
                "session_log_entries",
                "pins",
                "companion_queue",
                "issue_meta",
                "issue_creations",
                "plan_approvals",
                "rubato_approvals",
                "pr_merges",
            ] {
                let (bcorp, acme_on_theirs): (i64, i64) = sqlx::query_as(&format!(
                    "SELECT COALESCE(SUM(tenant_id = 'bcorp'), 0), COALESCE(SUM(tenant_id <> 'bcorp' AND repo = 'bcorp/web'), 0) FROM {t}"
                ))
                .fetch_one(&db)
                .await
                .unwrap();
                out.push((t, bcorp, acme_on_theirs));
            }
            out
        }
    };
    let before = counts(&h).await;

    let table = app::tenant_route_table();
    let mut problems = Vec::new();
    for (method, template) in &table {
        // 1. In bcorp's own paths: a member of acme is told the tenant does not exist.
        let theirs = template
            .replace("{tenant}", "bcorp")
            .replace("{ws}", "main")
            .replace("{launch_id}", B_LAUNCH)
            .replace("{draft_id}", B_DRAFT)
            .replace("{request_id}", B_REQUEST)
            .replace("{message_id}", B_MESSAGE)
            .replace("{owner}/{repo}/{number}", "bcorp/web/1")
            .replace("{rev}", "1")
            .replace("{seq}", "1")
            .replace("{id}", "1");
        let (status, body) = h.call("ada", method.clone(), &theirs, None).await;
        if (status, body.as_str()) != (404, r#"{"reason":"unknown_tenant"}"#) {
            problems.push(format!("{method} {theirs}: {status} {body}"));
        }
        // 2. In acme's own paths, naming bcorp's things.
        let Some((path, case)) = fixture(method, template) else {
            problems.push(format!(
                "{method} {template}: no fixture; a route nobody swept"
            ));
            continue;
        };
        let path = format!("/api/t/acme{path}").replace("{ws}", "main");
        let body = match &case {
            Case::Foreign(b) | Case::Own(b) => b.clone(),
        };
        let (status, text) = h.call("ada", method.clone(), &path, body).await;
        if let Err(why) = judge(&case, status, &text) {
            problems.push(format!("{method} {path}: {why}"));
        }
    }
    let after = counts(&h).await;
    for ((t, b0, a0), (_, b1, a1)) in before.iter().zip(&after) {
        if b0 != b1 {
            problems.push(format!("{t}: bcorp's rows changed, {b0} to {b1}"));
        }
        if a1 > a0 {
            problems.push(format!(
                "{t}: a row of bcorp's repository was written outside bcorp"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
    println!(
        "swept {} tenant routes twice: as another tenant, and with another tenant's resources",
        table.len()
    );
}

/// must fail: an answer that carries another tenant's data, one that serves another tenant's
/// resource, and a route with no fixture are each reported.
#[test]
fn the_sweep_reports_a_leak_a_served_foreign_resource_and_an_unswept_route() {
    assert!(
        judge(
            &Case::Own(None),
            200,
            &format!("{{\"title\":\"{MARKER}\"}}")
        )
        .is_err()
    );
    assert!(judge(&Case::Foreign(None), 200, "{}").is_err());
    assert!(judge(&Case::Foreign(None), 422, r#"{"reason":"unknown_repo"}"#).is_err());
    assert!(judge(&Case::Foreign(None), 500, "").is_err());
    assert!(judge(&Case::Foreign(None), 404, app::UNKNOWN_API_PATH).is_err());
    assert!(judge(&Case::Foreign(None), 404, r#"{"reason":"other_tenant"}"#).is_ok());
    assert!(judge(&Case::Own(None), 200, "{}").is_ok());
    assert!(fixture(&GET, "/api/t/{tenant}/w/{ws}/overview").is_some());
    assert!(
        fixture(&GET, "/api/t/{tenant}/w/{ws}/a-new-route").is_none(),
        "a route added to the table without a fixture fails the sweep"
    );
}
