//! Composition root: the router, shared by the binary and the integration tests.
//!
//! Unknown paths under `/api`, `/ws` and `/auth` answer 404 JSON and log a `warn!`, so a route
//! lost from this list cannot look like a page (design D13). Every other unknown GET serves
//! `index.html` for the single-page app's deep links.

use crate::kernel::state::App;
use crate::{auth, companion_link, launch, map, overview, pairing, pins, web_link};
use axum::Router;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use rubato_protocol::auth::routes as auth_routes;
use rubato_protocol::launch::routes as launch_routes;
use rubato_protocol::map::routes as map_routes;
use rubato_protocol::pairing::routes as pr;
use rubato_protocol::pins::routes as pin_routes;
use rubato_protocol::progress::routes as overview_routes;
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
        .route(
            pin_routes::PINS,
            post(pins::add_pin).delete(pins::remove_pin),
        )
        .route(launch_routes::LAUNCH, post(launch::launch))
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
            (Method::POST, pin_routes::PINS.into()),
            (Method::DELETE, pin_routes::PINS.into()),
            (Method::POST, launch_routes::LAUNCH.into()),
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
