//! The router, shared by the binary and the integration tests.

use crate::state::App;
use crate::{api, auth, pairing, ws_companion, ws_web};
use axum::Router;
use axum::routing::{delete, get, post};
use rubato_protocol::pairing::routes as pr;
use rubato_protocol::web::routes as wr;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

pub fn router(app: App) -> Router {
    let dist = app.config.web_dist.clone();
    let index = dist.join("index.html");
    let api = Router::new()
        .route("/api/health", get(api::health))
        .route(wr::ME, get(api::me))
        .route(wr::OVERVIEW, get(api::overview))
        .route(wr::MAP, get(api::map))
        .route(wr::MAP_RELOAD, post(api::map_reload))
        .route(wr::PINS, post(api::add_pin).delete(api::remove_pin))
        .route(wr::LAUNCH, post(api::launch))
        .route(wr::PAIRINGS, get(pairing::list))
        .route(
            &format!("{}/{{companion_id}}", wr::PAIRINGS),
            delete(pairing::revoke),
        )
        .route(wr::PAIR_APPROVE, post(pairing::approve))
        .route(pr::PAIR_START, post(pairing::start))
        .route(pr::PAIR_POLL, post(pairing::poll))
        .route(pr::COMPANION_WS, get(ws_companion::upgrade))
        .route(wr::WEB_WS, get(ws_web::upgrade))
        .route(wr::LOGIN, get(auth::login))
        .route(wr::CALLBACK, get(auth::callback))
        .route(wr::LOGOUT, post(auth::logout));
    api.fallback_service(ServeDir::new(dist).not_found_service(ServeFile::new(index)))
        .layer(TraceLayer::new_for_http())
        .with_state(app)
}
