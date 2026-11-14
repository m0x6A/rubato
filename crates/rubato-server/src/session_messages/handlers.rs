//! `GET` and `POST /api/launches/{launch_id}/messages`. Everyone who may log in reads a launch's
//! messages (the conversation is part of the shared picture, ADR 0050); only the launch's paired
//! user sends (ADR 0004, ADR 0051).

use super::dispatch;
use super::store::{self, MessageRow};
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use crate::launch;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rubato_protocol::session_message::routes;
use rubato_protocol::{
    LaunchMessages, SessionMessageKind, SessionMessageRequest, SessionMessageState,
    SessionMessageView, WebResource, capability,
};
use serde_json::{Value, json};
use uuid::Uuid;

/// The reason values of a refused message, as counted in `rubato.web.rejected`.
pub mod reason {
    pub const NOT_YOUR_SESSION: &str = "not_your_session";
    pub const UNKNOWN_LAUNCH: &str = "unknown_launch";
    pub const EMPTY: &str = "empty";
    pub const TOO_LONG: &str = "too_long";
}

type Refused = (StatusCode, Json<Value>);

fn internal(e: anyhow::Error) -> Refused {
    tracing::error!(slice = "session_messages", error = %format!("{e:#}"), "session messages failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "reason": "internal" })),
    )
}

fn refuse(app: &App, status: StatusCode, why: &str, user: &str) -> Refused {
    app.reject(routes::LAUNCH_MESSAGES, why, user);
    (status, Json(json!({ "reason": why })))
}

pub async fn list(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(launch_id): Path<Uuid>,
) -> Result<Json<LaunchMessages>, Refused> {
    let Some(target) = launch::launch_target(&app.db, launch_id)
        .await
        .map_err(internal)?
    else {
        return Err(refuse(
            &app,
            StatusCode::NOT_FOUND,
            reason::UNKNOWN_LAUNCH,
            &user,
        ));
    };
    let messages = store::for_launch(&app.db, launch_id)
        .await
        .map_err(internal)?
        .iter()
        .map(MessageRow::view)
        .collect();
    let now = app.clock.now();
    Ok(Json(LaunchMessages {
        launch_id,
        owner: target.user.clone(),
        companion_can_receive: app.announced(target.companion_id, capability::SESSION_INPUT),
        companion_live: app.companion_live(target.companion_id, capability::SESSION_INPUT, now),
        messages,
    }))
}

pub async fn send(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(launch_id): Path<Uuid>,
    Json(req): Json<SessionMessageRequest>,
) -> Result<Json<SessionMessageView>, Refused> {
    let Some(target) = launch::launch_target(&app.db, launch_id)
        .await
        .map_err(internal)?
    else {
        return Err(refuse(
            &app,
            StatusCode::NOT_FOUND,
            reason::UNKNOWN_LAUNCH,
            &user,
        ));
    };
    // Someone else's session and one that does not exist are both refused; this one says why.
    if !target.user.eq_ignore_ascii_case(&user) {
        return Err(refuse(
            &app,
            StatusCode::FORBIDDEN,
            reason::NOT_YOUR_SESSION,
            &user,
        ));
    }
    if req.text.trim().is_empty() {
        return Err(refuse(
            &app,
            StatusCode::UNPROCESSABLE_ENTITY,
            reason::EMPTY,
            &user,
        ));
    }
    if req.text.chars().count() as u64 > app.config.limits.session_message_max_chars {
        return Err(refuse(
            &app,
            StatusCode::UNPROCESSABLE_ENTITY,
            reason::TOO_LONG,
            &user,
        ));
    }
    let now = app.clock.now();
    let row = MessageRow {
        id: Uuid::new_v4(),
        launch_id,
        session_id: target.session_id.clone(),
        companion_id: target.companion_id,
        owner: target.user.clone(),
        issue: target.issue.clone(),
        kind: SessionMessageKind::Chat {
            text: req.text,
            author: user.clone(),
        },
        author: user.clone(),
        priority: 1,
        state: SessionMessageState::Queued,
        created_at: now,
        pushed_at: None,
        outcome_at: None,
        detail: None,
    };
    store::insert(&app.db, &row).await.map_err(internal)?;
    tracing::info!(rubato.session_message.id = %row.id, rubato.issue = %row.issue, rubato.user = %user, "session message queued");
    dispatch::dispatch(&app, launch_id).await;
    app.notify_resource(WebResource::LaunchMessages { launch_id });
    let now_row = store::get(&app.db, row.id)
        .await
        .map_err(internal)?
        .unwrap_or(row);
    Ok(Json(now_row.view()))
}
