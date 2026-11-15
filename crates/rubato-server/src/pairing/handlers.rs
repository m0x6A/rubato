//! Device-code pairing endpoints (ADR 0011).

use super::codes::{random_token, user_code};
use super::store::{self, Approval, PairingRow};
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rubato_protocol::pairing::{
    PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse, routes,
};
use rubato_protocol::{PairApproveRequest, PairingInfo};
use uuid::Uuid;

const SLICE: &str = "pairing";

pub async fn start(
    State(app): State<App>,
    Json(req): Json<PairStartRequest>,
) -> Result<Json<PairStartResponse>, StatusCode> {
    let device_code = random_token();
    let code = user_code();
    store::create_pairing_code(
        &app.db,
        &req,
        &device_code,
        &code,
        app.config.pairing_ttl_secs,
    )
    .await
    .map_err(|e| internal(SLICE, e))?;
    tracing::info!(companion_id = %req.companion_id, hostname = %req.hostname, "pairing started");
    Ok(Json(PairStartResponse {
        device_code,
        user_code: code,
        verification_url: format!("{}/pair", app.config.public_url.trim_end_matches('/')),
        interval_secs: 3,
        expires_in_secs: app.config.pairing_ttl_secs as u64,
    }))
}

pub async fn poll(
    State(app): State<App>,
    Json(req): Json<PairPollRequest>,
) -> Result<Json<PairPollResponse>, StatusCode> {
    store::poll_pairing_code(&app.db, &req.device_code)
        .await
        .map(Json)
        .map_err(|e| internal(SLICE, e))
}

/// The reason values of a refused approval, as counted in `rubato.web.rejected`.
pub mod reason {
    /// The companion id is paired to another person (#160).
    pub const COMPANION_TAKEN: &str = "companion_taken";
}

type Refusal = (StatusCode, String);

/// The logged-in human approves a code shown by their companion.
pub async fn approve(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PairApproveRequest>,
) -> Result<Json<PairingInfo>, Refusal> {
    let fail = |e| (internal(SLICE, e), String::new());
    let token = random_token();
    let code = req.user_code.trim().to_ascii_uppercase();
    let companion_id = match store::approve_pairing_code(&app.db, &code, &user, &token)
        .await
        .map_err(fail)?
    {
        Approval::Approved(id) => id,
        Approval::NotFound => return Err((StatusCode::NOT_FOUND, String::new())),
        Approval::Taken(id) => {
            tracing::info!(companion_id = %id, %user, "pairing refused: companion paired to another person");
            app.reject(routes::PAIR_APPROVE, reason::COMPANION_TAKEN, &user);
            return Err((
                StatusCode::CONFLICT,
                "This companion is paired to another person. Pair from your own machine, or remove its companion.json to give it a new id.".into(),
            ));
        }
    };
    tracing::info!(%companion_id, %user, "pairing approved");
    app.notify("pairing");
    let rows = store::list_pairings(&app.db, Some(&user))
        .await
        .map_err(fail)?;
    rows.into_iter()
        .find(|r| r.companion_id == companion_id)
        .map(|r| Json(to_info(&app, r)))
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, String::new()))
}

pub fn to_info(app: &App, r: PairingRow) -> PairingInfo {
    let connected = app.companions.lock().unwrap().contains_key(&r.companion_id);
    PairingInfo {
        companion_id: r.companion_id,
        user: r.user,
        hostname: r.hostname,
        created_at: r.created_at,
        last_seen: r.last_seen,
        revoked_at: r.revoked_at,
        connected,
        client: r.client,
    }
}

/// The caller's own pairings.
pub async fn list(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<PairingInfo>>, StatusCode> {
    let rows = store::list_pairings(&app.db, Some(&user))
        .await
        .map_err(|e| internal(SLICE, e))?;
    Ok(Json(rows.into_iter().map(|r| to_info(&app, r)).collect()))
}

pub async fn revoke(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(companion_id): Path<Uuid>,
) -> StatusCode {
    match store::revoke_pairing(&app.db, companion_id, &user).await {
        Ok(true) => {
            if let Some(c) = app.companions.lock().unwrap().remove(&companion_id) {
                let _ = c.tx.send(rubato_protocol::ServerMessage::Error {
                    code: "auth".into(),
                    message: "pairing revoked".into(),
                });
            }
            app.notify("pairing");
            StatusCode::NO_CONTENT
        }
        Ok(false) => StatusCode::NOT_FOUND,
        Err(e) => internal(SLICE, e),
    }
}
