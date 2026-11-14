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
