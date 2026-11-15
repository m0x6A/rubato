//! Calling an agent in (ADR 0057, plan 122 §D8). The owner calls their own agent in and it joins
//! at once; a teammate's call is a request, pending until the owner accepts or declines, which
//! its asker may withdraw. The owner, or whoever called it in, dismisses it; it leaves on its own
//! after `Limits.room_agent_idle_secs` without being addressed, or when its session ends.
//!
//! - `POST /api/room/agents` a `CallInRequest`: 201 `Joined` or `Requested`.
//! - `POST /api/room/requests/{id}` a `RequestAnswerBody`: the request as answered.
//! - `DELETE /api/room/agents` a `DismissAgentRequest`: the agent as it left.

use super::agents::{self, AgentRow};
use super::handlers::has_plan;
use super::turns::{notify, room_attr};
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use crate::launch;
use anyhow::Result;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rubato_protocol::room::{reason, routes};
use rubato_protocol::{
    CallInRequest, CallInResponse, DismissAgentRequest, LeftReason, RequestAnswer,
    RequestAnswerBody, RequestState, RoomAgent, RoomKey, RoomRequest, SessionStatus, capability,
};
use serde_json::{Value, json};
use uuid::Uuid;

type Refused = (StatusCode, Json<Value>);

/// A request id that names no request.
pub const UNKNOWN_REQUEST: &str = "unknown_request";
/// A handle that names no agent in the room.
pub const UNKNOWN_AGENT: &str = "unknown_agent";

fn internal(e: anyhow::Error) -> Refused {
    tracing::error!(slice = "rooms", error = %format!("{e:#}"), "room call-in failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "reason": "internal" })),
    )
}

fn refuse(app: &App, route: &str, status: StatusCode, why: &str, user: &str) -> Refused {
    app.reject(route, why, user);
    (status, Json(json!({ "reason": why })))
}

/// Agents that should have left: not addressed for the idle limit, or their session ended.
pub(super) async fn sweep_room(app: &App, room: &RoomKey) -> Result<usize> {
    let rows = agents::agents_in(&app.db, room).await?;
    sweep_rows(app, rows.into_iter().filter(AgentRow::present).collect()).await
}

/// Every room's agents (each poll tick).
pub async fn sweep_agents(app: &App) -> Result<usize> {
    let rows = agents::present_everywhere(&app.db).await?;
    sweep_rows(app, rows).await
}

async fn sweep_rows(app: &App, rows: Vec<AgentRow>) -> Result<usize> {
    let now = app.clock.now();
    let idle = chrono::Duration::seconds(app.config.limits.room_agent_idle_secs as i64);
    let sessions = app.latest_sessions();
    let mut left = 0;
    for a in rows {
        let of_launch: Vec<_> = sessions
            .iter()
            .filter(|s| s.launch_id == Some(a.agent.launch_id))
            .collect();
        let reason = if !of_launch.is_empty()
            && of_launch.iter().all(|s| s.status == SessionStatus::Ended)
        {
            Some(LeftReason::SessionEnded)
        } else if now - a.since() > idle {
            Some(LeftReason::TimedOut)
        } else {
            None
        };
        if let Some(r) = reason
            && agents::leave(&app.db, a.id, r, now).await?
        {
            left += 1;
            tracing::info!(rubato.room = %room_attr(&a.room), handle = %a.agent.handle, reason = ?r, "room agent left");
            notify(app, &a.room);
        }
    }
    Ok(left)
}

fn key_of(repo: &str, number: u64, branch: &str) -> RoomKey {
    RoomKey {
        repo: repo.to_owned(),
        number,
        branch: branch.to_owned(),
    }
}

