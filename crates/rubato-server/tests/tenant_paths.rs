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
use rubato_server::kernel::tenancy::{TenantId, WsKey};
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

/// The accounts GitHub knows: the four with a login here, and `Carol`, who has never logged in.
fn people() -> HashMap<String, (i64, String)> {
    [("ada", 1), ("bob", 2), ("mia", 3), ("zed", 4), ("Carol", 5)]
        .into_iter()
        .map(|(login, id)| (login.to_ascii_lowercase(), (id, login.to_owned())))
        .collect()
}

/// People: `ada` admin of acme, `mia` member of acme, `bob` admin of bcorp, `zed` in no tenant
/// and the operator.
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
            g.users = people();
            g.files.insert(
                (map_repo.into(), "system-map.yml".into(), "main".into()),
                map_yaml(name, code),
            );
            g.branches
                .insert(map_repo.into(), vec![("main".into(), "sha1".into())]);
        });
        f
    };
    let person = Arc::new(FakeGitHub::new());
    person.with(|g| {
        g.users = people();
        g.app_installations = [
            (11, "acme", "Organization"),
            (22, "bcorp", "Organization"),
            (33, "newco", "User"),
        ]
        .into_iter()
        .map(
            |(id, login, kind)| rubato_server::kernel::github::AppInstallation {
                id,
                account_login: login.into(),
                account_type: kind.into(),
                suspended: false,
            },
        )
        .collect();
    });
    let fakes: HashMap<u64, Client> = [
        (
            11,
            installation("acme/map", "Acme shop", "acme/api") as Client,
        ),
        (
            22,
            installation("bcorp/map", "Bcorp site", "bcorp/web") as Client,
        ),
        // Installed, attached to no tenant yet.
        (
            33,
            installation("newco/map", "Newco", "newco/app") as Client,
        ),
    ]
    .into_iter()
    .collect();
    let github = GitHubApps::installations(
        person,
        Box::new(move |id| {
            fakes
                .get(&id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no installation {id}"))
        }),
    );
    let mut cfg = Config::for_tests(db_url, "unused");
    cfg.dev_user = None;
    // `zed` is the operator, and in no tenant.
    cfg.operators = vec![4];
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
        // Members: bcorp's admin (GitHub id 2) is nobody in acme.
        ("GET", "/members") => (rest.into(), Own(None)),
        ("POST", "/members") => (
            rest.into(),
            Own(Some(serde_json::json!({"login": "nobody-by-this-name"}))),
        ),
        ("PUT", "/members/{github_id}") => (
            "/members/2".into(),
            Foreign(Some(serde_json::json!({"role": "member"}))),
        ),
        ("DELETE", "/members/{github_id}") => ("/members/2".into(), Foreign(None)),
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
            .replace("{id}", "1")
            .replace("{github_id}", "2");
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

// ---- The browser socket is one tenant's (design §6.1) ----

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl H {
    /// `GET /ws/web?t=<tenant>` as `login`: the socket, or the status of the refusal.
    async fn socket(&self, login: &str, tenant: Option<&str>) -> Result<Socket, u16> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let q = tenant.map(|t| format!("?t={t}")).unwrap_or_default();
        let mut req = format!("ws://{}/ws/web{q}", self.url.trim_start_matches("http://"))
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("cookie", self.cookie(login).await.parse().unwrap());
        match tokio_tungstenite::connect_async(req).await {
            Ok((ws, _)) => Ok(ws),
            Err(tokio_tungstenite::tungstenite::Error::Http(r)) => Err(r.status().as_u16()),
            Err(e) => panic!("{e}"),
        }
    }
}

/// Every text frame that arrives within `wait`.
async fn frames(ws: &mut Socket, wait: std::time::Duration) -> Vec<String> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    while let Ok(Some(Ok(m))) = tokio::time::timeout(wait, ws.next()).await {
        if let tokio_tungstenite::tungstenite::Message::Text(t) = m {
            out.push(t.to_string());
        }
    }
    out
}

