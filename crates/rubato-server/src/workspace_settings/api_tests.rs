//! Plan 189 PR 3: the settings routes, over HTTP. In the crate, not under `tests/`, because they
//! read the in-memory metrics and captured log lines (plan 189 Decision 13). A slice does not
//! name the composition root, so the routes are mounted here as `app.rs` mounts them, with the
//! workspace layer; `app::tests` checks the real router has them and refuses cross-origin writes.

use super::start;
use crate::auth::{Granted, keep_token};
use crate::kernel::config::{Config, GitHubApp, TokenKey};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState};
use crate::kernel::telemetry::{capture, metric_reader};
use crate::kernel::workspaces::WorkspaceEntry;
use crate::map;
use axum::routing::{delete, get};
use reqwest::Method;
use rubato_protocol::otel::{attr, metric, span};
use rubato_protocol::workspace_settings::reason;
use rubato_protocol::{
    SecretUpdate, SettingsRefusal, SourceFields, TrafficSourceUpdate, WebEvent, WorkspaceInfo,
    WorkspaceSettings, WorkspaceSettingsUpdate,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

const USER: &str = "m0x6A";
const RUBATO_MAP: &str = include_str!("../../../../docs/design/rubato-map.yml");
const HEADER: &str = "Authorization=Bearer s3cret-token";

/// A small map in `repo`, naming `code` as the repo behind its parts.
fn small_map(name: &str, code: &str) -> String {
    format!(
        "version: 1\nname: {name}\nrepos: [{{ name: {code} }}]\nnodes:\n  - {{ id: storefront, kind: context, repo: {code} }}\n  - {{ id: backend, kind: context, repo: {code} }}\nseams:\n  - {{ id: storefront-backend, from: storefront, to: backend, owner: backend }}\n"
    )
}

struct H {
    base: String,
    app: App,
    github: Arc<FakeGitHub>,
    reader: metric_reader::Reader,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

fn entry(id: &str, repo: &str, path: &str) -> WorkspaceEntry {
    WorkspaceEntry {
        tenant: crate::kernel::tenancy::TenantId::default_tenant(),
        id: id.into(),
        name: None,
        map_repo: repo.into(),
        map_path: path.into(),
        map_ref: "main".into(),
        map_file: None,
        settings: Default::default(),
    }
}

/// A server that is not local (public URL `http://test`), with the GitHub App and a token key,
/// seeded with `default`, `shop` and the file workspace `filed`; `configure` changes that. The
/// member has a stored GitHub token.
async fn harness(configure: impl FnOnce(&mut Config)) -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = crate::kernel::db::connect(&url).await.unwrap();
    let filed = tmp.path().join("filed.yml");
    std::fs::write(&filed, small_map("filed", "acme/filed")).unwrap();
    let mut cfg = Config::for_tests(url, USER);
    cfg.github_app = Some(GitHubApp {
        app_id: 1,
        private_key: "unused".into(),
        installation_id: 34,
    });
    cfg.token_key = Some(TokenKey::from_hex(&"07".repeat(32)).unwrap());
    cfg.workspaces = vec![
        entry("default", "m0x6A/rubato", "docs/design/rubato-map.yml"),
        entry("shop", "acme/shop-map", "system-map.yml"),
        WorkspaceEntry {
            map_file: Some(filed),
            ..entry("filed", "acme/filed-map", "system-map.yml")
        },
    ];
    configure(&mut cfg);
    let with_tokens = cfg.token_key.is_some() && cfg.github_app.is_some();
    let github = Arc::new(FakeGitHub::new());
    github.with(|g| {
        let mut serve = |repo: &str, path: &str, branch: &str, sha: &str, yaml: String| {
            g.files
                .insert((repo.into(), path.into(), branch.into()), yaml);
            g.branches
                .entry(repo.into())
                .or_default()
                .push((branch.into(), sha.into()));
        };
        serve(
            "m0x6A/rubato",
            "docs/design/rubato-map.yml",
            "main",
            "sha-rubato",
            RUBATO_MAP.into(),
        );
        serve(
            "acme/shop-map",
            "system-map.yml",
            "main",
            "sha-shop",
            small_map("shop", "acme/shop"),
        );
        serve(
            "acme/shop-map",
            "system-map.yml",
            "next",
            "sha-shop-next",
            small_map("shop next", "acme/shop"),
        );
        serve(
            "acme/blog-map",
            "system-map.yml",
            "main",
            "sha-blog",
            small_map("blog", "acme/blog"),
        );
        serve(
            "secret/map",
            "system-map.yml",
            "main",
            "sha-secret",
            small_map("secret", "acme/shop"),
        );
        serve(
            "acme/leaky-map",
            "system-map.yml",
            "main",
            "sha-leaky",
            small_map("leaky", "secret/code"),
        );
        g.unreadable.insert((USER.into(), "secret/map".into()));
        g.unreadable.insert((USER.into(), "secret/code".into()));
    });
    let (reader, metrics) = metric_reader::start();
    // ADR 0087: tenant `default` holds installation 34, which covers every repository here but
    // the two under `secret/`: those are another tenant's (installation 99).
    let client = github.clone();
    let apps = crate::kernel::github::GitHubApps::installations(
        github.clone(),
        Box::new(move |_| Ok(client.clone() as crate::kernel::github::Client)),
    );
    let tenant = |t: &str| crate::kernel::github::Attached {
        tenant: crate::kernel::tenancy::TenantId::parse(t).unwrap(),
        suspended: false,
    };
    apps.set_routes(crate::kernel::github::Routes {
        installations: [(34, tenant("default")), (99, tenant("other"))].into(),
        repos: [
            ("m0x6a/rubato", 34),
            ("acme/shop-map", 34),
            ("acme/shop", 34),
            ("acme/blog-map", 34),
            ("acme/blog", 34),
            ("acme/leaky-map", 34),
            ("acme/filed-map", 34),
            ("acme/filed", 34),
            ("secret/map", 99),
            ("secret/code", 99),
        ]
        .map(|(r, i)| (r.to_owned(), i))
        .into(),
    });
    let mut app = AppState::with_github(pool, cfg, apps);
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    start(&app).await.unwrap();
    map::reload(&app).await.unwrap();
    if with_tokens {
        let kept = keep_token(
            &app,
            USER,
            &Granted {
                access: format!("tok-{USER}"),
                expires_in: Some(chrono::Duration::hours(8)),
                refresh: Some("r".into()),
                refresh_expires_in: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(kept, None);
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let scoped = axum::Router::new()
        .route(
            "/api/w/{ws}/settings",
            get(super::settings).put(super::update),
        )
        .route("/api/w/{ws}", delete(super::delete))
        .route_layer(axum::middleware::from_fn_with_state(
            app.clone(),
            crate::kernel::http::check_workspace,
        ));
    let router = axum::Router::new()
        .merge(scoped)
        .route("/api/workspaces", get(map::workspaces).post(super::create))
        .route("/api/traffic/sources", get(super::sources))
        .layer(axum::middleware::from_fn(
            crate::kernel::tenancy::test_scope,
        ))
        .with_state(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    H {
        base,
        app,
        github,
        reader,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

fn settings_path(ws: &str) -> String {
    format!("/api/w/{ws}/settings")
}

impl H {
    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (u16, String) {
        let mut r = self.http.request(method, format!("{}{path}", self.base));
        if let Some(b) = body {
            r = r.json(&b);
        }
        let r = r.send().await.unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    async fn get(&self, ws: &str) -> WorkspaceSettings {
        let (status, body) = self.send(Method::GET, &settings_path(ws), None).await;
        assert_eq!(status, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    /// The update that changes nothing: what the view sends when only one field is edited.
    async fn unchanged(&self, ws: &str) -> WorkspaceSettingsUpdate {
        let s = self.get(ws).await;
        WorkspaceSettingsUpdate {
            revision: s.revision,
            name: s.name,
            map_repo: s.map_repo,
            map_path: s.map_path,
            map_ref: s.map_ref,
            traffic: s.traffic.map(|t| TrafficSourceUpdate {
                source: t.source,
                fields: t.fields,
                secrets: BTreeMap::new(),
            }),
            default_spend_cap_usd: s.default_spend_cap_usd,
            default_attempt_limit: s.default_attempt_limit,
        }
    }

    async fn put(&self, ws: &str, u: &WorkspaceSettingsUpdate) -> (u16, String) {
        self.send(
            Method::PUT,
            &settings_path(ws),
            Some(serde_json::to_value(u).unwrap()),
        )
        .await
    }

    async fn put_ok(&self, ws: &str, u: &WorkspaceSettingsUpdate) -> WorkspaceSettings {
        let (status, body) = self.put(ws, u).await;
        assert_eq!(status, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    /// Every stored row, as JSON text: a refusal must leave it as it was.
    async fn rows(&self) -> Vec<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT json_array(workspace_id, revision, name, map_repo, map_path, map_ref, traffic_source, traffic_fields, traffic_secrets_sealed, default_spend_cap_usd, default_attempt_limit, updated_by, updated_at) FROM workspace_settings ORDER BY position",
        )
        .fetch_all(&self.app.db)
        .await
        .unwrap()
    }

    fn rejected(&self, reason: &str) -> u64 {
        self.reader
            .points(metric::WEB_REJECTED)
            .into_iter()
            .filter(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(reason))
            .map(|p| p.value)
            .sum()
    }

    fn gauge(&self, name: &str) -> Option<u64> {
        self.reader.points(name).first().map(|p| p.value)
    }
}

fn traffic(url: &str, headers: Option<SecretUpdate>) -> TrafficSourceUpdate {
    TrafficSourceUpdate {
        source: "prometheus".into(),
        fields: [("url".to_owned(), url.to_owned())].into(),
        secrets: headers
            .map(|h| ("headers".to_owned(), h))
            .into_iter()
            .collect(),
    }
}

fn set(value: &str) -> SecretUpdate {
    SecretUpdate::Set {
        value: value.into(),
    }
}

/// Which server a case of the refusal table runs against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Server {
    Plain,
    NoTokenKey,
    DevTraffic,
    OpenLogin,
}

/// One refused change: the request and the reason it must answer.
struct Case {
    name: &'static str,
    server: Server,
    method: Method,
    path: &'static str,
    body: Body,
    reason: &'static str,
}

enum Body {
    None,
    Json(serde_json::Value),
    /// A `PUT`: the workspace's unchanged update with this change made to it.
    Edit(fn(&mut WorkspaceSettingsUpdate)),
}

const SHOP: &str = "/api/w/shop/settings";
const METADATA: &str = "169.254.169.254";

fn new_ws(id: &str, repo: &str) -> Body {
    Body::Json(json!({ "id": id, "map_repo": repo, "map_path": "system-map.yml" }))
}

fn put(
    name: &'static str,
    server: Server,
    path: &'static str,
    edit: fn(&mut WorkspaceSettingsUpdate),
    reason: &'static str,
) -> Case {
    Case {
        name,
        server,
        method: Method::PUT,
        path,
        body: Body::Edit(edit),
        reason,
    }
}

fn post(name: &'static str, body: Body, reason: &'static str) -> Case {
    Case {
        name,
        server: Server::Plain,
        method: Method::POST,
        path: "/api/workspaces",
        body,
        reason,
    }
}

fn url_case(name: &'static str, edit: fn(&mut WorkspaceSettingsUpdate)) -> Case {
    put(name, Server::Plain, SHOP, edit, reason::URL_REFUSED)
}

fn table() -> Vec<Case> {
    use Server::{DevTraffic, NoTokenKey, OpenLogin, Plain};
    vec![
        post("a bad id", new_ws("Shop!", "acme/blog-map"), reason::BAD_ID),
        post(
            "a duplicate id",
            new_ws("shop", "acme/blog-map"),
            reason::DUPLICATE,
        ),
        post(
            "the same map twice",
            new_ws("shop-2", "Acme/Shop-Map"),
            reason::SAME_MAP,
        ),
        Case {
            name: "deleting default",
            server: Plain,
            method: Method::DELETE,
            path: "/api/w/default",
            body: Body::None,
            reason: reason::DEFAULT_KEPT,
        },
        put(
            "a changed map_repo",
            Plain,
            SHOP,
            |u| u.map_repo = "acme/blog-map".into(),
            reason::MAP_SOURCE_FIXED,
        ),
        put(
            "a changed map_path",
            Plain,
            SHOP,
            |u| u.map_path = "other.yml".into(),
            reason::MAP_SOURCE_FIXED,
        ),
        put(
            "a ref move on a map_file workspace",
            Plain,
            "/api/w/filed/settings",
            |u| u.map_ref = "next".into(),
            reason::MAP_FILE,
        ),
        // `default`'s draft holds an operation (set up by `server`).
        put(
            "a ref move while the draft has an op",
            Plain,
            "/api/w/default/settings",
            |u| u.map_ref = "next".into(),
            reason::DRAFT_OPEN,
        ),
        put(
            "a ref that does not exist, with a cached map present",
            Plain,
            SHOP,
            |u| u.map_ref = "no-such-branch".into(),
            reason::MAP_INVALID,
        ),
        post(
            "a map repo of another tenant",
            new_ws("hidden", "secret/map"),
            reason::REPO_UNREADABLE,
        ),
        post(
            "a map that names a repository of another tenant",
            new_ws("leaky", "acme/leaky-map"),
            reason::REPO_UNREADABLE,
        ),
        url_case("ftp://", |u| {
            u.traffic = Some(traffic("ftp://prom.lan/", None))
        }),
        url_case("userinfo", |u| {
            u.traffic = Some(traffic("http://user:pw@prom.lan:9090", None))
        }),
        url_case("127.0.0.1", |u| {
            u.traffic = Some(traffic("http://127.0.0.1:9090", None))
        }),
        url_case("[::ffff:127.0.0.1]", |u| {
            u.traffic = Some(traffic("http://[::ffff:127.0.0.1]:9090", None))
        }),
        url_case(METADATA, |u| {
            u.traffic = Some(traffic("http://169.254.169.254/latest", None))
        }),
        url_case("fd00:ec2::254", |u| {
            u.traffic = Some(traffic("http://[fd00:ec2::254]/", None))
        }),
        url_case("100.100.100.200", |u| {
            u.traffic = Some(traffic("http://100.100.100.200/", None))
        }),
        put(
            "a secret with no token key",
            NoTokenKey,
            SHOP,
            |u| u.traffic = Some(traffic("http://prom.lan:9090", Some(set(HEADER)))),
            reason::NO_TOKEN_KEY,
        ),
        put(
            "a source while RUBATO_DEV_TRAFFIC is set",
            DevTraffic,
            SHOP,
            |u| u.traffic = Some(traffic("http://prom.lan:9090", None)),
            reason::DEV_TRAFFIC_SET,
        ),
        // A deleted workspace `gone` left a row (set up by `server`).
        post(
            "an id with kept rows",
            new_ws("gone", "acme/blog-map"),
            reason::ID_HAS_ROWS,
        ),
        put(
            "any write while the server is open login",
            OpenLogin,
            SHOP,
            |u| u.name = Some("Shop".into()),
            reason::OPEN_LOGIN,
        ),
    ]
}

async fn server(kind: Server, local: bool) -> H {
    let h = harness(|cfg| {
        if local {
            cfg.public_url = "http://localhost:4000".into();
        }
        match kind {
            Server::Plain => {}
            Server::NoTokenKey => cfg.token_key = None,
            Server::DevTraffic => cfg.dev_traffic = Some("/tmp/traffic.json".into()),
            Server::OpenLogin => {
                cfg.github_app = None;
                cfg.github_client_id = Some("id".into());
                cfg.github_client_secret = Some("secret".into());
            }
        }
    })
    .await;
    sqlx::query("INSERT INTO map_draft_ops (tenant_id, op_id, op, author, created_at, workspace_id) VALUES ('default', 'op-1', '{}', 'm0x6A', '2026-11-16T00:00:00Z', 'default')")
        .execute(&h.app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO map_cache (tenant_id, workspace_id, version, yaml, loaded_at) VALUES ('default', 'gone', 'v', 'x', '2026-11-16T00:00:00Z')")
        .execute(&h.app.db)
        .await
        .unwrap();
    h
}

/// Run one case: `Ok` when the change was refused with its reason, counted, and the stored rows
/// are as they were; `Err` says what was wrong.
async fn run(h: &H, case: &Case) -> Result<(), String> {
    let before = h.rows().await;
    let counted = h.rejected(case.reason);
    let body = match &case.body {
        Body::None => None,
        Body::Json(v) => Some(v.clone()),
        Body::Edit(edit) => {
            let ws = case.path.split('/').nth(3).unwrap();
            let mut u = h.unchanged(ws).await;
            edit(&mut u);
            Some(serde_json::to_value(&u).unwrap())
        }
    };
    let (status, text) = h.send(case.method.clone(), case.path, body).await;
    if !(400..500).contains(&status) {
        return Err(format!("{}: answered {status} {text}", case.name));
    }
    let refusal: SettingsRefusal =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}: {text}", case.name))?;
    if refusal.reason != case.reason {
        return Err(format!(
            "{}: reason {} ({}), expected {}",
            case.name, refusal.reason, refusal.message, case.reason
        ));
    }
    if refusal.message.is_empty() || refusal.message.contains("s3cret") {
        return Err(format!("{}: message {:?}", case.name, refusal.message));
    }
    if h.rejected(case.reason) != counted + 1 {
        return Err(format!("{}: not counted on rubato.web.rejected", case.name));
    }
    if h.rows().await != before {
        return Err(format!("{}: a refused change was stored", case.name));
    }
    Ok(())
}

#[tokio::test]
async fn a_refused_change_is_refused() {
    let cases = table();
    let mut servers: BTreeMap<Server, H> = BTreeMap::new();
    for case in &cases {
        let h = match servers.entry(case.server) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(server(case.server, false).await)
            }
        };
        if let Err(e) = run(h, case).await {
            panic!("{e}");
        }
    }
    assert_eq!(cases.len(), 22);
    // Every reason a refusal can carry is in the table, but a stale edit and a value that does
    // not read, which `a_stale_edit_answers_409` covers.
    let covered: std::collections::BTreeSet<&str> = cases.iter().map(|c| c.reason).collect();
    for r in reason::ALL {
        assert!(
            covered.contains(r) || r == reason::CHANGED_MEANWHILE || r == reason::BAD_VALUE,
            "no case for {r}"
        );
    }
    println!(
        "checked {} refused changes on {} servers",
        cases.len(),
        servers.len()
    );
}

/// The table's must-fail twin: with the address guard off (a server that is itself local, ADR
/// 0064) the metadata URL is accepted, so its case fails. A URL refused for what it is, not
/// where it points, is still refused.
#[tokio::test]
async fn the_refusal_table_fails_with_the_address_guard_off() {
    let h = server(Server::Plain, true).await;
    let cases = table();
    let case = |name: &str| cases.iter().find(|c| c.name == name).unwrap();
    let err = run(&h, case(METADATA)).await.unwrap_err();
    assert!(err.contains("answered 200"), "{err}");
    assert_eq!(
        h.get("shop").await.traffic.unwrap().fields["url"],
        "http://169.254.169.254/latest",
        "stored"
    );
    run(&h, case("userinfo")).await.unwrap();
    println!("checked 1 case that must fail and 1 that still passes");
}

#[tokio::test]
async fn get_never_returns_a_secret() {
    let h = harness(|_| {}).await;
    let mut u = h.unchanged("shop").await;
    u.traffic = Some(traffic("http://prom.lan:9090", Some(set(HEADER))));
    let (status, put_body) = h.put("shop", &u).await;
    assert_eq!(status, 200, "{put_body}");
    let (_, body) = h.send(Method::GET, SHOP, None).await;
    for text in [&put_body, &body] {
        assert!(
            !text.contains("s3cret") && !text.contains("Bearer"),
            "{text}"
        );
    }
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["traffic"]["secrets"]["headers"], json!({ "set": true }));
    assert_eq!(v["traffic"]["fields"]["url"], "http://prom.lan:9090");
    // Stored sealed: the column holds no part of it either.
    let sealed: String = sqlx::query_scalar(
        "SELECT traffic_secrets_sealed FROM workspace_settings WHERE workspace_id = 'shop'",
    )
    .fetch_one(&h.app.db)
    .await
    .unwrap();
    assert!(!sealed.contains("s3cret") && !sealed.contains("Bearer"));
    // The registry carries it sealed, for the traffic task to open, and prints no field value.
    let (entries, _) = h.app.workspace_entries();
    let shop = entries.iter().find(|w| w.id == "shop").unwrap();
    let t = shop.settings.traffic.as_ref().unwrap();
    assert_eq!(t.secrets_sealed.as_deref(), Some(sealed.as_str()));
    assert!(!format!("{shop:?}").contains("prom.lan"), "{shop:?}");
    // `default` has no source, and the field list names the secret as a secret.
    assert!(h.get("default").await.traffic.is_none());
    let (_, list) = h.send(Method::GET, "/api/traffic/sources", None).await;
    let list: Vec<SourceFields> = serde_json::from_str(&list).unwrap();
    assert!(
        list[0]
            .fields
            .iter()
            .any(|f| f.name == "headers" && f.secret)
    );
    println!("checked 2 bodies, the row and the registry");
}

#[tokio::test]
async fn a_new_origin_clears_the_secrets() {
    let h = harness(|_| {}).await;
    let sealed = || async {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT traffic_secrets_sealed FROM workspace_settings WHERE workspace_id = 'shop'",
        )
        .fetch_one(&h.app.db)
        .await
        .unwrap()
    };
    let mut u = h.unchanged("shop").await;
    u.traffic = Some(traffic("http://prom.lan:9090", Some(set(HEADER))));
    let s = h.put_ok("shop", &u).await;
    let first = sealed().await.expect("sealed");

    // The same origin, another path, secrets "keep": the stored secret stays as it is.
    u.revision = s.revision;
    u.traffic = Some(traffic(
        "http://prom.lan:9090/prometheus",
        Some(SecretUpdate::Keep),
    ));
    let s = h.put_ok("shop", &u).await;
    assert!(s.traffic.as_ref().unwrap().secrets["headers"].set);
    assert_eq!(sealed().await.as_deref(), Some(first.as_str()));

    // A new host, secrets "keep": nothing is kept, so the URL cannot be repointed to collect it.
    u.revision = s.revision;
    u.traffic = Some(traffic(
        "http://evil.example:9090",
        Some(SecretUpdate::Keep),
    ));
    let s = h.put_ok("shop", &u).await;
    assert!(!s.traffic.as_ref().unwrap().secrets["headers"].set);
    assert_eq!(sealed().await, None);

    // Another new host with secrets "set": the new ones are kept.
    u.revision = s.revision;
    u.traffic = Some(traffic(
        "http://prom-2.lan:9090",
        Some(set("X-Scope-OrgID=team-b")),
    ));
    let s = h.put_ok("shop", &u).await;
    assert!(s.traffic.as_ref().unwrap().secrets["headers"].set);
    let key = h.app.config.token_key.as_ref().unwrap();
    let plain = crate::kernel::secrets::open(key, &sealed().await.unwrap()).unwrap();
    assert_eq!(plain, r#"{"headers":"X-Scope-OrgID=team-b"}"#);

    // "clear" removes it.
    u.revision = s.revision;
    u.traffic = Some(traffic("http://prom-2.lan:9090", Some(SecretUpdate::Clear)));
    let s = h.put_ok("shop", &u).await;
    assert!(!s.traffic.unwrap().secrets["headers"].set);
    assert_eq!(sealed().await, None);
    println!("checked 5 changes of origin and secrets");
}

#[tokio::test]
async fn a_stale_edit_answers_409() {
    let h = harness(|_| {}).await;
    for n in 1..=3 {
        let mut u = h.unchanged("shop").await;
        u.name = Some(format!("Shop {n}"));
        h.put_ok("shop", &u).await;
    }
    let mut a = h.unchanged("shop").await;
    assert_eq!(a.revision, 4);
    let mut b = a.clone();
    a.name = Some("Ann's shop".into());
    b.name = Some("Bo's shop".into());
    assert_eq!(h.put_ok("shop", &a).await.revision, 5);
    let (status, body) = h.put("shop", &b).await;
    assert_eq!(status, 409, "{body}");
    let refusal: SettingsRefusal = serde_json::from_str(&body).unwrap();
    assert_eq!(refusal.reason, reason::CHANGED_MEANWHILE);
    let current = refusal.current.expect("the settings as they are now");
    assert_eq!(
        (current.revision, current.name.as_deref()),
        (5, Some("Ann's shop"))
    );
    assert_eq!(h.get("shop").await.name.as_deref(), Some("Ann's shop"));
    assert_eq!(h.rejected(reason::CHANGED_MEANWHILE), 1);
    // The same settings again change nothing: no new revision.
    let same = h.unchanged("shop").await;
    assert_eq!(h.put_ok("shop", &same).await.revision, 5);
    // A value that does not read is refused by name.
    let mut bad = h.unchanged("shop").await;
    bad.default_attempt_limit = Some(0);
    let (status, body) = h.put("shop", &bad).await;
    assert_eq!(status, 422, "{body}");
    assert!(body.contains(reason::BAD_VALUE), "{body}");
    println!("checked 2 edits at revision 4, an unchanged one and a bad value");
}

#[tokio::test]
async fn a_ref_change_reloads_only_that_workspace() {
    let h = harness(|_| {}).await;
    let default_before = h
        .app
        .map_of(&crate::kernel::tenancy::WsKey::of_default("default"))
        .await
        .unwrap();
    let (entries, _) = h.app.workspace_entries();
    let old = entries.iter().find(|w| w.id == "shop").unwrap().clone();
    assert_eq!(
        h.app
            .map_of(&crate::kernel::tenancy::WsKey::of_default("shop"))
            .await
            .unwrap()
            .version,
        "sha-shop"
    );
    let mut events = h.app.web_tx.subscribe();

    let mut u = h.unchanged("shop").await;
    u.map_ref = "next".into();
    let s = h.put_ok("shop", &u).await;
    assert_eq!((s.map_ref.as_str(), s.revision), ("next", 2));

    let shop = h
        .app
        .map_of(&crate::kernel::tenancy::WsKey::of_default("shop"))
        .await
        .unwrap();
    assert_eq!(
        (shop.version.as_str(), shop.map.name.as_str()),
        ("sha-shop-next", "shop next")
    );
    let default_after = h
        .app
        .map_of(&crate::kernel::tenancy::WsKey::of_default("default"))
        .await
        .unwrap();
    assert!(
        Arc::ptr_eq(&default_before, &default_after),
        "default's map was reloaded"
    );
    assert_eq!(default_before.loaded_at, default_after.loaded_at);
    let cached: String =
        sqlx::query_scalar("SELECT version FROM map_cache WHERE workspace_id = 'shop'")
            .fetch_one(&h.app.db)
            .await
            .unwrap();
    assert_eq!(cached, "sha-shop-next");

    let mut seen = Vec::new();
    while let Ok(WebEvent::Changed {
        reason, workspace, ..
    }) = events.try_recv()
    {
        seen.push((reason, workspace));
    }
    assert!(
        seen.contains(&("map".into(), Some("shop".into()))),
        "{seen:?}"
    );
    assert!(
        seen.contains(&("settings".into(), Some("shop".into()))),
        "{seen:?}"
    );
    assert!(
        seen.iter().all(|(_, ws)| ws.as_deref() == Some("shop")),
        "only shop: {seen:?}"
    );

    // ADR 0087: what a workspace may use is what the tenant's installations cover; nothing is
    // read with the member's own token any more.
    let reads: Vec<String> = h
        .github
        .calls()
        .into_iter()
        .filter(|(m, _)| *m == "repo_as")
        .map(|(_, a)| a)
        .collect();
    assert!(reads.is_empty(), "{reads:?}");

    // A poll's reload of the old ref that finishes afterwards is dropped.
    let e = map::reload_ws(&h.app, &old).await.unwrap_err().to_string();
    assert!(e.contains("changed meanwhile"), "{e}");
    assert_eq!(
        h.app
            .map_of(&crate::kernel::tenancy::WsKey::of_default("shop"))
            .await
            .unwrap()
            .version,
        "sha-shop-next"
    );
    println!(
        "checked the reload, {} events, {} token reads and a dropped load",
        seen.len(),
        reads.len()
    );
}

#[tokio::test]
async fn create_and_delete() {
    let h = harness(|_| {}).await;
    let mut events = h.app.web_tx.subscribe();
    let (status, body) = h
        .send(
            Method::POST,
            "/api/workspaces",
            Some(json!({ "id": "blog", "name": "Blog", "map_repo": "acme/blog-map", "map_path": "system-map.yml" })),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let made: WorkspaceSettings = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (
            made.revision,
            made.map_ref.as_str(),
            made.updated_by.as_str()
        ),
        (1, "main", USER)
    );
    let list = || async {
        let (_, body) = h.send(Method::GET, "/api/workspaces", None).await;
        let list: Vec<WorkspaceInfo> = serde_json::from_str(&body).unwrap();
        list.into_iter()
            .map(|w| (w.id, w.name, w.version))
            .collect::<Vec<_>>()
    };
    let listed = list().await;
    assert_eq!(listed.len(), 4);
    assert_eq!(
        listed[3],
        ("blog".into(), "Blog".into(), Some("sha-blog".into())),
        "new workspaces last, with their map loaded"
    );
    assert!(matches!(
        events.try_recv(),
        Ok(WebEvent::Changed { reason, workspace: None, .. }) if reason == "workspaces"
    ));
    assert_eq!(h.gauge(metric::WORKSPACES_CONFIGURED), Some(4));
    // `gone` is not a row here: only what this test makes counts.
    assert_eq!(h.gauge(metric::WORKSPACE_ORPHAN_ROWS), Some(0));
    // Its routes answer at once.
    let (status, _) = h.send(Method::GET, "/api/w/blog/settings", None).await;
    assert_eq!(status, 200);

    // A pin in the new workspace, then the delete: the pin and the cached map are kept.
    sqlx::query("INSERT INTO pins (tenant_id, workspace_id, repo, number, kind, target, pinned_by, pinned_at) VALUES ('default', 'blog', 'acme/blog', 1, 'node', 'storefront', 'm0x6A', '2026-11-16T00:00:00Z')")
        .execute(&h.app.db)
        .await
        .unwrap();
    let (entries, _) = h.app.workspace_entries();
    let blog = entries.iter().find(|w| w.id == "blog").unwrap().clone();
    let (status, body) = h.send(Method::DELETE, "/api/w/blog", None).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(list().await.len(), 3);
    assert!(matches!(
        events.try_recv(),
        Ok(WebEvent::Changed { reason, workspace: None, .. }) if reason == "workspaces"
    ));
    let pins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pins WHERE workspace_id = 'blog'")
        .fetch_one(&h.app.db)
        .await
        .unwrap();
    assert_eq!(pins, 1, "rows are kept");
    assert_eq!(h.gauge(metric::WORKSPACES_CONFIGURED), Some(3));
    assert_eq!(
        h.gauge(metric::WORKSPACE_ORPHAN_ROWS),
        Some(2),
        "the pin and the cached map"
    );
    assert!(
        h.app
            .map_of(&crate::kernel::tenancy::WsKey::of_default("blog"))
            .await
            .is_none()
    );
    let (status, body) = h.send(Method::GET, "/api/w/blog/settings", None).await;
    assert_eq!(status, 404, "{body}");
    assert!(body.contains("unknown_workspace"), "{body}");

    // A map load that was in flight does not bring it back.
    assert!(map::reload_ws(&h.app, &blog).await.is_err());
    assert!(
        h.app
            .map_of(&crate::kernel::tenancy::WsKey::of_default("blog"))
            .await
            .is_none()
    );

    // Its id cannot be created again while its rows remain.
    let (status, body) = h
        .send(
            Method::POST,
            "/api/workspaces",
            Some(
                json!({ "id": "blog", "map_repo": "acme/blog-map", "map_path": "system-map.yml" }),
            ),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert!(body.contains(reason::ID_HAS_ROWS), "{body}");
    // Deleting it again: the workspace is unknown.
    let (status, _) = h.send(Method::DELETE, "/api/w/blog", None).await;
    assert_eq!(status, 404);
    println!("checked a create, a delete, 2 events, 2 gauges and a refused re-create");
}

#[tokio::test(flavor = "current_thread")]
async fn a_change_is_recorded_without_values() {
    let h = harness(|_| {}).await;
    let (events, _guard) = capture::start();
    let mut u = h.unchanged("shop").await;
    u.name = Some("Corner shop".into());
    u.default_spend_cap_usd = Some(7.5);
    u.traffic = Some(traffic("http://prom.lan:9090", Some(set(HEADER))));
    h.put_ok("shop", &u).await;

    let events = events.lock().unwrap().clone();
    let line = events
        .iter()
        .find(|e| e.message == "workspace config changed")
        .expect("the log line");
    assert_eq!(line.level, tracing::Level::INFO);
    let recorded = events
        .iter()
        .find(|e| e.message == format!("span {}", span::WORKSPACE_CONFIG_CHANGED))
        .expect("the span");
    let want = "name,default_spend_cap_usd,traffic.source,traffic.url,traffic.secrets.headers";
    assert_eq!(line.fields.get("keys").map(String::as_str), Some(want));
    assert_eq!(
        recorded
            .fields
            .get(attr::WORKSPACE_CHANGED_KEYS)
            .map(String::as_str),
        Some(want)
    );
    for e in [line, recorded] {
        assert_eq!(
            e.fields.get(attr::WORKSPACE).map(String::as_str),
            Some("shop")
        );
        assert_eq!(e.fields.get(attr::USER).map(String::as_str), Some(USER));
    }
    // No value and no secret, in the record or in any other line of the request.
    for e in &events {
        let text = format!("{} {:?}", e.message, e.fields);
        for value in ["s3cret", "Bearer", "Corner shop", "prom.lan", "7.5"] {
            assert!(!text.contains(value), "{value} in: {text}");
        }
    }
    println!("checked the log line, the span and {} events", events.len());
}
