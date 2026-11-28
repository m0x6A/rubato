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
    PAIR_FIELD_MAX_CHARS, PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse,
    routes,
};
use rubato_protocol::{PairApproveRequest, PairingInfo};
use uuid::Uuid;

const SLICE: &str = "pairing";

/// The reason values of a refused pairing start or approval, as counted in
/// `rubato.web.rejected`.
pub mod reason {
    /// A start's hostname or version is too long (#168).
    pub const FIELD_TOO_LONG: &str = "field_too_long";
    /// Too many codes are pending (#168).
    pub const PENDING_CAP: &str = "pending_cap";
    /// One address holds its share of pending codes (ADR 0090).
    pub const PENDING_CAP_ADDRESS: &str = "pending_cap_address";
    /// The companion id is paired to another person (#160).
    pub const COMPANION_TAKEN: &str = "companion_taken";
}

/// Who a refusal of the anonymous start is logged as.
const ANONYMOUS: &str = "anonymous";

type Refusal = (StatusCode, String);

pub async fn start(
    State(app): State<App>,
    client: Option<axum::Extension<crate::kernel::limits::ClientAddr>>,
    Json(req): Json<PairStartRequest>,
) -> Result<Json<PairStartResponse>, Refusal> {
    for (field, value) in [
        ("hostname", &req.hostname),
        ("companion_version", &req.companion_version),
    ] {
        if value.chars().count() > PAIR_FIELD_MAX_CHARS {
            app.reject(routes::PAIR_START, reason::FIELD_TOO_LONG, ANONYMOUS);
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("{field} is longer than {PAIR_FIELD_MAX_CHARS} characters."),
            ));
        }
    }
    let device_code = random_token();
    let code = user_code();
    let stored = store::create_pairing_code(
        &app.db,
        &req,
        &device_code,
        &code,
        app.config.pairing_ttl_secs,
        app.config.pairing_max_pending,
        client.map(|c| c.0.0.to_string()).as_deref(),
    )
    .await
    .map_err(|e| (internal(SLICE, e), String::new()))?;
    if stored != store::Started::Stored {
        // The same answer for both caps: one address is not told how full the server is.
        let why = if stored == store::Started::AddressFull {
            reason::PENDING_CAP_ADDRESS
        } else {
            reason::PENDING_CAP
        };
        app.reject(routes::PAIR_START, why, ANONYMOUS);
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many pairings are waiting for approval; try again in a few minutes.".into(),
        ));
    }
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
    let caps = app.capabilities(r.companion_id);
    PairingInfo {
        companion_id: r.companion_id,
        user: r.user,
        hostname: r.hostname,
        created_at: r.created_at,
        last_seen: r.last_seen,
        revoked_at: r.revoked_at,
        connected,
        client: r.client,
        // A lead may start here only with both flags on (ADR 0072).
        can_lead: app.config.orchestrator
            && caps.iter().any(|c| c == rubato_protocol::capability::FLEET),
        // The kind only, as the companion announced it (ADR 0076).
        claude_auth: caps.iter().find_map(|c| {
            c.strip_prefix(rubato_protocol::capability::CLAUDE_AUTH_PREFIX)
                .map(str::to_owned)
        }),
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
