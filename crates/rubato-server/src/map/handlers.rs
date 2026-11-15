//! `GET /api/map`, `POST /api/map/reload` (each per workspace, ADR 0049) and `GET /api/workspaces`.

use super::load;
use crate::auth::CurrentUser;
use crate::kernel::http::Ws;
use crate::kernel::state::App;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rubato_protocol::{MapResponse, WorkspaceInfo};

pub async fn map(
    State(app): State<App>,
    ws: Ws,
    _user: CurrentUser,
) -> Result<Json<MapResponse>, StatusCode> {
    let m = app
        .map_of(ws.as_str())
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(MapResponse {
        map: m.map,
        version: m.version,
        loaded_at: m.loaded_at,
    }))
}

pub async fn map_reload(
    State(app): State<App>,
    ws: Ws,
    _user: CurrentUser,
) -> Result<Json<MapResponse>, StatusCode> {
    let (entries, _) = app.config.workspace_entries();
    let entry = entries
        .iter()
        .find(|w| w.id == ws.as_str())
        .ok_or(StatusCode::NOT_FOUND)?;
    let m = load::reload_ws(&app, entry).await.map_err(|e| {
        tracing::error!(slice = "map", rubato.workspace = %ws.as_str(), error = %e, "map reload");
        StatusCode::BAD_GATEWAY
    })?;
    app.notify_ws("map", ws.as_str());
    Ok(Json(MapResponse {
        map: m.map,
        version: m.version,
        loaded_at: m.loaded_at,
    }))
}

/// Every configured workspace in switcher order, named by its entry or else by its map.
pub async fn list(app: &App) -> Vec<WorkspaceInfo> {
    let (entries, _) = app.config.workspace_entries();
    let mut out = Vec::with_capacity(entries.len());
    for w in entries {
        let loaded = app.map_of(&w.id).await;
        out.push(WorkspaceInfo {
            name: w
                .name
                .clone()
                .or_else(|| loaded.as_ref().map(|m| m.map.name.clone()))
                .unwrap_or_else(|| w.id.clone()),
            id: w.id,
            map_repo: w.map_repo,
            version: loaded.as_ref().map(|m| m.version.clone()),
            loaded_at: loaded.as_ref().map(|m| m.loaded_at),
        });
    }
    out
}

pub async fn workspaces(State(app): State<App>, _user: CurrentUser) -> Json<Vec<WorkspaceInfo>> {
    Json(list(&app).await)
}
