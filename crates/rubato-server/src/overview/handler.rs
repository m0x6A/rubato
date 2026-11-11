//! `GET /api/overview`: the map, every issue with its derived progress, findings, companions.

use super::build;
use super::inputs::build_overview_inputs;
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::{map_draft, pairing};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::{MapResponse, Overview};

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
    let companions: Vec<_> = pairing::list_pairings(&app.db, None)
        .await
        .map_err(|e| internal(SLICE, e))?
        .into_iter()
        .map(|r| pairing::to_info(&app, r))
        .collect();
    // A revoked companion's lane takes no starts, so its queue is not shown (its rows go with
    // the next queue write).
    let queue = inputs
        .queue
        .iter()
        .filter(|q| {
            companions
                .iter()
                .any(|c| c.companion_id == q.companion_id && c.revoked_at.is_none())
        })
        .cloned()
        .collect();
    let draft = map_draft::snapshot(&app.db, &map.map)
        .await
        .map_err(|e| internal(SLICE, e))?;
    // A part that exists only in the draft may have a position too (ADR 0016).
    let mut layout = built.layout;
    if let Some(preview) = &draft.preview {
        for n in &preview.nodes {
            if let Some(p) = inputs.layout.positions.get(&n.id) {
                layout.positions.insert(n.id.clone(), *p);
            }
        }
    }
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
        backlog: built.backlog,
        layout,
        queue,
        draft,
        generated_at: Utc::now(),
    }))
}