/// Whether the owner's agent can join now; the refusal when not.
async fn may_join(
    app: &App,
    route: &str,
    user: &str,
    room: &RoomKey,
    owner: &str,
    companion_id: Uuid,
) -> Result<(), Refused> {
    if !app.announced(companion_id, capability::ROOM_TURNS) {
        return Err(refuse(
            app,
            route,
            StatusCode::CONFLICT,
            reason::COMPANION_CANNOT,
            user,
        ));
    }
    sweep_room(app, room).await.map_err(internal)?;
    let present = agents::agents_in(&app.db, room).await.map_err(internal)?;
    if present
        .iter()
        .any(|a| a.present() && a.agent.owner.eq_ignore_ascii_case(owner))
    {
        return Err(refuse(
            app,
            route,
            StatusCode::CONFLICT,
            reason::ALREADY_IN,
            user,
        ));
    }
    Ok(())
}

async fn join(
    app: &App,
    route: &str,
    user: &str,
    room: &RoomKey,
    owner: &str,
    launch_id: Uuid,
    called_by: &str,
) -> Result<RoomAgent, Refused> {
    let agent = RoomAgent {
        handle: rubato_protocol::agent_handle(owner),
        owner: owner.to_owned(),
        launch_id,
        called_by: called_by.to_owned(),
        joined_at: app.clock.now(),
        left_at: None,
        left_reason: None,
        last_turn_at: None,
    };
    match agents::join(&app.db, room, &agent)
        .await
        .map_err(internal)?
    {
        Some(row) => {
            tracing::info!(rubato.room = %room_attr(room), handle = %row.agent.handle, called_by, "room agent joined");
            notify(app, room);
            Ok(row.agent)
        }
        None => Err(refuse(
            app,
            route,
            StatusCode::CONFLICT,
            reason::ALREADY_IN,
            user,
        )),
    }
}

pub async fn call_in(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CallInRequest>,
) -> Result<(StatusCode, Json<CallInResponse>), Refused> {
    let route = routes::ROOM_AGENTS;
    let room = key_of(&req.repo, req.number, &req.branch);
    if !has_plan(&app, &room).await.map_err(internal)? {
        return Err(refuse(
            &app,
            route,
            StatusCode::NOT_FOUND,
            reason::NO_PLAN,
            &user,
        ));
    }
    let target = launch::launch_target(&app.db, req.launch_id)
        .await
        .map_err(internal)?
        .filter(|t| {
            t.issue.repo.eq_ignore_ascii_case(&room.repo)
                && t.issue.number == room.number
                && t.branch.as_deref() == Some(room.branch.as_str())
        });
    let Some(target) = target else {
        return Err(refuse(
            &app,
            route,
            StatusCode::UNPROCESSABLE_ENTITY,
            reason::NOT_THIS_PLAN,
            &user,
        ));
    };
    may_join(&app, route, &user, &room, &target.user, target.companion_id).await?;
    if target.user.eq_ignore_ascii_case(&user) {
        let agent = join(
            &app,
            route,
            &user,
            &room,
            &target.user,
            target.launch_id,
            &user,
        )
        .await?;
        return Ok((StatusCode::CREATED, Json(CallInResponse::Joined { agent })));
    }
    // Someone else's agent: a request the owner answers. One pending request per launch.
    if let Some(open) = agents::requests_in(&app.db, &room)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|q| q.state == RequestState::Pending && q.launch_id == target.launch_id)
    {
        return Ok((
            StatusCode::OK,
            Json(CallInResponse::Requested { request: open }),
        ));
    }
    let request = RoomRequest {
        id: Uuid::new_v4(),
        owner: target.user.clone(),
        launch_id: target.launch_id,
        requested_by: user.clone(),
        at: app.clock.now(),
        state: RequestState::Pending,
        answered_at: None,
    };
    agents::add_request(&app.db, &room, &request)
        .await
        .map_err(internal)?;
    tracing::info!(rubato.room = %room_attr(&room), owner = %request.owner, requested_by = %user, "room call-in requested");
    notify(&app, &room);
    Ok((
        StatusCode::CREATED,
        Json(CallInResponse::Requested { request }),
    ))
}

