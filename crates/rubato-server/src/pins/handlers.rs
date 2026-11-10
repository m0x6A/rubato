//! Pin an issue to a map node or seam, and unpin it.

use super::store;
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::IssuePin;
use rubato_protocol::web::PinRequest;

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
    if !map.map.ids().contains(req.target.id()) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{} is not an id in map {}", req.target.id(), map.version),
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
