//! Device-code pairing endpoints (ADR 0011).

use crate::auth::CurrentUser;
use crate::db;
use crate::state::App;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rand::RngCore;
use rubato_protocol::pairing::{
    PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse,
};
use rubato_protocol::web::{PairApproveRequest, PairingInfo};
use uuid::Uuid;

/// Eight characters from an alphabet without look-alikes, as `XXXX-XXXX`.
pub fn user_code() -> String {
    const ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";
    let mut bytes = [0u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    let chars: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub async fn start(
    State(app): State<App>,
    Json(req): Json<PairStartRequest>,
) -> Result<Json<PairStartResponse>, StatusCode> {
    let device_code = random_token();
    let code = user_code();
    db::create_pairing_code(
        &app.db,
        &req,
        &device_code,
        &code,
        app.config.pairing_ttl_secs,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "pairing start");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
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
    db::poll_pairing_code(&app.db, &req.device_code)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(error = %e, "pairing poll");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// The logged-in human approves a code shown by their companion.
pub async fn approve(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PairApproveRequest>,
) -> Result<Json<PairingInfo>, StatusCode> {
    let token = random_token();
    let code = req.user_code.trim().to_ascii_uppercase();
    let companion_id = db::approve_pairing_code(&app.db, &code, &user, &token)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "pairing approve");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let Some(companion_id) = companion_id else {
        return Err(StatusCode::NOT_FOUND);
    };
    tracing::info!(%companion_id, %user, "pairing approved");
    app.notify("pairing");
    let rows = db::list_pairings(&app.db, Some(&user))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    rows.into_iter()
        .find(|r| r.companion_id == companion_id)
        .map(|r| Json(to_info(&app, r)))
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)
}

pub fn to_info(app: &App, r: db::PairingRow) -> PairingInfo {
    let connected = app.companions.lock().unwrap().contains_key(&r.companion_id);
    PairingInfo {
        companion_id: r.companion_id,
        user: r.user,
        hostname: r.hostname,
        created_at: r.created_at,
        last_seen: r.last_seen,
        revoked_at: r.revoked_at,
        connected,
    }
}

/// The caller's own pairings.
pub async fn list(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<PairingInfo>>, StatusCode> {
    let rows = db::list_pairings(&app.db, Some(&user))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(rows.into_iter().map(|r| to_info(&app, r)).collect()))
}

pub async fn revoke(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(companion_id): Path<Uuid>,
) -> StatusCode {
    match db::revoke_pairing(&app.db, companion_id, &user).await {
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
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn user_codes_are_shaped() {
        let c = super::user_code();
        assert_eq!(c.len(), 9);
        assert_eq!(&c[4..5], "-");
        assert!(
            c.chars()
                .filter(|c| *c != '-')
                .all(|c| "BCDFGHJKLMNPQRSTVWXZ23456789".contains(c))
        );
    }
}