#[tokio::test]
async fn a_socket_is_opened_for_a_tenant_one_is_in_and_hears_only_that_tenant() {
    let h = two_tenants().await;
    h.with_main().await;
    // No tenant, a tenant one is not in and one that does not exist: the same 404.
    for (login, tenant) in [
        ("ada", None),
        ("ada", Some("bcorp")),
        ("ada", Some("nosuch")),
        ("ada", Some("NOT-A-TENANT")),
        ("zed", Some("acme")),
    ] {
        assert_eq!(
            h.socket(login, tenant).await.err(),
            Some(404),
            "{login} {tenant:?}"
        );
    }
    let mut ada = h.socket("ada", Some("acme")).await.unwrap();
    let mut bob = h.socket("bob", Some("bcorp")).await.unwrap();
    for _ in 0..200 {
        if h.app.web_tx.receiver_count() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let acme = TenantId::parse("acme").unwrap();
    let bcorp = TenantId::parse("bcorp").unwrap();
    // One of every kind of event, in each tenant, and one that concerns every tenant.
    for (t, mark) in [(&acme, "for-acme"), (&bcorp, "for-bcorp")] {
        h.app.notify_in(t, mark);
        h.app.notify_ws(mark, &WsKey::new(t.clone(), "main"));
        h.app.notify_resource(
            t,
            rubato_protocol::WebResource::Room {
                repo: format!("{t}/web"),
                number: 1,
                branch: mark.into(),
            },
        );
    }
    h.app.notify("poll");
    let wait = std::time::Duration::from_millis(300);
    let to_ada = frames(&mut ada, wait).await;
    let to_bob = frames(&mut bob, wait).await;
    assert_eq!(to_ada.len(), 4, "{to_ada:?}");
    assert_eq!(to_bob.len(), 4, "{to_bob:?}");
    assert!(to_ada.iter().all(|f| !f.contains("bcorp")), "{to_ada:?}");
    assert!(to_bob.iter().all(|f| !f.contains("acme")), "{to_bob:?}");
    println!("checked 5 refused upgrades and 7 events on 2 sockets");
}

#[tokio::test]
async fn a_socket_closes_when_its_person_leaves_the_tenant() {
    let h = two_tenants_with(|c| {
        c.web_membership_check = std::time::Duration::from_millis(100);
    })
    .await;
    let mut mia = h.socket("mia", Some("acme")).await.unwrap();
    let mut ada = h.socket("ada", Some("acme")).await.unwrap();
    let mut tx = h.app.db.begin().await.unwrap();
    tenants::remove_member(&mut tx, "acme", 3)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    use futures_util::StreamExt;
    let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match mia.next().await {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(Some(f)))) => {
                    return Some(u16::from(f.code));
                }
                Some(Ok(_)) => {}
                _ => return None,
            }
        }
    })
    .await
    .expect("the socket is closed within the check interval");
    assert_eq!(
        closed,
        Some(rubato_protocol::web_link::routes::CLOSE_NOT_A_MEMBER)
    );
    // Nothing more reaches the removed person; a member still hears the tenant.
    h.app.notify_in(&TenantId::parse("acme").unwrap(), "after");
    let to_ada = frames(&mut ada, std::time::Duration::from_millis(300)).await;
    assert_eq!(to_ada.len(), 1, "{to_ada:?}");
    assert_eq!(h.socket("mia", Some("acme")).await.err(), Some(404));
}

// ---- A companion acts in a tenant only while its owner is a member (design §6.3) ----