pub async fn answer(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(request_id): Path<Uuid>,
    Json(body): Json<RequestAnswerBody>,
) -> Result<Json<RoomRequest>, Refused> {
    let route = routes::ROOM_REQUEST;
    let Some((room, request)) = agents::request_of(&app.db, request_id)
        .await
        .map_err(internal)?
    else {
        return Err(refuse(
            &app,
            route,
            StatusCode::NOT_FOUND,
            UNKNOWN_REQUEST,
            &user,
        ));
    };
    let (allowed, to) = match body.answer {
        RequestAnswer::Accept => (
            request.owner.eq_ignore_ascii_case(&user),
            RequestState::Accepted,
        ),
        RequestAnswer::Decline => (
            request.owner.eq_ignore_ascii_case(&user),
            RequestState::Declined,
        ),
        RequestAnswer::Withdraw => (
            request.requested_by.eq_ignore_ascii_case(&user),
            RequestState::Withdrawn,
        ),
    };
    if !allowed {
        return Err(refuse(
            &app,
            route,
            StatusCode::FORBIDDEN,
            reason::NOT_YOURS,
            &user,
        ));
    }
    if request.state != RequestState::Pending {
        return Err(refuse(
            &app,
            route,
            StatusCode::CONFLICT,
            reason::ANSWERED,
            &user,
        ));
    }
    let target = if to == RequestState::Accepted {
        let t = launch::launch_target(&app.db, request.launch_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                refuse(
                    &app,
                    route,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    reason::NOT_THIS_PLAN,
                    &user,
                )
            })?;
        may_join(&app, route, &user, &room, &request.owner, t.companion_id).await?;
        Some(t)
    } else {
        None
    };
    let now = app.clock.now();
    if !agents::answer(&app.db, request_id, to, now)
        .await
        .map_err(internal)?
    {
        return Err(refuse(
            &app,
            route,
            StatusCode::CONFLICT,
            reason::ANSWERED,
            &user,
        ));
    }
    if let Some(t) = target {
        // The person who asked called it in; the owner lent it by accepting.
        join(
            &app,
            route,
            &user,
            &room,
            &request.owner,
            t.launch_id,
            &request.requested_by,
        )
        .await?;
    }
    tracing::info!(rubato.room = %room_attr(&room), request = %request_id, state = ?to, rubato.user = %user, "room call-in answered");
    notify(&app, &room);
    Ok(Json(RoomRequest {
        state: to,
        answered_at: Some(now),
        ..request
    }))
}

pub async fn dismiss(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<DismissAgentRequest>,
) -> Result<Json<RoomAgent>, Refused> {
    let route = routes::ROOM_AGENTS;
    let room = key_of(&req.repo, req.number, &req.branch);
    sweep_room(&app, &room).await.map_err(internal)?;
    let Some(a) = agents::agents_in(&app.db, &room)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|a| a.present() && a.agent.handle.eq_ignore_ascii_case(&req.handle))
    else {
        return Err(refuse(
            &app,
            route,
            StatusCode::NOT_FOUND,
            UNKNOWN_AGENT,
            &user,
        ));
    };
    if !(a.agent.owner.eq_ignore_ascii_case(&user) || a.agent.called_by.eq_ignore_ascii_case(&user))
    {
        return Err(refuse(
            &app,
            route,
            StatusCode::FORBIDDEN,
            reason::NOT_YOURS,
            &user,
        ));
    }
    let now = app.clock.now();
    agents::leave(&app.db, a.id, LeftReason::Dismissed, now)
        .await
        .map_err(internal)?;
    tracing::info!(rubato.room = %room_attr(&room), handle = %a.agent.handle, rubato.user = %user, "room agent dismissed");
    notify(&app, &room);
    Ok(Json(RoomAgent {
        left_at: Some(now),
        left_reason: Some(LeftReason::Dismissed),
        ..a.agent
    }))
}
