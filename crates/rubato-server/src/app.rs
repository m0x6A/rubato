//! Composition root: the router, shared by the binary and the integration tests.
//!
//! Unknown paths under `/api`, `/ws` and `/auth` answer 404 JSON and log a `warn!`, so a route
//! lost from this list cannot look like a page (design D13). Every other unknown GET serves
//! `index.html` for the single-page app's deep links.

use crate::kernel::state::App;
use crate::{
    auth, companion_link, launch, layout, map, map_draft, overview, pairing, pins, queue, web_link,
};
use axum::Router;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post, put};
use rubato_protocol::auth::routes as auth_routes;
use rubato_protocol::launch::routes as launch_routes;
use rubato_protocol::layout::routes as layout_routes;
use rubato_protocol::map::routes as map_routes;
use rubato_protocol::map_draft::routes as draft_routes;
use rubato_protocol::pairing::routes as pr;
use rubato_protocol::pins::routes as pin_routes;
use rubato_protocol::progress::routes as overview_routes;
use rubato_protocol::queue::routes as queue_routes;
use rubato_protocol::web_link::routes as web_routes;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

/// The body of the 404 answer for an unknown API path.
pub const UNKNOWN_API_PATH: &str = r#"{"error":"unknown api path"}"#;

pub fn router(app: App) -> Router {
    let dist = app.config.web_dist.clone();
    let index = dist.join("index.html");
    let api = Router::new()
        .route("/api/health", get(health))
        .route(auth_routes::ME, get(auth::me))
        .route(overview_routes::OVERVIEW, get(overview::overview))
        .route(map_routes::MAP, get(map::map))
        .route(map_routes::MAP_RELOAD, post(map::map_reload))
        .route(draft_routes::DRAFT_OPS, post(map_draft::add_op))
        .route(
            &format!("{}/{{seq}}", draft_routes::DRAFT_OPS),
            delete(map_draft::remove_op),
        )
        .route(draft_routes::DRAFT_DISCARD, post(map_draft::discard))
        .route(draft_routes::DRAFT_RESTORE, post(map_draft::restore))
        .route(draft_routes::DRAFT_YAML, get(map_draft::proposed_yaml))
        .route(draft_routes::DRAFT_PROPOSE, post(map_draft::propose))
        .route(draft_routes::DRAFT_WITHDRAW, post(map_draft::withdraw))
        .route(
            pin_routes::PINS,
            post(pins::add_pin).delete(pins::remove_pin),
        )
        .route(launch_routes::LAUNCH, post(launch::launch))
        .route(
            layout_routes::LAYOUT,
            put(layout::put_layout).delete(layout::clear_layout),
        )
        .route(
            queue_routes::QUEUE,
            post(queue::add).put(queue::reorder).delete(queue::remove),
        )
        .route(pr::PAIRINGS, get(pairing::list))
        .route(
            &format!("{}/{{companion_id}}", pr::PAIRINGS),
            delete(pairing::revoke),
        )
        .route(pr::PAIR_APPROVE, post(pairing::approve))
        .route(pr::PAIR_START, post(pairing::start))
        .route(pr::PAIR_POLL, post(pairing::poll))
        .route(pr::COMPANION_WS, get(companion_link::upgrade))
        .route(web_routes::WEB_WS, get(web_link::upgrade))
        .route(auth_routes::LOGIN, get(auth::login))
        .route(auth_routes::CALLBACK, get(auth::callback))
        .route(auth_routes::LOGOUT, post(auth::logout))
        .route("/api/{*rest}", any(unknown_api_path))
        .route("/ws/{*rest}", any(unknown_api_path))
        .route("/auth/{*rest}", any(unknown_api_path));
    api.fallback_service(ServeDir::new(dist).not_found_service(ServeFile::new(index)))
        .layer(TraceLayer::new_for_http())
        .with_state(app)
}

async fn health() -> &'static str {
    "ok"
}

