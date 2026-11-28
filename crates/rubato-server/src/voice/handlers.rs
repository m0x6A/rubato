//! The voice routes (plan 146 design §2–§4). Every one answers 404 `voice_off` while
//! `RUBATO_VOICE` is off, and counts each refusal on `rubato.web.rejected` under its own route.
//! Cheap checks come before any provider call.

use super::gate::{self, CallError};
use super::{cut, token};
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use crate::rooms;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::voice::{join_reason, limits, reason, routes};
use rubato_protocol::{
    RoomAuthorKind, RoomKey, RoomMessage, UtteranceQuery, VoiceJoinFailed, VoiceToken,
    VoiceTokenRequest,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

type Refused = (StatusCode, Json<Value>);

/// The provider failed: 502 `provider`, with what kind of failure it was.
fn provider_failed(
    app: &App,
    route: &str,
    e: &crate::kernel::speech::SpeechError,
    user: &str,
) -> Refused {
    app.reject(route, reason::PROVIDER, user);
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({ "reason": reason::PROVIDER, "outcome": e.outcome() })),
    )
}

fn refuse(app: &App, route: &str, status: StatusCode, why: &str, user: &str) -> Refused {
    app.reject(route, why, user);
    (status, Json(json!({ "reason": why })))
}

fn internal(e: anyhow::Error) -> Refused {
    tracing::error!(slice = "voice", error = %format!("{e:#}"), "voice failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "reason": "internal" })),
    )
}

fn room_attr(k: &RoomKey) -> String {
    format!("{}#{}@{}", k.repo, k.number, k.branch)
}

/// Voice is on: 404 `voice_off` otherwise.
fn voice_on(app: &App, route: &str, user: &str) -> Result<(), Refused> {
    if app.config.voice.is_none() {
        return Err(refuse(
            app,
            route,
            StatusCode::NOT_FOUND,
            reason::VOICE_OFF,
            user,
        ));
    }
    Ok(())
}

/// `POST /api/room/voice/token`: a LiveKit join token for the caller in a plan's room.
pub async fn token(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<VoiceTokenRequest>,
) -> Result<Json<VoiceToken>, Refused> {
    let route = routes::VOICE_TOKEN;
    voice_on(&app, route, &user)?;
    let count = |outcome: &'static str| {
        app.metrics
            .voice_tokens
            .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
    };
    let Some(lk) = app.config.voice.as_ref().and_then(|v| v.livekit.clone()) else {
        count("refused");
        return Err(refuse(
            &app,
            route,
            StatusCode::CONFLICT,
            reason::NO_LIVEKIT,
            &user,
        ));
    };
    let key = RoomKey {
        repo: req.repo,
        number: req.number,
        branch: req.branch,
    };
    if !rooms::has_plan(&app, &key).await.map_err(internal)? {
        count("refused");
        return Err(refuse(
            &app,
            route,
            StatusCode::NOT_FOUND,
            rubato_protocol::room::reason::NO_PLAN,
            &user,
        ));
    }
    let suffix = Uuid::new_v4().simple().to_string()[..8].to_owned();
    let t = token::mint(&lk, &user, &key, app.clock.now(), &suffix);
    let _s = tracing::info_span!(
        span::VOICE_TOKEN,
        rubato.room = %room_attr(&key),
        rubato.user = %user,
        rubato.outcome = "minted"
    )
    .entered();
    count("minted");
    tracing::info!(rubato.room = %room_attr(&key), rubato.user = %user, livekit_room = %t.room, "voice token minted");
    Ok(Json(t))
}

