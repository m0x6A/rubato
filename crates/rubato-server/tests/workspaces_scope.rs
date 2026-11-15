//! Plan 89 PR 4: two workspaces on one server (ADRs 0044, 0047, 0049). `default` holds the team's
//! own map; `shop` holds a small map that reuses the id `rubato-server`, so a write to one
//! workspace that leaks into the other shows. Each map-scoped write is made under `/api/w/shop`
//! and read back from both.

use futures_util::StreamExt;
use rubato_protocol::{
    IssueRef, MapRef, Overview, PinRequest, WebEvent, WorkspaceInfo, workspace::routes::scoped,
};
use rubato_server::kernel::config::Config;
use rubato_server::kernel::db;
use rubato_server::kernel::github::FakeGitHub;
use rubato_server::kernel::state::{App, AppState};
use rubato_server::kernel::workspaces::WorkspaceEntry;
use rubato_server::{app, map};
use std::path::PathBuf;
use std::sync::Arc;

const SHOP: &str = r#"
version: 1
name: shop
repos: [{ name: acme/shop }]
nodes:
  - { id: rubato-server, kind: context, repo: acme/shop, description: "The shop's own server" }
  - { id: storefront, kind: context, repo: acme/shop }
seams:
  - { id: storefront-server, from: storefront, to: rubato-server, owner: rubato-server }
"#;

