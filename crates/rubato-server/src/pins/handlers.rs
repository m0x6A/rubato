//! Pin an issue to a map node or seam, and unpin it.

use super::store;
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::pins::routes;
use rubato_protocol::{IssuePin, PinRequest};

const SLICE: &str = "pins";

pub async fn add_pin(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PinRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let map = app
        .map
        .read()
        .await
        .clone()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))?;
    let id = req.target.id();
    if !map.map.ids().contains(id) {
        app.reject(routes::PINS, "unknown_id", &user);
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{id} is not an id in map {}", map.version),
        ));
    }
    // The kind must be the id's own: a node id pinned as a seam would never show on the map.
    let (kind, of_kind) = if req.target.is_node() {
        ("part", map.map.node_ids().contains(id))
    } else {
        ("seam", map.map.seam_ids().contains(id))
    };
    if !of_kind {
        app.reject(routes::PINS, "kind_mismatch", &user);
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{id} is not a {kind} in map {}", map.version),
        ));
    }
    store::add_pin(
        &app.db,
        &IssuePin {
            issue: req.issue,
            target: req.target,
            pinned_by: user,
            pinned_at: Utc::now(),
        },
    )
    .await
    .map_err(|e| (internal(SLICE, e), String::new()))?;
    app.notify("pin");
    Ok(StatusCode::CREATED)
}

pub async fn remove_pin(
    State(app): State<App>,
    _user: CurrentUser,
    Json(req): Json<PinRequest>,
) -> Result<StatusCode, StatusCode> {
    let removed = store::remove_pin(&app.db, &req.issue, &req.target)
        .await
        .map_err(|e| internal(SLICE, e))?;
    app.notify("pin");
    Ok(if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}