/// `POST /api/room/voice/utterance`: one push-to-talk utterance, transcribed and posted in the
/// room as the caller, marked `spoken`. 201 when posted, 200 for a repeat of the same id.
pub async fn utterance(
    State(app): State<App>,
    scope: crate::kernel::tenancy::TenantScope,
    CurrentUser(user): CurrentUser,
    Query(q): Query<UtteranceQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<RoomMessage>), Refused> {
    let route = routes::VOICE_UTTERANCE;
    voice_on(&app, route, &user)?;
    let Some(speech) = app.speech() else {
        return Err(refuse(
            &app,
            route,
            StatusCode::CONFLICT,
            reason::NO_SPEECH,
            &user,
        ));
    };
    let key = RoomKey {
        repo: q.repo.clone(),
        number: q.number,
        branch: q.branch.clone(),
    };
    // Cheap checks first: none of these costs a transcription.
    if !rooms::has_plan(&app, &key).await.map_err(internal)? {
        return Err(refuse(
            &app,
            route,
            StatusCode::NOT_FOUND,
            rubato_protocol::room::reason::NO_PLAN,
            &user,
        ));
    }
    let to =
        q.to.as_deref()
            .map(|h| h.trim().trim_start_matches('@').to_ascii_lowercase());
    if let Some(h) = &to {
        let present = rooms::agents_present(&app, &key).await.map_err(internal)?;
        if !present.contains(h) {
            return Err(refuse(
                &app,
                route,
                StatusCode::UNPROCESSABLE_ENTITY,
                reason::UNKNOWN_AGENT,
                &user,
            ));
        }
    }
    let audio = match axum::body::to_bytes(body, limits::UTTERANCE_MAX_BYTES).await {
        Ok(b) => b.to_vec(),
        Err(_) => {
            return Err(refuse(
                &app,
                route,
                StatusCode::PAYLOAD_TOO_LARGE,
                rubato_protocol::room::reason::TOO_LONG,
                &user,
            ));
        }
    };
    // A repeat of a posted utterance answers the message, with no second transcription.
    if let Some((room, m)) = rooms::message(&app, scope.tenant(), q.id)
        .await
        .map_err(internal)?
    {
        if room == key && m.author.eq_ignore_ascii_case(&user) {
            return Ok((StatusCode::OK, Json(m)));
        }
        return Err(refuse(
            &app,
            route,
            StatusCode::UNPROCESSABLE_ENTITY,
            reason::ID_REUSED,
            &user,
        ));
    }
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/webm")
        .to_owned();
    let started = std::time::Instant::now();
    let transcript = match gate::transcribe(&app, &speech, &room_attr(&key), audio, &mime).await {
        Ok(t) => t,
        Err(CallError::Busy) => {
            return Err(refuse(
                &app,
                route,
                StatusCode::SERVICE_UNAVAILABLE,
                reason::BUSY,
                &user,
            ));
        }
        Err(CallError::Provider(e)) => return Err(provider_failed(&app, route, &e, &user)),
    };
    let stt_ms = started.elapsed().as_millis() as u64;
    if transcript.is_empty() {
        gate::record_empty(&app, &speech);
        return Err(refuse(
            &app,
            route,
            StatusCode::UNPROCESSABLE_ENTITY,
            rubato_protocol::room::reason::EMPTY,
            &user,
        ));
    }
    let text = match &to {
        Some(h) => format!("@{h} {transcript}"),
        None => transcript,
    };
    match rooms::post_as(
        &app,
        &user,
        &key,
        &text,
        rooms::PostSource::Voice { id: q.id },
    )
    .await
    {
        Ok(p) => {
            tracing::info!(
                rubato.room = %room_attr(&key),
                rubato.user = %user,
                message_id = %p.message.id,
                turn_id = ?p.message.turn_id,
                stt_ms,
                "spoken message posted"
            );
            let status = if p.new {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            Ok((status, Json(p.message)))
        }
        Err(rooms::PostRefused::Internal(e)) => Err(internal(e)),
        Err(r) => Err(refuse(&app, route, r.status(), r.reason(), &user)),
    }
}

/// The `{message_id}` of the speech route. Named, because the path also carries `{tenant}`.
#[derive(serde::Deserialize)]
pub struct MessageParam {
    message_id: Uuid,
}

impl From<Uuid> for MessageParam {
    fn from(message_id: Uuid) -> Self {
        Self { message_id }
    }
}

/// `GET /api/room/voice/speech/{message_id}`: the speech of an agent's reply to a spoken turn.
/// Replies to typed turns, and people's messages, are never sent to the provider.
pub async fn speech(
    State(app): State<App>,
    scope: crate::kernel::tenancy::TenantScope,
    CurrentUser(user): CurrentUser,
    Path(MessageParam { message_id }): Path<MessageParam>,
) -> Result<Response, Refused> {
    let route = routes::VOICE_SPEECH;
    voice_on(&app, route, &user)?;
    let Some(speech) = app.speech() else {
        return Err(refuse(
            &app,
            route,
            StatusCode::CONFLICT,
            reason::NO_SPEECH,
            &user,
        ));
    };
    let not_spoken =
        |app: &App| refuse(app, route, StatusCode::NOT_FOUND, reason::NOT_SPOKEN, &user);
    let Some((room, m)) = rooms::message(&app, scope.tenant(), message_id)
        .await
        .map_err(internal)?
    else {
        return Err(not_spoken(&app));
    };
    let Some(turn_id) = m.turn_id.filter(|_| m.author_kind == RoomAuthorKind::Agent) else {
        return Err(not_spoken(&app));
    };
    if !rooms::turn_was_spoken(&app, &room, turn_id)
        .await
        .map_err(internal)?
    {
        return Err(not_spoken(&app));
    }
    let said = cut::spoken_cut(&m.text);
    match gate::clip(&app, &speech, &room_attr(&room), message_id, &said).await {
        Ok(clip) => {
            let mut res = clip.bytes.into_response();
            if let Ok(v) = HeaderValue::from_str(&clip.mime) {
                res.headers_mut().insert(header::CONTENT_TYPE, v);
            }
            res.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            Ok(res)
        }
        Err(CallError::Busy) => Err(refuse(
            &app,
            route,
            StatusCode::SERVICE_UNAVAILABLE,
            reason::BUSY,
            &user,
        )),
        Err(CallError::Provider(e)) => Err(provider_failed(&app, route, &e, &user)),
    }
}

/// When each person last reported a failed join for a room: at most one counted per minute.
#[derive(Default)]
struct Reported(Mutex<HashMap<(String, String), chrono::DateTime<chrono::Utc>>>);

/// `POST /api/room/voice/join-failed`: the browser could not join LiveKit. Counted by reason and
/// logged; nothing is stored. 204.
pub async fn join_failed(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    body: Body,
) -> Result<StatusCode, Refused> {
    let route = routes::VOICE_JOIN_FAILED;
    voice_on(&app, route, &user)?;
    let Ok(bytes) = axum::body::to_bytes(body, limits::JOIN_FAILED_MAX_BYTES).await else {
        return Err(refuse(
            &app,
            route,
            StatusCode::PAYLOAD_TOO_LARGE,
            reason::TOO_LARGE,
            &user,
        ));
    };
    let Ok(r) = serde_json::from_slice::<VoiceJoinFailed>(&bytes) else {
        return Err(refuse(
            &app,
            route,
            StatusCode::BAD_REQUEST,
            "bad_body",
            &user,
        ));
    };
    let key = RoomKey {
        repo: r.repo,
        number: r.number,
        branch: r.branch,
    };
    let why = join_reason::normalise(&r.reason);
    let now = app.clock.now();
    let seen = app.slots.get::<Reported>();
    {
        let mut m = seen.0.lock().unwrap();
        m.retain(|_, at| now - *at < chrono::Duration::minutes(1));
        let k = (user.to_ascii_lowercase(), room_attr(&key));
        if m.contains_key(&k) {
            return Ok(StatusCode::NO_CONTENT);
        }
        m.insert(k, now);
    }
    app.metrics
        .voice_join_failed
        .add(1, &[KeyValue::new(attr::DROP_REASON, why)]);
    tracing::warn!(rubato.room = %room_attr(&key), rubato.user = %user, rubato.reason = why, "voice join failed");
    Ok(StatusCode::NO_CONTENT)
}
