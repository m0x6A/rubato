//! `GET /api/map` and `POST /api/map/reload`.

use super::load;
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rubato_protocol::web::MapResponse;

pub async fn map(
    State(app): State<App>,
    _user: CurrentUser,
) -> Result<Json<MapResponse>, StatusCode> {
    let m = app
        .map
        .read()
        .await
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(MapResponse {
        map: m.map,
        version: m.version,
        loaded_at: m.loaded_at,
    }))
}

pub async fn map_reload(
    State(app): State<App>,
    _user: CurrentUser,
) -> Result<Json<MapResponse>, StatusCode> {
    let m = load::reload(&app).await.map_err(|e| {
        tracing::error!(slice = "map", error = %e, "map reload");
        StatusCode::BAD_GATEWAY
    })?;
    app.notify("map");
    Ok(Json(MapResponse {
        map: m.map,
        version: m.version,
        loaded_at: m.loaded_at,
    }))
}