struct H {
    url: String,
    app: App,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

async fn two_workspaces() -> H {
    let tmp = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = db::connect(&db_url).await.unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let shop_file = tmp.path().join("shop.yml");
    std::fs::write(&shop_file, SHOP).unwrap();
    let mut cfg = Config::for_tests(db_url, "m0x6A");
    cfg.workspaces = vec![
        WorkspaceEntry {
            id: "default".into(),
            name: Some("Rubato".into()),
            map_repo: "m0x6A/rubato".into(),
            map_path: "docs/design/rubato-map.yml".into(),
            map_ref: "main".into(),
            map_file: Some(root.join("docs/design/rubato-map.yml")),
        },
        WorkspaceEntry {
            id: "shop".into(),
            name: None,
            map_repo: "acme/shop-map".into(),
            map_path: "system-map.yml".into(),
            map_ref: "main".into(),
            map_file: Some(shop_file),
        },
    ];
    let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    map::reload(&app).await.unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
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
    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.url))
            .header("x-rubato-dev-user", "m0x6A")
    }

    async fn overview(&self, ws: Option<&str>) -> Overview {
        let path = ws.map_or("/api/overview".to_string(), |w| scoped(w, "/api/overview"));
        self.req(reqwest::Method::GET, &path)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn each_workspace_has_its_own_map_and_the_list_names_both() {
    let h = two_workspaces().await;
    let list: Vec<WorkspaceInfo> = h
        .req(reqwest::Method::GET, "/api/workspaces")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let names: Vec<(&str, &str)> = list
        .iter()
        .map(|w| (w.id.as_str(), w.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [("default", "Rubato"), ("shop", "shop")],
        "entry name, else the map's"
    );
    assert!(list.iter().all(|w| w.version.is_some()), "both maps loaded");

    let shop = h.overview(Some("shop")).await;
    assert_eq!(shop.workspace, "shop");
    assert_eq!(shop.map.map.name, "shop");
    assert_eq!(shop.workspaces.len(), 2);
    let default = h.overview(None).await;
    assert_eq!(default.workspace, "default");
    assert_eq!(default.map.map.name, "rubato");
    let same = h.overview(Some("default")).await;
    assert_eq!(
        same.map.version, default.map.version,
        "unprefixed is default"
    );
}

#[tokio::test]
async fn a_layout_written_in_one_workspace_stays_there() {
    let h = two_workspaces().await;
    let r = h
        .req(reqwest::Method::PUT, &scoped("shop", "/api/layout"))
        .json(&serde_json::json!({ "positions": { "rubato-server": { "x": 7.0, "y": 9.0 } } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204, "{}", r.text().await.unwrap());
    let shop = h.overview(Some("shop")).await;
    assert_eq!(
        shop.layout
            .positions
            .get("rubato-server")
            .map(|p| (p.x, p.y)),
        Some((7.0, 9.0))
    );
    let default = h.overview(None).await;
    assert!(
        !default.layout.positions.contains_key("rubato-server"),
        "default's part of the same id did not move"
    );
}

#[tokio::test]
async fn a_pin_and_a_draft_op_in_one_workspace_stay_there() {
    let h = two_workspaces().await;
    let issue = IssueRef::new("acme/shop", 3);
    let r = h
        .req(reqwest::Method::POST, &scoped("shop", "/api/pins"))
        .json(&PinRequest {
            issue: issue.clone(),
            target: MapRef::node("storefront"),
        })
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let pins_in = |ov: &Overview| {
        ov.issues
            .iter()
            .filter(|i| i.issue == issue)
            .flat_map(|i| i.pins.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        pins_in(&h.overview(Some("shop")).await),
        [MapRef::node("storefront")]
    );
    assert!(
        pins_in(&h.overview(None).await).is_empty(),
        "not a pin in default"
    );

    // `storefront` is not in default's map: pinning it there is refused, so nothing falls back.
    let r = h
        .req(reqwest::Method::POST, "/api/pins")
        .json(&PinRequest {
            issue: issue.clone(),
            target: MapRef::node("storefront"),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);

    let op = serde_json::json!({ "op_id": uuid::Uuid::new_v4(), "op": "add_node", "node": { "id": "checkout", "kind": "context" } });
    let r = h
        .req(reqwest::Method::POST, &scoped("shop", "/api/map/draft/ops"))
        .json(&op)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    assert_eq!(h.overview(Some("shop")).await.draft.ops.len(), 1);
    assert!(
        h.overview(None).await.draft.ops.is_empty(),
        "default's draft is its own"
    );
}

#[tokio::test]
async fn a_layout_change_tells_browsers_which_workspace_changed() {
    let h = two_workspaces().await;
    let ws_url = format!("ws://{}/ws/web", h.url.trim_start_matches("http://"));
    let mut req =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(ws_url)
            .unwrap();
    req.headers_mut()
        .insert("x-rubato-dev-user", "m0x6A".parse().unwrap());
    let (mut web, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    h.req(reqwest::Method::PUT, &scoped("shop", "/api/layout"))
        .json(&serde_json::json!({ "positions": { "storefront": { "x": 1.0, "y": 2.0 } } }))
        .send()
        .await
        .unwrap();
    let ev = loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), web.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
            let ev: WebEvent = serde_json::from_str(&t).unwrap();
            let WebEvent::Changed { reason, .. } = &ev;
            if reason == "layout" {
                break ev;
            }
        }
    };
    let WebEvent::Changed { workspace, .. } = ev;
    assert_eq!(workspace.as_deref(), Some("shop"));
    let _ = &h.app;
}

// Two maps that both name `acme/auth` (ADR 0045): `default` is the admin application, `shop`
// the storefront. `auth-api` is in both; `storefront` only in shop.
const ADMIN: &str = r#"
version: 1
name: admin
repos: [{ name: acme/auth }]
nodes:
  - { id: auth-api, kind: context, repo: acme/auth }
  - { id: admin-ui, kind: context, repo: acme/admin }
seams:
  - { id: admin-auth, from: admin-ui, to: auth-api, owner: auth-api }
"#;
const STORE: &str = r#"
version: 1
name: shop
repos: [{ name: acme/auth }]
nodes:
  - { id: auth-api, kind: context, repo: acme/auth }
  - { id: storefront, kind: context, repo: acme/shop }
seams:
  - { id: storefront-auth, from: storefront, to: auth-api, owner: auth-api }
"#;
const SHA: &str = "5555555555555555555555555555555555555555";

async fn shared_auth() -> H {
    let tmp = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = db::connect(&db_url).await.unwrap();
    let mut cfg = Config::for_tests(db_url, "m0x6A");
    let mut entries = vec![];
    for (id, yaml) in [("default", ADMIN), ("shop", STORE)] {
        let file = tmp.path().join(format!("{id}.yml"));
        std::fs::write(&file, yaml).unwrap();
        entries.push(WorkspaceEntry {
            id: id.into(),
            name: None,
            map_repo: format!("acme/{id}-map"),
            map_path: "system-map.yml".into(),
            map_ref: "main".into(),
            map_file: Some(file),
        });
    }
    cfg.workspaces = entries;
    let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    map::reload(&app).await.unwrap();
    // Plans as the poll stores them: #5 names a part only shop has, #6 one neither has.
    for (n, part) in [(5, "storefront"), (6, "billing")] {
        let plan = serde_json::json!({
            "issue": { "repo": "acme/auth", "number": n }, "path": format!("docs/plans/{n}.md"),
            "affected_parts": [{ "node": part, "status": "affected", "change": "x" }], "o_rows": []
        });
        sqlx::query("INSERT INTO plans (repo, number, branch, path, plan, parse_error, fetched_at, approval_sha) VALUES ('acme/auth', ?, ?, ?, ?, NULL, '2026-11-14T08:00:00Z', ?)")
            .bind(n).bind(format!("issue/{n}-x")).bind(format!("docs/plans/{n}.md")).bind(plan.to_string()).bind(SHA)
            .execute(&app.db).await.unwrap();
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app::router(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    H {
        url: format!("http://{addr}"),
        app,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

fn summary(ov: &Overview, n: u64) -> &rubato_protocol::IssueSummary {
    ov.issues
        .iter()
        .find(|i| i.issue.number == n)
        .unwrap_or_else(|| panic!("#{n} not in {}", ov.workspace))
}

fn unknown_ids_for(ov: &Overview, n: u64) -> bool {
    ov.findings.iter().any(
        |f| matches!(f, rubato_protocol::Finding::UnknownIds { issue, .. } if issue.number == n),
    )
}

#[tokio::test]
async fn a_plan_fits_where_its_ids_resolve_and_is_a_finding_only_where_none_do() {
    let h = shared_auth().await;
    let admin = h.overview(None).await;
    let shop = h.overview(Some("shop")).await;

    // #5 names `storefront`: it fits shop, shows in both (both maps name acme/auth), and admin
    // says where it counts instead of raising unknown ids.
    assert_eq!(summary(&shop, 5).fits_in, ["shop"]);
    assert!(summary(&shop, 5).fit.as_ref().unwrap().fits);
    assert_eq!(summary(&admin, 5).fits_in, ["shop"]);
    assert!(
        summary(&admin, 5).fit.as_ref().unwrap().fits,
        "the fit where it counts is shown"
    );
    assert!(!unknown_ids_for(&admin, 5) && !unknown_ids_for(&shop, 5));

    // #6 names `billing`: it fits nowhere, so both raise it.
    assert!(summary(&admin, 6).fits_in.is_empty());
    assert!(unknown_ids_for(&admin, 6) && unknown_ids_for(&shop, 6));
}

#[tokio::test]
async fn accept_plan_refuses_only_a_plan_that_fits_none_of_its_workspaces() {
    let h = shared_auth().await;
    let reason = |n: u64| {
        let h = &h;
        async move {
            let r = h
                .req(reqwest::Method::POST, "/api/plans/approve")
                .json(&rubato_protocol::ApproveRequest {
                    issue: IssueRef::new("acme/auth", n),
                    sha: SHA.into(),
                })
                .send()
                .await
                .unwrap();
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            body["reason"].as_str().unwrap_or_default().to_string()
        }
    };
    // #5 fits shop: accepted from admin's view past the fit check (it stops later, for want of a
    // person's GitHub token in this test).
    assert_ne!(reason(5).await, "plan_does_not_fit");
    assert_eq!(reason(6).await, "plan_does_not_fit");
}

#[tokio::test]
async fn an_issue_pinned_in_another_workspace_says_so() {
    let h = shared_auth().await;
    let issue = IssueRef::new("acme/auth", 7);
    let r = h
        .req(reqwest::Method::POST, &scoped("shop", "/api/pins"))
        .json(&PinRequest {
            issue: issue.clone(),
            target: MapRef::node("auth-api"),
        })
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let admin = h.overview(None).await;
    let shop = h.overview(Some("shop")).await;
    assert_eq!(
        summary(&admin, 7).pinned_in,
        ["shop"],
        "admin holds acme/auth and says where it is pinned"
    );
    assert!(summary(&admin, 7).pins.is_empty());
    assert!(summary(&shop, 7).pinned_in.is_empty());
    // An issue in a repo only shop's map names, pinned in shop, is not admin's.
    let r = h
        .req(reqwest::Method::POST, &scoped("shop", "/api/pins"))
        .json(&PinRequest {
            issue: IssueRef::new("acme/shop", 8),
            target: MapRef::node("storefront"),
        })
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    assert!(
        h.overview(None)
            .await
            .issues
            .iter()
            .all(|i| i.issue.number != 8)
    );
    assert!(
        h.overview(Some("shop"))
            .await
            .issues
            .iter()
            .any(|i| i.issue.number == 8)
    );
}
