//! `PUT /api/layout` stores what moved, in the flat map and in the By repo grouping;
//! `DELETE /api/layout` clears the flat map and `DELETE /api/layout/grouped` the grouping, each
//! answering with what it removed, so the caller can offer Undo.

use super::store;
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::kernel::tenancy::WsKey;
use crate::map_draft;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::layout::{MAX_COORDINATE, MapLayout, Position, arrangement, routes};
use rubato_protocol::otel::attr;
use std::collections::{BTreeMap, BTreeSet};

const SLICE: &str = "layout";

/// The reason values of a refused layout write, as counted in `rubato.web.rejected`.
pub mod reason {
    pub const UNKNOWN_ID: &str = "unknown_id";
    pub const BAD_POSITION: &str = "bad_position";
}

fn written(app: &App, ws: &WsKey, which: &str) {
    app.metrics.layout_writes.add(
        1,
        &[KeyValue::new(attr::LAYOUT_ARRANGEMENT, which.to_owned())],
    );
    app.notify_ws("layout", ws);
}

/// The arrangements a write touches.
fn arrangements(l: &MapLayout) -> Vec<&'static str> {
    let mut out = Vec::new();
    if !l.positions.is_empty() {
        out.push(arrangement::FLAT);
    }
    if !l.grouped.is_empty() || !l.boxes.is_empty() {
        out.push(arrangement::GROUPED);
    }
    out
}

/// The parts of the merged map and of the shared draft (ADR 0016), and the repos they name: a
/// box is a repo some part names (ADR 0068). With the map's version, for the refusal's text.
struct Known {
    version: String,
    parts: BTreeSet<String>,
    repos: BTreeSet<String>,
}

async fn known(app: &App, ws: &WsKey) -> Result<Option<Known>, StatusCode> {
    let Some(map) = app.map_of(ws).await else {
        return Ok(None);
    };
    let draft = map_draft::snapshot(&app.db, ws, &map.map)
        .await
        .map_err(|e| internal(SLICE, e))?;
    let mut out = Known {
        version: map.version.clone(),
        parts: BTreeSet::new(),
        repos: BTreeSet::new(),
    };
    let draft_nodes = draft.preview.iter().flat_map(|p| p.nodes.iter());
    for n in map.map.nodes.iter().chain(draft_nodes) {
        out.parts.insert(n.id.clone());
        if let Some(r) = &n.repo {
            out.repos.insert(r.clone());
        }
    }
    Ok(Some(out))
}

/// What a clear answers: only rows for parts and repos there are now, so a `PUT` of it (Undo) is
/// never refused for a row the next write would have dropped anyway.
fn current(mut removed: MapLayout, known: Option<&Known>) -> MapLayout {
    if let Some(k) = known {
        removed.positions.retain(|id, _| k.parts.contains(id));
        removed.grouped.retain(|id, _| k.parts.contains(id));
        removed.boxes.retain(|id, _| k.repos.contains(id));
    }
    removed
}

pub async fn put_layout(
    State(app): State<App>,
    ws: crate::kernel::http::Ws,
    CurrentUser(user): CurrentUser,
    Json(moved): Json<MapLayout>,
) -> Result<StatusCode, (StatusCode, String)> {
    let k = known(&app, ws.key())
        .await
        .map_err(|s| (s, String::new()))?
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))?;
    // All or none: one bad entry refuses the write, so a browser never half-saves a drag.
    let checks: [(&BTreeMap<String, Position>, &BTreeSet<String>, &str); 3] = [
        (&moved.positions, &k.parts, "a part"),
        (&moved.grouped, &k.parts, "a part"),
        (&moved.boxes, &k.repos, "a repo"),
    ];
    for (positions, known, what) in checks {
        for (id, p) in positions {
            if !known.contains(id) {
                app.reject(routes::LAYOUT, reason::UNKNOWN_ID, &user);
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("{id} is not {what} in map {}", k.version),
                ));
            }
            if !p.on_canvas() {
                app.reject(routes::LAYOUT, reason::BAD_POSITION, &user);
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("the position of {id} is not within {MAX_COORDINATE} of the origin"),
                ));
            }
        }
    }
    let parts: Vec<&str> = k.parts.iter().map(String::as_str).collect();
    let repos: Vec<&str> = k.repos.iter().map(String::as_str).collect();
    let known = store::Known {
        parts: &parts,
        repos: &repos,
    };
    store::put(&app.db, ws.key(), &moved, &known, &user, Utc::now())
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?;
    for which in arrangements(&moved) {
        // At info: the 3 am signal for a lost or clobbering move (plan 144 O-row).
        tracing::info!(
            rubato.user = %user,
            rubato.layout.arrangement = which,
            parts = moved.positions.len() + moved.grouped.len(),
            boxes = moved.boxes.len(),
            "layout written"
        );
        written(&app, ws.key(), which);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn clear_layout(
    State(app): State<App>,
    ws: crate::kernel::http::Ws,
    CurrentUser(user): CurrentUser,
) -> Result<Json<MapLayout>, StatusCode> {
    let k = known(&app, ws.key()).await?;
    let removed = store::clear(&app.db, ws.key())
        .await
        .map_err(|e| internal(SLICE, e))?;
    let removed = current(removed, k.as_ref());
    // Clearing moves every part for the whole team, so it is said at info with who did it.
    tracing::info!(
        rubato.user = %user,
        rubato.layout.arrangement = arrangement::FLAT,
        parts = removed.positions.len(),
        "layout cleared"
    );
    written(&app, ws.key(), arrangement::FLAT);
    Ok(Json(removed))
}

/// `DELETE /api/layout/grouped`: the By repo grouping goes back to ELK for everyone (ADR 0068).
pub async fn clear_grouped_layout(
    State(app): State<App>,
    ws: crate::kernel::http::Ws,
    CurrentUser(user): CurrentUser,
) -> Result<Json<MapLayout>, StatusCode> {
    let k = known(&app, ws.key()).await?;
    let removed = store::clear_grouped(&app.db, ws.key())
        .await
        .map_err(|e| internal(SLICE, e))?;
    let removed = current(removed, k.as_ref());
    tracing::info!(
        rubato.user = %user,
        rubato.layout.arrangement = arrangement::GROUPED,
        parts = removed.grouped.len(),
        boxes = removed.boxes.len(),
        "layout cleared"
    );
    written(&app, ws.key(), arrangement::GROUPED);
    Ok(Json(removed))
}
