//! Pin an issue to a map node or seam, and unpin it.

use super::map_lines::{check_repo, check_target};
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
        .map_of(ws.key())
        .await
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))?;
    // The repo first: an issue of a repo no map names would be read with the App's token and
    // shown to every member (issue #163).
    if let Err((reason, msg)) = check_repo(&map.map, &map.version, &req.issue.repo)
        .and_then(|()| check_target(&map.map, &map.version, &req.target))
    {
        app.reject(routes::PINS, reason, &user);
        return Err((StatusCode::UNPROCESSABLE_ENTITY, msg));
    }
    store::add_pin(
        &app.db,
        ws.key(),
        &IssuePin {
            issue: req.issue,
            target: req.target,
            pinned_by: user,
            pinned_at: Utc::now(),
        },
    )
    .await
    .map_err(|e| (internal(SLICE, e), String::new()))?;
    app.notify_in(ws.key().tenant(), "pin");
    Ok(StatusCode::CREATED)
}

pub async fn remove_pin(
    State(app): State<App>,
    ws: crate::kernel::http::Ws,
    _user: CurrentUser,
    Json(req): Json<PinRequest>,
) -> Result<StatusCode, StatusCode> {
    let removed = store::remove_pin(&app.db, ws.key(), &req.issue, &req.target)
        .await
        .map_err(|e| internal(SLICE, e))?;
    app.notify_in(ws.key().tenant(), "pin");
    Ok(if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}
