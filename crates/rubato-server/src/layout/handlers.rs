//! `PUT /api/layout` stores the parts that moved; `DELETE /api/layout` clears the layout and
//! answers with what it removed, so the caller can offer Undo.

use super::store;
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::map_draft;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::layout::{MAX_COORDINATE, MapLayout, routes};

const SLICE: &str = "layout";

/// The reason values of a refused layout write, as counted in `rubato.web.rejected`.
pub mod reason {
    pub const UNKNOWN_ID: &str = "unknown_id";
    pub const BAD_POSITION: &str = "bad_position";
}

fn written(app: &App, ws: &str) {
    app.metrics.layout_writes.add(1, &[]);
    app.notify_ws("layout", ws);
}

pub async fn put_layout(
    State(app): State<App>,
    ws: crate::kernel::http::Ws,
    CurrentUser(user): CurrentUser,
    Json(moved): Json<MapLayout>,
) -> Result<StatusCode, (StatusCode, String)> {
    let map = app
        .map_of(ws.as_str())
        .await
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))?;
    // Parts of the merged map, and parts that exist only in the shared draft (ADR 0016).
    let draft = map_draft::snapshot(&app.db, ws.as_str(), &map.map)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?;
    let mut known = map.map.node_ids();
    if let Some(preview) = &draft.preview {
        known.extend(preview.nodes.iter().map(|n| n.id.as_str()));
    }
    // All or none: one bad entry refuses the write, so a browser never half-saves a drag.
    for (id, p) in &moved.positions {
        if !known.contains(id.as_str()) {
            app.reject(routes::LAYOUT, reason::UNKNOWN_ID, &user);
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("{id} is not a part in map {}", map.version),
            ));
        }
        if !p.on_canvas() {
            app.reject(routes::LAYOUT, reason::BAD_POSITION, &user);
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("the position of {id} is not within {MAX_COORDINATE} of the origin"),
            ));
        }
    }
    let known: Vec<&str> = known.into_iter().collect();
    store::put(&app.db, ws.as_str(), &moved, &known, &user, Utc::now())
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?;
    tracing::debug!(rubato.user = %user, parts = moved.positions.len(), "layout written");
    written(&app, ws.as_str());
    Ok(StatusCode::NO_CONTENT)
}

pub async fn clear_layout(
    State(app): State<App>,
    ws: crate::kernel::http::Ws,
    CurrentUser(user): CurrentUser,
) -> Result<Json<MapLayout>, StatusCode> {
    let removed = store::clear(&app.db, ws.as_str())
        .await
        .map_err(|e| internal(SLICE, e))?;
    // Clearing moves every part for the whole team, so it is said at info with who did it.
    tracing::info!(rubato.user = %user, parts = removed.positions.len(), "layout cleared");
    written(&app, ws.as_str());
    Ok(Json(removed))
}
