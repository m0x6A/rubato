//! `GET /api/overview`: the map, every issue with its derived progress, findings, companions.

use super::build;
use super::inputs::build_overview_inputs;
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::pairing;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::web::{MapResponse, Overview};

const SLICE: &str = "overview";

pub async fn overview(
    State(app): State<App>,
    _user: CurrentUser,
) -> Result<Json<Overview>, StatusCode> {
    let map = app
        .map
        .read()
        .await
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let inputs = build_overview_inputs(&app)
        .await
        .map_err(|e| internal(SLICE, e))?;
    let built = build::build(&inputs, Some(&map), &app.config.limits, Utc::now());
    let companions = pairing::list_pairings(&app.db, None)
        .await
        .map_err(|e| internal(SLICE, e))?
        .into_iter()
        .map(|r| pairing::to_info(&app, r))
        .collect();
    Ok(Json(Overview {
        map: MapResponse {
            map: map.map,
            version: map.version,
            loaded_at: map.loaded_at,
        },
        issues: built.issues,
        findings: built.findings,
        companions,
        unpinned_sessions: built.unpinned_sessions,
        generated_at: Utc::now(),
    }))
}
