//! Pin an issue to a map node or seam, and unpin it.

use super::map_lines::check_target;
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
    ws: crate::kernel::http::Ws,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PinRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let map = app
        .map_of(ws.as_str())
        .await
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))?;
    if let Err((reason, msg)) = check_target(&map.map, &map.version, &req.target) {
        app.reject(routes::PINS, reason, &user);
        return Err((StatusCode::UNPROCESSABLE_ENTITY, msg));
    }
    store::add_pin(
        &app.db,
        ws.as_str(),
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
    ws: crate::kernel::http::Ws,
    _user: CurrentUser,
    Json(req): Json<PinRequest>,
) -> Result<StatusCode, StatusCode> {
    let removed = store::remove_pin(&app.db, ws.as_str(), &req.issue, &req.target)
        .await
        .map_err(|e| internal(SLICE, e))?;
    app.notify("pin");
    Ok(if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}
