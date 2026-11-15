//! `GET /api/room?repo=&number=&branch=` and `POST /api/room/messages`. Team routes: a room is a
//! plan's, not one workspace's. Everyone who may log in reads a room and posts in it as
//! themselves (ADR 0022, ADR 0057); a post needs a plan or a draft on that branch.

use super::{agents, store, turns};
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use crate::{plan_drafts, plans};
use anyhow::Result;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use rubato_protocol::room::{reason, routes};
use rubato_protocol::{
    IssueRef, RoomAuthorKind, RoomKey, RoomMessage, RoomPostRequest, RoomView, WebResource,
};
use serde_json::{Value, json};
use uuid::Uuid;

type Refused = (StatusCode, Json<Value>);

fn internal(e: anyhow::Error) -> Refused {
    tracing::error!(slice = "rooms", error = %format!("{e:#}"), "rooms failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "reason": "internal" })),
    )
}

fn refuse(app: &App, status: StatusCode, why: &str, user: &str) -> Refused {
    app.reject(routes::ROOM_MESSAGES, why, user);
    (status, Json(json!({ "reason": why })))
}

/// A plan file or a draft on that branch of the issue: the room has a plan to be about.
pub(super) async fn has_plan(app: &App, room: &RoomKey) -> Result<bool> {
    let issue = IssueRef::new(room.repo.clone(), room.number);
    if plans::all(&app.db)
        .await?
        .iter()
        .any(|p| p.issue == issue && p.branch == room.branch)
    {
        return Ok(true);
    }
    Ok(plan_drafts::on_branch(app, &issue, &room.branch)
        .await?
        .is_some())
}

pub async fn room(
    State(app): State<App>,
    CurrentUser(_user): CurrentUser,
    Query(key): Query<RoomKey>,
) -> Result<Json<RoomView>, Refused> {
    super::callin::sweep_room(&app, &key)
        .await
        .map_err(internal)?;
    let messages = store::messages(&app.db, &key).await.map_err(internal)?;
    let agents = agents::agents_in(&app.db, &key)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|a| a.agent)
        .collect();
    let requests = agents::requests_in(&app.db, &key).await.map_err(internal)?;
    Ok(Json(RoomView {
        repo: key.repo,
        number: key.number,
        branch: key.branch,
        messages,
        agents,
        requests,
    }))
}

pub async fn post(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RoomPostRequest>,
) -> Result<(StatusCode, Json<RoomMessage>), Refused> {
    let text = req.text.trim();
    if text.is_empty() {
        return Err(refuse(
            &app,
            StatusCode::UNPROCESSABLE_ENTITY,
            reason::EMPTY,
            &user,
        ));
    }
    if text.chars().count() as u64 > app.config.limits.room_message_max_chars {
        return Err(refuse(
            &app,
            StatusCode::UNPROCESSABLE_ENTITY,
            reason::TOO_LONG,
            &user,
        ));
    }
    let key = RoomKey {
        repo: req.repo.clone(),
        number: req.number,
        branch: req.branch.clone(),
    };
    if !has_plan(&app, &key).await.map_err(internal)? {
        return Err(refuse(&app, StatusCode::NOT_FOUND, reason::NO_PLAN, &user));
    }
    // The agents it addresses that are in the room now: each gets a turn (ADR 0057).
    super::callin::sweep_room(&app, &key)
        .await
        .map_err(internal)?;
    let present = agents::agents_in(&app.db, &key).await.map_err(internal)?;
    let addressed = turns::person_turns(text, &present);
    let turn_id = (!addressed.is_empty()).then(Uuid::new_v4);
    let m = RoomMessage {
        id: Uuid::new_v4(),
        author_kind: RoomAuthorKind::Person,
        author: user.clone(),
        text: text.to_owned(),
        at: app.clock.now(),
        notice: None,
        addressed: addressed
            .iter()
            .map(|a| a.agent.handle.to_ascii_lowercase())
            .collect(),
        turn_id,
    };
    store::insert(&app.db, &key, &m).await.map_err(internal)?;
    if let Some(turn_id) = turn_id {
        for a in &addressed {
            turns::dispatch_turn(&app, &key, a, &m, turn_id, &user)
                .await
                .map_err(internal)?;
        }
    }
    tracing::info!(
        rubato.issue = %IssueRef::new(key.repo.clone(), key.number),
        rubato.branch = %key.branch,
        rubato.user = %user,
        "room message posted"
    );
    app.notify_resource(WebResource::Room {
        repo: key.repo,
        number: key.number,
        branch: key.branch,
    });
    Ok((StatusCode::CREATED, Json(m)))
}