async fn unknown_api_path(method: Method, uri: Uri) -> Response {
    tracing::warn!(%method, path = %uri.path(), "unknown api path");
    (
        StatusCode::NOT_FOUND,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        UNKNOWN_API_PATH,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use std::sync::Arc;

    async fn serve(web_dist: Option<std::path::PathBuf>) -> (String, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let mut cfg = Config::for_tests(url, "m0x6A");
        if let Some(d) = web_dist {
            cfg.web_dist = d;
        }
        let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router(app)).await.unwrap() });
        (format!("http://{addr}"), tmp)
    }

    /// Every route the server registers, with its method. Kept by hand: the probe fails when a
    /// line above goes missing, because the path then reaches the 404 fallback.
    fn known_routes() -> Vec<(Method, String)> {
        let revoke = format!("{}/{}", pr::PAIRINGS, uuid::Uuid::new_v4());
        vec![
            (Method::GET, "/api/health".into()),
            (Method::GET, auth_routes::ME.into()),
            (Method::GET, overview_routes::OVERVIEW.into()),
            (Method::GET, map_routes::MAP.into()),
            (Method::POST, map_routes::MAP_RELOAD.into()),
            (Method::POST, draft_routes::DRAFT_OPS.into()),
            (Method::DELETE, format!("{}/1", draft_routes::DRAFT_OPS)),
            (Method::POST, draft_routes::DRAFT_DISCARD.into()),
            (Method::POST, draft_routes::DRAFT_RESTORE.into()),
            (Method::GET, draft_routes::DRAFT_YAML.into()),
            (Method::POST, draft_routes::DRAFT_PROPOSE.into()),
            (Method::POST, draft_routes::DRAFT_WITHDRAW.into()),
            (Method::POST, pin_routes::PINS.into()),
            (Method::DELETE, pin_routes::PINS.into()),
            (Method::POST, launch_routes::LAUNCH.into()),
            (Method::PUT, layout_routes::LAYOUT.into()),
            (Method::DELETE, layout_routes::LAYOUT.into()),
            (Method::POST, queue_routes::QUEUE.into()),
            (Method::PUT, queue_routes::QUEUE.into()),
            (Method::DELETE, queue_routes::QUEUE.into()),
            (Method::GET, pr::PAIRINGS.into()),
            (Method::DELETE, revoke),
            (Method::POST, pr::PAIR_APPROVE.into()),
            (Method::POST, pr::PAIR_START.into()),
            (Method::POST, pr::PAIR_POLL.into()),
            (Method::GET, pr::COMPANION_WS.into()),
            (Method::GET, web_routes::WEB_WS.into()),
            (Method::GET, auth_routes::LOGIN.into()),
            (Method::GET, auth_routes::CALLBACK.into()),
            (Method::POST, auth_routes::LOGOUT.into()),
        ]
    }

    /// Refusals are counted by route and reason, and logged with who (design O3, O12).
    #[tokio::test(flavor = "current_thread")]
    async fn refusals_are_counted_by_route_and_reason() {
        use crate::kernel::config::GitHubApp;
        use crate::kernel::telemetry::{capture, metric_reader};
        use rubato_protocol::otel::{attr, metric};
        use rubato_protocol::{IssueRef, MapRef, PinRequest, SystemMap};

        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(
            pool,
            Config::for_tests(url, "m0x6A"),
            Arc::new(FakeGitHub::new()),
        );
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        let map: SystemMap =
            serde_yaml_ng::from_str(include_str!("../../../docs/design/rubato-map.yml")).unwrap();
        *app.map.write().await = Some(crate::kernel::state::LoadedMap {
            map,
            version: "v".into(),
            loaded_at: chrono::Utc::now(),
            yaml: String::new(),
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = app.clone();
        tokio::spawn(async move { axum::serve(listener, router(served)).await.unwrap() });
        let http = reqwest::Client::new();
        let (events, _guard) = capture::start();
        let count = |reason: &str| {
            reader
                .points(metric::WEB_REJECTED)
                .into_iter()
                .find(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(reason))
                .map(|p| {
                    (
                        p.value,
                        p.attrs.get(attr::ROUTE).cloned().unwrap_or_default(),
                    )
                })
        };

        // A part's id pinned as a seam, then an id the map does not have.
        for (target, _) in [
            (MapRef::seam("rubato-server"), "kind_mismatch"),
            (MapRef::node("nope"), "unknown_id"),
        ] {
            let r = http
                .post(format!("{base}{}", pin_routes::PINS))
                .json(&PinRequest {
                    issue: IssueRef::new("m0x6A/rubato", 48),
                    target,
                })
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 422);
        }
        assert_eq!(
            count("kind_mismatch"),
            Some((1, pin_routes::PINS.to_owned()))
        );
        assert_eq!(count("unknown_id"), Some((1, pin_routes::PINS.to_owned())));
        {
            let events = events.lock().unwrap();
            let line = events
                .iter()
                .find(|e| {
                    e.message == "web write rejected"
                        && e.fields.get("rubato.reason").map(String::as_str)
                            == Some("kind_mismatch")
                })
                .expect("an info line per refusal");
            assert_eq!(line.level, tracing::Level::INFO);
            assert_eq!(
                line.fields.get("rubato.user").map(String::as_str),
                Some("m0x6A")
            );
            assert_eq!(
                line.fields.get("rubato.route").map(String::as_str),
                Some(pin_routes::PINS)
            );
        }

        // The layout: a part the map lacks and a position off the canvas are refused and
        // counted; an accepted write and a clear each move `rubato.layout.writes`.
        use rubato_protocol::layout::{MapLayout, Position};
        let layout = |id: &str, x: f64| MapLayout {
            positions: [(id.to_owned(), Position { x, y: 0.0 })].into(),
        };
        let put = |l: MapLayout| {
            http.put(format!("{base}{}", layout_routes::LAYOUT))
                .json(&l)
                .send()
        };
        assert_eq!(put(layout("nope", 1.0)).await.unwrap().status(), 422);
        assert_eq!(put(layout("web", 1.0e9)).await.unwrap().status(), 422);
        assert!(
            reader.points(metric::LAYOUT_WRITES).is_empty(),
            "a refused write is not a write"
        );
        let by_route = |reason: &str| {
            reader
                .points(metric::WEB_REJECTED)
                .into_iter()
                .find(|p| {
                    p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(reason)
                        && p.attrs.get(attr::ROUTE).map(String::as_str)
                            == Some(layout_routes::LAYOUT)
                })
                .map(|p| p.value)
        };
        assert_eq!(by_route("unknown_id"), Some(1));
        assert_eq!(by_route("bad_position"), Some(1));
        assert_eq!(put(layout("web", 10.0)).await.unwrap().status(), 204);
        let cleared = http
            .delete(format!("{base}{}", layout_routes::LAYOUT))
            .send()
            .await
            .unwrap();
        assert_eq!(cleared.status(), 200);
        assert_eq!(reader.points(metric::LAYOUT_WRITES)[0].value, 2);
        {
            let events = events.lock().unwrap();
            assert!(
                events.iter().any(|e| e.message == "layout cleared"
                    && e.level == tracing::Level::INFO
                    && e.fields.get("rubato.user").map(String::as_str) == Some("m0x6A")),
                "clearing the team's layout is logged with who did it; max level {:?}, captured: {:?}",
                tracing::level_filters::LevelFilter::current(),
                events
                    .iter()
                    .map(|e| format!("{} {} {:?}", e.level, e.message, e.fields))
                    .collect::<Vec<_>>()
            );
        }

        // The map draft: an operation the rules refuse is counted with its reason.
        let bad_op = serde_json::json!({ "op": "add_node", "op_id": uuid::Uuid::new_v4(), "node": { "id": "web", "kind": "context" } });
        let r = http
            .post(format!("{base}{}", draft_routes::DRAFT_OPS))
            .json(&bad_op)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422);
        assert!(r.text().await.unwrap().contains("already exists"));
        let refused = reader
            .points(metric::WEB_REJECTED)
            .into_iter()
            .find(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some("invalid_op"))
            .expect("counted");
        assert_eq!(refused.attrs[attr::ROUTE], draft_routes::DRAFT_OPS);
        // Proposing through a connection that cannot write is refused and counted too.
        let ok_op = serde_json::json!({ "op": "add_node", "op_id": uuid::Uuid::new_v4(), "node": { "id": "billing", "kind": "context" } });
        assert_eq!(
            http.post(format!("{base}{}", draft_routes::DRAFT_OPS))
                .json(&ok_op)
                .send()
                .await
                .unwrap()
                .status(),
            201
        );
        assert_eq!(
            http.post(format!("{base}{}", draft_routes::DRAFT_PROPOSE))
                .send()
                .await
                .unwrap()
                .status(),
            409
        );
        assert!(reader.points(metric::WEB_REJECTED).iter().any(|p| {
            p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some("cannot_write")
                && p.attrs[attr::ROUTE] == draft_routes::DRAFT_PROPOSE
        }));

        // The queue: a lane that is not the caller's (or does not exist) refuses all three
        // writes, each counted.
        use rubato_protocol::queue::{QueueOrder, QueueRequest};
        let nobody = uuid::Uuid::new_v4();
        let entry = QueueRequest {
            issue: IssueRef::new("m0x6A/rubato", 48),
            companion_id: nobody,
        };
        let url = format!("{base}{}", queue_routes::QUEUE);
        assert_eq!(
            http.post(&url).json(&entry).send().await.unwrap().status(),
            403
        );
        assert_eq!(
            http.delete(&url)
                .json(&entry)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        let order = QueueOrder {
            companion_id: nobody,
            issues: vec![],
        };
        assert_eq!(
            http.put(&url).json(&order).send().await.unwrap().status(),
            403
        );
        let refused = reader
            .points(metric::WEB_REJECTED)
            .into_iter()
            .find(|p| {
                p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some("not_own_companion")
            })
            .expect("counted");
        assert_eq!(refused.value, 3);
        assert_eq!(refused.attrs[attr::ROUTE], queue_routes::QUEUE);

        // As a GitHub App with no dev user: a session without a team record is refused, counted.
        let tmp2 = tempfile::tempdir().unwrap();
        let url2 = format!("sqlite://{}/t.db?mode=rwc", tmp2.path().display());
        let pool2 = crate::kernel::db::connect(&url2).await.unwrap();
        let mut cfg = Config::for_tests(url2, "unused");
        cfg.dev_user = None;
        cfg.github_app = Some(GitHubApp {
            app_id: 1,
            private_key: "unused".into(),
            installation_id: 2,
        });
        let (reader2, metrics2) = metric_reader::start();
        let mut gated = AppState::new(pool2, cfg, Arc::new(FakeGitHub::new()));
        Arc::get_mut(&mut gated).unwrap().metrics = metrics2;
        sqlx::query("INSERT INTO web_sessions (id, login, created_at) VALUES ('s1', 'old-timer', '2026-11-01T00:00:00Z')")
            .execute(&gated.db)
            .await
            .unwrap();
        let cookie = format!("rubato_session={}", gated.signer.sign("s1"));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base2 = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router(gated)).await.unwrap() });
        let r = http
            .get(format!("{base2}{}", overview_routes::OVERVIEW))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
        let stale = reader2.points(metric::WEB_REJECTED);
        assert_eq!(stale.len(), 1, "{stale:?}");
        assert_eq!(stale[0].attrs[attr::DROP_REASON], "team_record_stale");
        assert_eq!(stale[0].attrs[attr::ROUTE], overview_routes::OVERVIEW);
        println!("checked 8 refusal reasons");
    }

    #[tokio::test]
    async fn every_known_route_answers() {
        let (url, _tmp) = serve(None).await;
        let http = reqwest::Client::new();
        let routes = known_routes();
        for (method, path) in &routes {
            let r = http
                .request(method.clone(), format!("{url}{path}"))
                .send()
                .await
                .unwrap();
            let status = r.status();
            let body = r.text().await.unwrap();
            assert_ne!(status, 405, "{method} {path} is not routed for its method");
            assert_ne!(
                body, UNKNOWN_API_PATH,
                "{method} {path} reached the fallback"
            );
        }
        println!("probed {} routes", routes.len());
    }

    #[tokio::test]
    async fn unknown_api_paths_are_404_json() {
        let (url, _tmp) = serve(None).await;
        let http = reqwest::Client::new();
        for (method, path) in [
            (Method::GET, "/api/nope"),
            (Method::POST, "/api/nope"),
            (Method::GET, "/ws/nope"),
            (Method::GET, "/auth/nope"),
        ] {
            let r = http
                .request(method.clone(), format!("{url}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 404, "{method} {path}");
            assert_eq!(
                r.headers()["content-type"],
                "application/json",
                "{method} {path}"
            );
            assert_eq!(r.text().await.unwrap(), UNKNOWN_API_PATH);
        }
    }

    #[tokio::test]
    async fn deep_links_still_serve_the_app() {
        let dist = tempfile::tempdir().unwrap();
        std::fs::write(
            dist.path().join("index.html"),
            "<!doctype html><title>r</title>",
        )
        .unwrap();
        let (url, _tmp) = serve(Some(dist.path().to_owned())).await;
        let r = reqwest::get(format!("{url}/some/page")).await.unwrap();
        // As on main: `not_found_service` serves index.html with status 404 (design N17 said
        // 200; the status is kept, not changed, by this move).
        assert_eq!(r.status(), 404);
        assert!(
            r.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert!(r.text().await.unwrap().contains("<title>r</title>"));
    }
}