#[tokio::test]
async fn a_launch_of_a_person_who_left_the_tenant_takes_nothing_from_its_companion() {
    let h = two_tenants().await;
    h.app.set_admission(std::sync::Arc::new(admin::Admission));
    let launch: uuid::Uuid = "00000000-0000-0000-0000-0000000000a9".parse().unwrap();
    sqlx::query(&format!(
        "INSERT INTO launches (tenant_id, id, companion_id, user, repo, number, command, created_at, workspace_id) VALUES ('acme', '{launch}', '{A_COMPANION}', 'mia', 'acme/api', 1, '{{}}', '2026-11-28T00:00:00Z', 'main')"
    ))
    .execute(&h.app.db)
    .await
    .unwrap();
    let acting = || async {
        rubato_server::launch::acting_target(&h.app, launch)
            .await
            .unwrap()
            .map(|t| t.user)
    };
    assert_eq!(acting().await.as_deref(), Some("mia"));
    let mut tx = h.app.db.begin().await.unwrap();
    tenants::remove_member(&mut tx, "acme", 3)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    // The launch still exists; what its companion sends for it, and what would be pushed to
    // it, finds no launch to act for.
    assert!(
        rubato_server::launch::launch_target(&h.app.db, launch)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(acting().await, None);
    // A member of another tenant only is no member of this one.
    sqlx::query("UPDATE launches SET user = 'bob' WHERE id = ?")
        .bind(launch.to_string())
        .execute(&h.app.db)
        .await
        .unwrap();
    assert_eq!(acting().await, None);
}

// ---- The context graph goes to a tenant's members only (design §6.4) ----

mod graph {
    use super::*;
    use rubato_protocol::graph::{GraphNode, kind, routes as graph_routes, source_key};
    use rubato_protocol::pairing::routes as pr;
    use rubato_protocol::{
        PairApproveRequest, PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse,
    };
    use rubato_server::context_graph;
    use rubato_server::kernel::config::ContextGraphConfig;

    async fn with_graph() -> H {
        let h = two_tenants_with(|c| {
            let dir = std::env::temp_dir().join(format!("rubato-graph-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            c.context_graph = Some(ContextGraphConfig {
                regen_secs: 900,
                database_url: format!("sqlite://{}/graph.db?mode=rwc", dir.display()),
            });
        })
        .await;
        context_graph::start(&h.app).await.unwrap();
        h
    }

    /// One built source, holding one node whose name is the key itself.
    async fn built(h: &H, key: &str) {
        let store = context_graph::store(&h.app).unwrap();
        let source_kind = source_key::kind_of(key).unwrap();
        let build = rubato_graph::Build {
            source: key.to_owned(),
            kind: source_kind.into(),
            head_sha: None,
            map_version: None,
            generator: context_graph::GENERATOR,
            generated_at: chrono::Utc::now(),
            truncated: false,
            nodes: vec![GraphNode {
                id: format!("file:{key}:x"),
                kind: kind::node::FILE.into(),
                name: key.to_owned(),
                repo: None,
                path: None,
                line: None,
                end_line: None,
                summary: None,
                detail: Default::default(),
            }],
            edges: vec![],
        };
        let g = store.begin_build(key, source_kind).await.unwrap();
        store
            .replace_source(Some(g), build, chrono::Utc::now())
            .await
            .unwrap();
    }

    impl H {
        /// A companion paired by `login`: its pairing token.
        async fn paired(&self, login: &str) -> String {
            let post = |path: &'static str, body: serde_json::Value| {
                self.http.post(format!("{}{path}", self.url)).json(&body)
            };
            let start: PairStartResponse = post(
                pr::PAIR_START,
                serde_json::to_value(PairStartRequest {
                    companion_id: uuid::Uuid::new_v4(),
                    hostname: "h".into(),
                    companion_version: "t".into(),
                })
                .unwrap(),
            )
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
            let approved = post(
                pr::PAIR_APPROVE,
                serde_json::to_value(PairApproveRequest {
                    user_code: start.user_code.clone(),
                })
                .unwrap(),
            )
            .header("cookie", self.cookie(login).await)
            .send()
            .await
            .unwrap();
            assert_eq!(approved.status(), 200, "{login} approves");
            let poll: PairPollResponse = post(
                pr::PAIR_POLL,
                serde_json::to_value(PairPollRequest {
                    device_code: start.device_code,
                })
                .unwrap(),
            )
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
            let PairPollResponse::Approved { token, .. } = poll else {
                panic!("{poll:?}")
            };
            token
        }

        async fn snapshot(&self, token: &str, source: &str) -> (u16, String) {
            let r = self
                .http
                .get(format!(
                    "{}{}?source={source}",
                    self.url,
                    graph_routes::GRAPH_SNAPSHOT
                ))
                .bearer_auth(token)
                .send()
                .await
                .unwrap();
            let status = r.status().as_u16();
            let reason = if status == 200 {
                String::new()
            } else {
                r.json::<serde_json::Value>().await.unwrap()["reason"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            };
            (status, reason)
        }
    }

    const ACME: [&str; 3] = ["repo/acme/api", "workspace/acme/main", "team/acme"];
    const BCORP: [&str; 3] = ["repo/bcorp/web", "workspace/bcorp/main", "team/bcorp"];

    #[tokio::test]
    async fn a_pairing_reads_only_the_sources_of_its_owners_tenants() {
        let h = with_graph().await;
        for key in ACME.iter().chain(&BCORP) {
            built(&h, key).await;
        }
        // Sources of no tenant: the keys of before tenants, and a repository nobody installed.
        let store = context_graph::store(&h.app).unwrap();
        for (key, k) in [
            ("team", kind::source::TEAM),
            ("repo/stranger/repo", kind::source::REPO),
        ] {
            let g = store.begin_build(key, k).await.unwrap();
            store
                .replace_source(
                    Some(g),
                    rubato_graph::Build {
                        source: key.into(),
                        kind: k.into(),
                        head_sha: None,
                        map_version: None,
                        generator: context_graph::GENERATOR,
                        generated_at: chrono::Utc::now(),
                        truncated: false,
                        nodes: vec![],
                        edges: vec![],
                    },
                    chrono::Utc::now(),
                )
                .await
                .unwrap();
        }
        let (ada, bob, zed) = (
            h.paired("ada").await,
            h.paired("bob").await,
            h.paired("zed").await,
        );
        let mut checked = 0;
        for (token, own, other) in [(&ada, ACME, BCORP), (&bob, BCORP, ACME)] {
            for key in own {
                assert_eq!(h.snapshot(token, key).await.0, 200, "{key}");
                checked += 1;
            }
            // Another tenant's source is answered as one that does not exist.
            for key in other.iter().chain(&["team", "repo/stranger/repo"]) {
                assert_eq!(
                    h.snapshot(token, key).await,
                    (404, "unknown_source".into()),
                    "{key}"
                );
                checked += 1;
            }
        }
        // In no tenant: nothing, with the answer a companion waits on.
        assert_eq!(
            h.snapshot(&zed, ACME[0]).await,
            (403, "team_record_stale".into())
        );
        // Removed from the tenant: its sources are gone for that person's pairing at once.
        let mia = h.paired("mia").await;
        assert_eq!(h.snapshot(&mia, ACME[2]).await.0, 200);
        let mut tx = h.app.db.begin().await.unwrap();
        tenants::remove_member(&mut tx, "acme", 3)
            .await
            .unwrap()
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(h.snapshot(&mia, ACME[2]).await.0, 403);
        println!("checked {} snapshot reads on 8 sources", checked + 3);
    }

    #[tokio::test]
    async fn the_overview_shows_only_the_tenants_own_sources() {
        let h = with_graph().await;
        h.with_main().await;
        for key in ACME.iter().chain(&BCORP) {
            built(&h, key).await;
        }
        for (login, tenant, own) in [("ada", "acme", ACME), ("bob", "bcorp", BCORP)] {
            let (status, body) = h
                .call(login, GET, &scoped(tenant, "main", "/api/overview"), None)
                .await;
            assert_eq!(status, 200, "{body}");
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            let mut shown: Vec<&str> = v["graph"]["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["source"].as_str().unwrap())
                .collect();
            shown.sort();
            let mut want = own.to_vec();
            want.sort();
            assert_eq!(shown, want, "{tenant}");
        }
    }
}

// ---- Members and the operator (ADR 0087, 0088; design §3.3, §4.5, §4.6, §5.4) ----

mod people_and_operator {
    use super::*;
    use serde_json::{Value, json};

    const MEMBERS: &str = "/api/t/acme/members";

    fn parsed(body: &str) -> Value {
        serde_json::from_str(body).unwrap_or(Value::Null)
    }

    async fn count(h: &H, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&h.app.db).await.unwrap()
    }

    #[tokio::test]
    async fn an_admin_invites_by_login_and_the_invite_binds_to_the_github_id() {
        let h = two_tenants().await;
        // Every member reads the list; only an admin writes.
        let (status, body) = h.call("mia", GET, MEMBERS, None).await;
        assert_eq!(status, 200, "{body}");
        let logins: Vec<String> = parsed(&body)
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["login"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(logins, ["ada", "mia"], "admins first; nobody of bcorp");
        for (method, path, body) in [
            (POST, MEMBERS.to_owned(), Some(json!({"login": "carol"}))),
            (PUT, format!("{MEMBERS}/1"), Some(json!({"role": "member"}))),
            (DELETE, format!("{MEMBERS}/1"), None),
        ] {
            let (status, text) = h.call("mia", method, &path, body).await;
            assert_eq!((status, text.as_str()), (403, r#"{"reason":"not_admin"}"#));
        }
        // An invite by login: the account GitHub names, by its id and its own spelling.
        let (status, body) = h
            .call("ada", POST, MEMBERS, Some(json!({"login": "@carol"})))
            .await;
        assert_eq!(status, 201, "{body}");
        let m = parsed(&body);
        assert_eq!(
            (
                m["github_id"].as_i64(),
                m["login"].as_str(),
                m["role"].as_str()
            ),
            (Some(5), Some("Carol"), Some("member"))
        );
        assert_eq!(m["joined"], false, "invited, never logged in");
        assert_eq!(m["invited_by"], "ada");
        let again = h
            .call("ada", POST, MEMBERS, Some(json!({"login": "CAROL"})))
            .await;
        assert_eq!(again.0, 409, "{}", again.1);
        assert_eq!(parsed(&again.1)["reason"], "already_member");
        let nobody = h
            .call(
                "ada",
                POST,
                MEMBERS,
                Some(json!({"login": "nobody-by-this-name"})),
            )
            .await;
        assert_eq!(nobody.0, 422);
        assert_eq!(parsed(&nobody.1)["reason"], "unknown_login");
        // A person may be in several tenants: bcorp's admin is invited to acme as well.
        let (status, _) = h
            .call(
                "ada",
                POST,
                MEMBERS,
                Some(json!({"login": "bob", "role": "admin"})),
            )
            .await;
        assert_eq!(status, 201);
        let (_, me) = h.call("bob", GET, "/api/me", None).await;
        let mut of_bob: Vec<String> = parsed(&me)["tenants"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                format!(
                    "{}:{}",
                    t["id"].as_str().unwrap(),
                    t["role"].as_str().unwrap()
                )
            })
            .collect();
        of_bob.sort();
        assert_eq!(of_bob, ["acme:admin", "bcorp:admin"]);
        // Nothing of this reached bcorp.
        assert_eq!(
            count(
                &h,
                "SELECT COUNT(*) FROM tenant_members WHERE tenant_id = 'bcorp'"
            )
            .await,
            1
        );
    }

    #[tokio::test]
    async fn the_last_admin_stays_and_roles_change_only_inside_the_tenant() {
        let h = two_tenants().await;
        for (method, body) in [(PUT, Some(json!({"role": "member"}))), (DELETE, None)] {
            let (status, text) = h.call("ada", method, &format!("{MEMBERS}/1"), body).await;
            assert_eq!(status, 409, "{text}");
            assert_eq!(parsed(&text)["reason"], "last_admin");
        }
        // With a second admin the first may step down, and then no longer writes.
        let (status, _) = h
            .call(
                "ada",
                PUT,
                &format!("{MEMBERS}/3"),
                Some(json!({"role": "admin"})),
            )
            .await;
        assert_eq!(status, 204);
        let (status, _) = h
            .call(
                "ada",
                PUT,
                &format!("{MEMBERS}/1"),
                Some(json!({"role": "member"})),
            )
            .await;
        assert_eq!(status, 204);
        let (status, _) = h
            .call(
                "ada",
                PUT,
                &format!("{MEMBERS}/1"),
                Some(json!({"role": "admin"})),
            )
            .await;
        assert_eq!(status, 403, "a member now");
        // A member of another tenant is not one here, whatever the id.
        let (status, text) = h
            .call(
                "mia",
                PUT,
                &format!("{MEMBERS}/2"),
                Some(json!({"role": "admin"})),
            )
            .await;
        assert_eq!(status, 404, "{text}");
        assert_eq!(parsed(&text)["reason"], "not_a_member");
    }

    #[tokio::test]
    async fn removing_a_person_ends_what_they_had_in_the_tenant_and_everything_when_it_was_their_last()
     {
        let h = two_tenants().await;
        h.with_main().await;
        // mia has a session, a pairing, something queued and an agent in a room of acme.
        let mias_cookie = h.cookie("mia").await;
        let now = chrono::Utc::now().to_rfc3339();
        let companion = "00000000-0000-0000-0000-0000000000c3";
        for sql in [
            format!(
                "INSERT INTO pairings (companion_id, user, token_hash, hostname, created_at) VALUES ('{companion}', 'mia', 'hm', 'mia-laptop', '{now}')"
            ),
            format!(
                "INSERT INTO companion_queue (tenant_id, companion_id, repo, number, position, queued_by, queued_at) VALUES ('acme', '{companion}', 'acme/api', 7, 0, 'mia', '{now}')"
            ),
            format!(
                "INSERT INTO room_requests (tenant_id, id, repo, number, branch, owner, launch_id, requested_by, at, state) VALUES ('acme', '00000000-0000-0000-0000-0000000000d1', 'acme/api', 7, 'issue/7-x', 'ada', '{B_LAUNCH}', 'mia', '{now}', 'pending')"
            ),
        ] {
            sqlx::query(&sql)
                .execute(&h.app.db)
                .await
                .unwrap_or_else(|e| panic!("{e}: {sql}"));
        }
        let mut socket = h.socket("mia", Some("acme")).await.unwrap();
        let reach = |cookie: String| {
            let (http, url) = (h.http.clone(), h.url.clone());
            async move {
                http.get(format!("{url}/api/me"))
                    .header("cookie", cookie)
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .as_u16()
            }
        };
        assert_eq!(reach(mias_cookie.clone()).await, 200);

        let (status, text) = h.call("ada", DELETE, &format!("{MEMBERS}/3"), None).await;
        assert_eq!(status, 204, "{text}");

        // Her socket for the tenant is closed at once, by the event that says members changed.
        use futures_util::StreamExt;
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match socket.next().await {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(Some(f)))) => {
                        return Some(u16::from(f.code));
                    }
                    Some(Ok(_)) => {}
                    _ => return None,
                }
            }
        })
        .await
        .expect("closed without waiting for the timer");
        assert_eq!(closed, Some(4403));
        // It was her last membership: her session, her pairing and her queue are gone.
        assert_eq!(reach(mias_cookie).await, 401);
        assert_eq!(
            count(
                &h,
                "SELECT COUNT(*) FROM pairings WHERE user = 'mia' AND revoked_at IS NULL"
            )
            .await,
            0
        );
        assert_eq!(
            count(
                &h,
                "SELECT COUNT(*) FROM companion_queue WHERE tenant_id = 'acme'"
            )
            .await,
            0
        );
        assert_eq!(
            count(
                &h,
                "SELECT COUNT(*) FROM room_requests WHERE tenant_id = 'acme' AND state = 'pending'"
            )
            .await,
            0
        );
        // Removed twice is not a member; and a login with no invite gets no session again.
        let (status, _) = h.call("ada", DELETE, &format!("{MEMBERS}/3"), None).await;
        assert_eq!(status, 404);

        // bob is in two tenants: leaving one ends nothing of the other.
        let (status, _) = h
            .call("ada", POST, MEMBERS, Some(json!({"login": "bob"})))
            .await;
        assert_eq!(status, 201);
        let bobs_cookie = h.cookie("bob").await;
        let (status, _) = h.call("ada", DELETE, &format!("{MEMBERS}/2"), None).await;
        assert_eq!(status, 204);
        assert_eq!(
            reach(bobs_cookie).await,
            200,
            "still in bcorp: the session stays"
        );
        let (status, _) = h
            .call("bob", GET, &scoped("bcorp", "main", "/api/overview"), None)
            .await;
        assert_eq!(status, 200);
        let (status, _) = h
            .call("bob", GET, &scoped("acme", "main", "/api/overview"), None)
            .await;
        assert_eq!(status, 404);
    }

    #[tokio::test]
    async fn the_operator_makes_tenants_and_sees_names_and_counts_but_no_data() {
        let h = two_tenants().await;
        h.with_main().await;
        // Not an operator: the routes do not exist, admin of a tenant or not.
        for (method, path) in [
            (GET, "/api/admin/installations"),
            (GET, "/api/admin/tenants"),
            (POST, "/api/admin/tenants"),
            (DELETE, "/api/admin/tenants/bcorp"),
            (POST, "/api/admin/tenants/acme/installations/33"),
            (DELETE, "/api/admin/tenants/bcorp/installations/22"),
            (POST, "/api/admin/installations/22/purge?confirm=true"),
            (POST, "/api/admin/tenants/bcorp/admins"),
        ] {
            let body = (method != GET && method != DELETE)
                .then(|| json!({"id": "x", "name": "x", "first_admin": "ada", "login": "ada"}));
            let (status, text) = h.call("ada", method.clone(), path, body).await;
            assert_eq!(status, 404, "{method} {path}: {text}");
            assert_eq!(parsed(&text)["reason"], "not_operator", "{method} {path}");
        }
        // The operator is in no tenant: every tenant's data answers as if there were none.
        let (status, _) = h
            .call("zed", GET, &scoped("acme", "main", "/api/overview"), None)
            .await;
        assert_eq!(status, 404);

        let (status, body) = h.call("zed", GET, "/api/admin/installations", None).await;
        assert_eq!(status, 200, "{body}");
        let seen: Vec<(u64, String, String, Option<String>)> = parsed(&body)
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i["installation_id"].as_u64().unwrap(),
                    i["account_login"].as_str().unwrap().to_owned(),
                    i["state"].as_str().unwrap().to_owned(),
                    i["tenant"].as_str().map(str::to_owned),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (
                    11,
                    "acme".to_owned(),
                    "attached".to_owned(),
                    Some("acme".to_owned())
                ),
                (
                    22,
                    "bcorp".to_owned(),
                    "attached".to_owned(),
                    Some("bcorp".to_owned())
                ),
                (33, "newco".to_owned(), "unattached".to_owned(), None),
            ]
        );
        let (status, body) = h.call("zed", GET, "/api/admin/tenants", None).await;
        assert_eq!(status, 200);
        assert_eq!(
            parsed(&body),
            json!([
                {"id": "acme", "name": "acme", "installations": [11], "members": 2, "admins": 1, "workspaces": 1},
                {"id": "bcorp", "name": "bcorp", "installations": [22], "members": 1, "admins": 1, "workspaces": 1},
            ])
        );

        // A tenant, its installation and its first admin, in one step.
        let new =
            json!({"id": "newco", "name": "Newco", "first_admin": "carol", "installations": [33]});
        let (status, body) = h
            .call("zed", POST, "/api/admin/tenants", Some(new.clone()))
            .await;
        assert_eq!(status, 201, "{body}");
        let (status, body) = h.call("zed", POST, "/api/admin/tenants", Some(new)).await;
        assert_eq!(
            (status, parsed(&body)["reason"].as_str()),
            (409, Some("duplicate"))
        );
        for (bad, why) in [
            (
                json!({"id": "Not An Id", "name": "x", "first_admin": "ada"}),
                "bad_id",
            ),
            (
                json!({"id": "other", "name": "x", "first_admin": "nobody-by-this-name"}),
                "unknown_login",
            ),
            (
                json!({"id": "other", "name": "x", "first_admin": "ada", "installations": [99]}),
                "unknown_installation",
            ),
            (
                json!({"id": "other", "name": "x", "first_admin": "ada", "installations": [22]}),
                "installation_taken",
            ),
        ] {
            let (_, body) = h.call("zed", POST, "/api/admin/tenants", Some(bad)).await;
            assert_eq!(parsed(&body)["reason"], why, "{body}");
        }
        assert_eq!(
            count(&h, "SELECT COUNT(*) FROM tenants WHERE id = 'other'").await,
            0,
            "all or nothing"
        );
        // Its repositories are its own now, and its first admin is the account GitHub named.
        assert_eq!(
            h.app.github.tenant_of("newco/app").map(|t| t.to_string()),
            Some("newco".to_owned())
        );
        assert_eq!(
            count(&h, "SELECT COUNT(*) FROM tenant_members WHERE tenant_id = 'newco' AND github_id = 5 AND role = 'admin'").await,
            1
        );
        // An installation belongs to one tenant: the answer names the one that holds it.
        let (status, body) = h
            .call(
                "zed",
                POST,
                "/api/admin/tenants/acme/installations/33",
                None,
            )
            .await;
        assert_eq!(status, 409, "{body}");
        assert_eq!(
            parsed(&body),
            json!({"reason": "installation_taken", "detail": "newco"})
        );
        // The operator can always name an admin: a member is promoted, a stranger invited.
        let (status, body) = h
            .call(
                "zed",
                POST,
                "/api/admin/tenants/bcorp/admins",
                Some(json!({"login": "mia"})),
            )
            .await;
        assert_eq!(status, 204, "{body}");
        let (status, _) = h
            .call(
                "zed",
                POST,
                "/api/admin/tenants/acme/admins",
                Some(json!({"login": "mia"})),
            )
            .await;
        assert_eq!(status, 204);
        assert_eq!(
            count(
                &h,
                "SELECT COUNT(*) FROM tenant_members WHERE github_id = 3 AND role = 'admin'"
            )
            .await,
            2
        );
    }

    #[tokio::test]
    async fn detach_keeps_the_rows_hidden_and_purge_names_them_before_it_deletes_them() {
        let h = two_tenants().await;
        h.with_main().await;
        seed_bcorp(&h).await;
        let purge = "/api/admin/installations/22/purge";
        let detach = "/api/admin/tenants/bcorp/installations/22";
        // Attached: nothing to purge. In use by a workspace: not detached.
        let (status, body) = h.call("zed", POST, purge, None).await;
        assert_eq!(
            (status, parsed(&body)["reason"].as_str()),
            (409, Some("not_detached"))
        );
        let (status, body) = h.call("zed", DELETE, detach, None).await;
        assert_eq!(status, 409, "{body}");
        assert_eq!(
            parsed(&body),
            json!({"reason": "installation_in_use", "detail": "workspace main"})
        );
        // A tenant with a workspace or an installation is not deleted.
        let (status, body) = h
            .call("zed", DELETE, "/api/admin/tenants/bcorp", None)
            .await;
        assert_eq!(
            (status, parsed(&body)["reason"].as_str()),
            (409, Some("tenant_not_empty"))
        );
        // Its admin deletes the workspace; then the operator detaches.
        let (status, body) = h.call("bob", DELETE, "/api/t/bcorp/w/main", None).await;
        assert!(status == 204 || status == 200, "{status} {body}");
        let (status, body) = h.call("zed", DELETE, detach, None).await;
        assert_eq!(status, 204, "{body}");
        assert_eq!(h.app.github.tenant_of("bcorp/web"), None, "no longer read");
        let kept = count(
            &h,
            "SELECT COUNT(*) FROM room_messages WHERE tenant_id = 'bcorp'",
        )
        .await;
        assert_eq!(kept, 1, "kept, hidden, until purged");
        assert_eq!(
            count(
                &h,
                "SELECT COUNT(*) FROM companion_queue WHERE tenant_id = 'bcorp'"
            )
            .await,
            0
        );
        // Another tenant cannot take the installation while bcorp's rows of it remain.
        let (status, body) = h
            .call(
                "zed",
                POST,
                "/api/admin/tenants/acme/installations/22",
                None,
            )
            .await;
        assert_eq!(status, 409, "{body}");
        assert_eq!(
            parsed(&body),
            json!({"reason": "rows_of_other_tenant", "detail": "bcorp"})
        );
        // The purge names what it would delete, and deletes nothing until confirmed.
        let (status, body) = h.call("zed", POST, purge, None).await;
        assert_eq!(status, 200, "{body}");
        let report = parsed(&body);
        assert_eq!(report["purged"], false);
        assert_eq!(report["tenant"], "bcorp");
        assert_eq!(report["repos"], json!(["bcorp/map", "bcorp/web"]));
        let rows = report["rows"].as_object().unwrap().clone();
        for table in [
            "launches",
            "plans",
            "plan_drafts",
            "plan_draft_revisions",
            "room_messages",
            "room_requests",
            "session_messages",
            "session_log_entries",
            "issue_meta",
            "open_issues",
            "pins",
        ] {
            assert!(
                rows.get(table).and_then(Value::as_u64) >= Some(1),
                "{table}: {rows:?}"
            );
        }
        assert_eq!(
            count(
                &h,
                "SELECT COUNT(*) FROM room_messages WHERE tenant_id = 'bcorp'"
            )
            .await,
            1
        );
        let acme_before = count(&h, "SELECT COUNT(*) FROM pins WHERE tenant_id = 'acme'").await
            + count(
                &h,
                "SELECT COUNT(*) FROM tenant_repos WHERE tenant_id = 'acme'",
            )
            .await;
        let (status, body) = h
            .call("zed", POST, &format!("{purge}?confirm=true"), None)
            .await;
        assert_eq!(status, 200, "{body}");
        let done = parsed(&body);
        assert_eq!(done["purged"], true);
        assert_eq!(
            done["rows"], report["rows"],
            "what it named is what it deleted"
        );
        // Nothing of bcorp's repositories is left in any table, and acme is as it was.
        let mut conn = h.app.db.acquire().await.unwrap();
        let left = rubato_server::kernel::purge::rows_of_repos(
            &mut conn,
            "bcorp",
            &["bcorp/map".into(), "bcorp/web".into()],
            false,
        )
        .await
        .unwrap();
        assert!(left.is_empty(), "{left:?}");
        drop(conn);
        assert_eq!(
            count(
                &h,
                "SELECT COUNT(*) FROM tenant_installations WHERE installation_id = 22"
            )
            .await,
            0
        );
        assert_eq!(
            count(&h, "SELECT COUNT(*) FROM pins WHERE tenant_id = 'acme'").await
                + count(
                    &h,
                    "SELECT COUNT(*) FROM tenant_repos WHERE tenant_id = 'acme'"
                )
                .await,
            acme_before
        );
        let (status, _) = h.call("zed", POST, purge, None).await;
        assert_eq!(status, 404, "purged: no such installation here");
        // Now the installation is free, and the empty tenant can go; its admin with it.
        let bobs = h.cookie("bob").await;
        let (status, body) = h
            .call("zed", DELETE, "/api/admin/tenants/bcorp", None)
            .await;
        assert_eq!(status, 204, "{body}");
        let r = h
            .http
            .get(format!("{}/api/me", h.url))
            .header("cookie", bobs)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401, "bcorp was bob's only tenant");
        let (status, body) = h
            .call(
                "zed",
                POST,
                "/api/admin/tenants/acme/installations/22",
                None,
            )
            .await;
        assert_eq!(status, 204, "{body}");
        assert_eq!(
            h.app.github.tenant_of("bcorp/web").map(|t| t.to_string()),
            Some("acme".to_owned())
        );
    }
}
