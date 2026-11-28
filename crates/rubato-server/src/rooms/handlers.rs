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
pub async fn has_plan(app: &App, room: &RoomKey) -> Result<bool> {
    let t = &app.repo_tenant(&room.repo)?;
    let issue = IssueRef::new(room.repo.clone(), room.number);
    if plans::all(&app.db, t)
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
    let t = &app.repo_tenant(&key.repo).map_err(internal)?;
    let messages = store::messages(&app.db, t, &key).await.map_err(internal)?;
    let agents = agents::agents_in(&app.db, t, &key)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|a| a.agent)
        .collect();
    let requests = agents::requests_in(&app.db, t, &key)
        .await
        .map_err(internal)?;
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
    let key = RoomKey {
        repo: req.repo.clone(),
        number: req.number,
        branch: req.branch.clone(),
    };
    match post_as(&app, &user, &key, &req.text, PostSource::Typed).await {
        Ok(p) => Ok((StatusCode::CREATED, Json(p.message))),
        Err(PostRefused::Internal(e)) => Err(internal(e)),
        Err(r) => Err(refuse(&app, r.status(), r.reason(), &user)),
    }
}

/// Where a post came from: typed in the web, or a push-to-talk utterance (ADR 0073) whose id
/// the browser chose, so a repeat of it posts once.
#[derive(Debug, Clone, Copy)]
pub enum PostSource {
    Typed,
    Voice { id: Uuid },
}

/// A post that was made, or found: `new` is false for a repeated utterance id, which made no
/// second turn.
#[derive(Debug)]
pub struct Posted {
    pub message: RoomMessage,
    pub new: bool,
}

/// Why `post_as` posted nothing. The caller counts it under its own route.
#[derive(Debug)]
pub enum PostRefused {
    Empty,
    TooLong,
    NoPlan,
    /// An utterance id already used in another room, or by another person.
    IdReused,
    Internal(anyhow::Error),
}

impl PostRefused {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Empty => reason::EMPTY,
            Self::TooLong => reason::TOO_LONG,
            Self::NoPlan => reason::NO_PLAN,
            Self::IdReused => rubato_protocol::voice::reason::ID_REUSED,
            Self::Internal(_) => "internal",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::Empty | Self::TooLong | Self::IdReused => StatusCode::UNPROCESSABLE_ENTITY,
            Self::NoPlan => StatusCode::NOT_FOUND,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Post `text` in `key`'s room as `user`, and make a turn for each agent present that it
/// addresses (ADR 0057). The body of the web route, shared with voice (ADR 0073): a spoken post
/// carries `spoken` and its utterance id, and makes turns only when it was not posted before.
pub async fn post_as(
    app: &App,
    user: &str,
    key: &RoomKey,
    text: &str,
    source: PostSource,
) -> Result<Posted, PostRefused> {
    let text = text.trim();
    if text.is_empty() {
        return Err(PostRefused::Empty);
    }
    if text.chars().count() as u64 > app.config.limits.room_message_max_chars {
        return Err(PostRefused::TooLong);
    }
    let t = &app.repo_tenant(&key.repo).map_err(PostRefused::Internal)?;
    if !has_plan(app, key).await.map_err(PostRefused::Internal)? {
        return Err(PostRefused::NoPlan);
    }
    let (id, spoken) = match source {
        PostSource::Typed => (Uuid::new_v4(), false),
        PostSource::Voice { id } => (id, true),
    };
    if spoken && let Some(found) = repeat_of(app, user, key, id).await? {
        return Ok(found);
    }
    // The agents it addresses that are in the room now: each gets a turn (ADR 0057).
    super::callin::sweep_room(app, key)
        .await
        .map_err(PostRefused::Internal)?;
    let present = agents::agents_in(&app.db, t, key)
        .await
        .map_err(PostRefused::Internal)?;
    let addressed = turns::person_turns(text, &present);
    let turn_id = (!addressed.is_empty()).then(Uuid::new_v4);
    let m = RoomMessage {
        id,
        author_kind: RoomAuthorKind::Person,
        author: user.to_owned(),
        text: text.to_owned(),
        at: app.clock.now(),
        notice: None,
        addressed: addressed
            .iter()
            .map(|a| a.agent.handle.to_ascii_lowercase())
            .collect(),
        turn_id,
        spoken,
    };
    // Stored already (the same utterance id racing itself): no second turn.
    if !store::insert(&app.db, t, key, &m)
        .await
        .map_err(PostRefused::Internal)?
    {
        return match repeat_of(app, user, key, id).await? {
            Some(found) => Ok(found),
            None => Err(PostRefused::IdReused),
        };
    }
    if let Some(turn_id) = turn_id {
        for a in &addressed {
            turns::dispatch_turn(app, key, a, &m, turn_id, user)
                .await
                .map_err(PostRefused::Internal)?;
        }
    }
    tracing::info!(
        rubato.issue = %IssueRef::new(key.repo.clone(), key.number),
        rubato.branch = %key.branch,
        rubato.user = %user,
        spoken,
        "room message posted"
    );
    app.notify_resource(WebResource::Room {
        repo: key.repo.clone(),
        number: key.number,
        branch: key.branch.clone(),
    });
    Ok(Posted {
        message: m,
        new: true,
    })
}

/// The message an utterance id already posted: `None` when it posted nothing, refused when the
/// id is in another room or another person's.
async fn repeat_of(
    app: &App,
    user: &str,
    key: &RoomKey,
    id: Uuid,
) -> Result<Option<Posted>, PostRefused> {
    let t = &app.repo_tenant(&key.repo).map_err(PostRefused::Internal)?;
    match store::by_id(&app.db, t, id)
        .await
        .map_err(PostRefused::Internal)?
    {
        None => Ok(None),
        Some((room, m)) if room == *key && m.author.eq_ignore_ascii_case(user) => {
            Ok(Some(Posted {
                message: m,
                new: false,
            }))
        }
        Some(_) => Err(PostRefused::IdReused),
    }
}

/// A message of a room by id, with its room (ADR 0073: the speech of an agent's reply).
pub async fn message(
    app: &App,
    t: &crate::kernel::tenancy::TenantId,
    id: Uuid,
) -> Result<Option<(RoomKey, RoomMessage)>> {
    store::by_id(&app.db, t, id).await
}

/// Whether turn `turn_id` in `room` was made by a person's spoken message (ADR 0073).
pub async fn turn_was_spoken(app: &App, room: &RoomKey, turn_id: Uuid) -> Result<bool> {
    let t = &app.repo_tenant(&room.repo)?;
    store::turn_was_spoken(&app.db, t, room, turn_id).await
}

/// The handles of the agents present in `room` now, after the idle sweep.
pub async fn agents_present(app: &App, room: &RoomKey) -> Result<Vec<String>> {
    super::callin::sweep_room(app, room).await?;
    let t = &app.repo_tenant(&room.repo)?;
    Ok(agents::agents_in(&app.db, t, room)
        .await?
        .into_iter()
        .filter(|a| a.present())
        .map(|a| a.agent.handle.to_ascii_lowercase())
        .collect())
}
