//! `GET` and `POST /api/launches/{launch_id}/messages`. Everyone who may log in reads a launch's
//! messages (the conversation is part of the shared picture, ADR 0050); only the launch's paired
//! user sends (ADR 0004, ADR 0051).

use super::dispatch;
use super::store::{self, MessageRow};
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use crate::kernel::tenancy::TenantScope;
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
    /// A stop for a companion that is not live: never queued (ADR 0062).
    pub const COMPANION_OFFLINE: &str = "companion_offline";
    /// A stop for a companion that did not announce `session_stop`.
    pub const CANNOT_STOP: &str = "companion_cannot_stop";
}

/// The `{launch_id}` of a tenant route. Named, because the path also carries `{tenant}` (ADR 0089).
#[derive(serde::Deserialize)]
pub struct LaunchParam {
    launch_id: Uuid,
}

impl From<Uuid> for LaunchParam {
    fn from(launch_id: Uuid) -> Self {
        Self { launch_id }
    }
}

/// `POST /api/launches/{launch_id}/stop` (ADR 0062, plan 122 §D14). Only the owner; never queued
/// (409 when the companion is not live, so a stop never reaches a later session); pushed at once,
/// past any message in flight; the companion skips only its idle check.
pub async fn stop(
    State(app): State<App>,
    scope: TenantScope,
    CurrentUser(user): CurrentUser,
    Path(LaunchParam { launch_id }): Path<LaunchParam>,
) -> Result<Json<SessionMessageView>, Refused> {
    if launch_in(&app, &scope, launch_id)
        .await
        .map_err(internal)?
        .is_none()
    {
        app.reject(routes::LAUNCH_STOP, reason::UNKNOWN_LAUNCH, &user);
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "reason": reason::UNKNOWN_LAUNCH })),
        ));
    }
    match stop_as(&app, &user, launch_id, None).await {
        Ok(v) => Ok(Json(v)),
        Err(StopRefused::Internal(e)) => Err(internal(e)),
        Err(StopRefused::No(status, why)) => {
            app.reject(routes::LAUNCH_STOP, why, &user);
            Err((status, Json(json!({ "reason": why }))))
        }
    }
}

/// The launch with this id in the tenant the request acts in. One of another tenant is unknown
/// here, exactly like one that does not exist (ADR 0089, design §4.2).
async fn launch_in(
    app: &App,
    scope: &TenantScope,
    launch_id: Uuid,
) -> anyhow::Result<Option<launch::LaunchTarget>> {
    Ok(launch::launch_target(&app.db, launch_id)
        .await?
        .filter(|l| &l.tenant == scope.tenant()))
}

/// Why `stop_as` sent nothing: a status and a `reason`, or a store error.
#[derive(Debug)]
pub enum StopRefused {
    No(StatusCode, &'static str),
    Internal(anyhow::Error),
}

/// A stop for `launch_id`, as its owner `user`, sent past the queue (ADR 0062). `via` is the
/// owner's lead agent that asked (ADR 0072); the web route passes `None`. The caller counts a
/// refusal under its own route.
pub async fn stop_as(
    app: &App,
    user: &str,
    launch_id: Uuid,
    via: Option<Uuid>,
) -> Result<SessionMessageView, StopRefused> {
    let no = |status, why| StopRefused::No(status, why);
    let Some(target) = launch::launch_target(&app.db, launch_id)
        .await
        .map_err(StopRefused::Internal)?
    else {
        return Err(no(StatusCode::NOT_FOUND, reason::UNKNOWN_LAUNCH));
    };
    if !target.user.eq_ignore_ascii_case(user) {
        return Err(no(StatusCode::FORBIDDEN, reason::NOT_YOUR_SESSION));
    }
    if !app.announced(target.companion_id, capability::SESSION_STOP) {
        return Err(no(StatusCode::CONFLICT, reason::CANNOT_STOP));
    }
    let now = app.clock.now();
    if !app.companion_live(target.companion_id, capability::SESSION_STOP, now) {
        return Err(no(StatusCode::CONFLICT, reason::COMPANION_OFFLINE));
    }
    let row = MessageRow {
        tenant: target.tenant.clone(),
        id: Uuid::new_v4(),
        launch_id,
        session_id: target.session_id.clone(),
        companion_id: target.companion_id,
        owner: target.user.clone(),
        issue: target.issue.clone(),
        kind: SessionMessageKind::Stop {
            requested_by: user.to_owned(),
        },
        author: user.to_owned(),
        priority: 0,
        // Recorded as pushed: it bypasses the queue, so dispatch never pushes it again.
        state: SessionMessageState::Sent,
        created_at: now,
        pushed_at: Some(now),
        outcome_at: None,
        detail: None,
        via_launch: via,
    };
    store::insert(&app.db, &row)
        .await
        .map_err(StopRefused::Internal)?;
    if !app.send_to_companion(target.companion_id, dispatch::wire(&row)) {
        let _ = store::settle(
            &app.db,
            row.id,
            target.companion_id,
            SessionMessageState::Failed,
            Some("companion went away"),
            now,
        )
        .await;
        return Err(no(StatusCode::CONFLICT, reason::COMPANION_OFFLINE));
    }
    tracing::info!(rubato.session_message.id = %row.id, rubato.issue = %row.issue, rubato.user = %user, via_launch = ?via, "session stop sent");
    app.notify_resource(&target.tenant, WebResource::LaunchMessages { launch_id });
    Ok(row.view())
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
    scope: TenantScope,
    CurrentUser(user): CurrentUser,
    Path(LaunchParam { launch_id }): Path<LaunchParam>,
) -> Result<Json<LaunchMessages>, Refused> {
    let Some(target) = launch_in(&app, &scope, launch_id).await.map_err(internal)? else {
        return Err(refuse(
            &app,
            StatusCode::NOT_FOUND,
            reason::UNKNOWN_LAUNCH,
            &user,
        ));
    };
    let messages = store::for_launch(&app.db, &target.tenant, launch_id)
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
    scope: TenantScope,
    CurrentUser(user): CurrentUser,
    Path(LaunchParam { launch_id }): Path<LaunchParam>,
    Json(req): Json<SessionMessageRequest>,
) -> Result<Json<SessionMessageView>, Refused> {
    let Some(target) = launch_in(&app, &scope, launch_id).await.map_err(internal)? else {
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
        tenant: target.tenant.clone(),
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
        priority: super::enqueue::CHAT_PRIORITY,
        state: SessionMessageState::Queued,
        created_at: now,
        pushed_at: None,
        outcome_at: None,
        detail: None,
        via_launch: None,
    };
    store::insert(&app.db, &row).await.map_err(internal)?;
    tracing::info!(rubato.session_message.id = %row.id, rubato.issue = %row.issue, rubato.user = %user, "session message queued");
    dispatch::dispatch(&app, launch_id).await;
    app.notify_resource(&target.tenant, WebResource::LaunchMessages { launch_id });
    let now_row = store::get(&app.db, row.id)
        .await
        .map_err(internal)?
        .unwrap_or(row);
    Ok(Json(now_row.view()))
}
