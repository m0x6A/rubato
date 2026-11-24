//! Dismissed discrepancies (ADR 0060): table `traffic_dismissals`, per workspace, and the two
//! routes that change it. A dismissal hides what it names from the overlay and the findings,
//! never from the map file, and is never an input to progress, pins, plans or approvals.

use crate::auth::CurrentUser;
use crate::kernel::db::{parse_ts, ts};
use crate::kernel::http::{Ws, internal};
use crate::kernel::state::App;
use anyhow::Result;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use rubato_protocol::traffic::{MAX_DISMISSALS, routes};
use rubato_protocol::{DiscrepancyKey, DismissRequest, TrafficDismissal, TrafficEnd};
use sqlx::{Row, SqlitePool};

const SLICE: &str = "traffic";

/// The reason values of a refused dismissal, as counted in `rubato.web.rejected`.
pub mod reason {
    pub const TOO_MANY: &str = "too_many";
    pub const UNKNOWN_TARGET: &str = "unknown_target";
}

fn row(r: &sqlx::sqlite::SqliteRow) -> Result<TrafficDismissal> {
    Ok(TrafficDismissal {
        id: r.get("id"),
        target: serde_json::from_str(&r.get::<String, _>("target_key"))?,
        author: r.get("author"),
        at: parse_ts(&r.get::<String, _>("created_at")),
    })
}

/// Every dismissal in the workspace, oldest first. A row that no longer reads as a key (an
/// older format) is skipped and logged, never fatal.
pub async fn list(db: &SqlitePool, ws: &str) -> Result<Vec<TrafficDismissal>> {
    let rows = sqlx::query(
        "SELECT id, target_key, author, created_at FROM traffic_dismissals WHERE workspace_id = ? ORDER BY id",
    )
    .bind(ws)
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            row(r)
                .inspect_err(|e| tracing::warn!(error = %e, "traffic dismissal unreadable"))
                .ok()
        })
        .collect())
}

pub enum Added {
    New(TrafficDismissal),
    Existing(TrafficDismissal),
    Full,
}

/// Dismiss `key` for the workspace as `by`, once: the same key again answers the first row.
pub async fn add(
    db: &SqlitePool,
    ws: &str,
    key: &DiscrepancyKey,
    by: &str,
    at: DateTime<Utc>,
) -> Result<Added> {
    let text = serde_json::to_string(key)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let existing = sqlx::query(
        "SELECT id, target_key, author, created_at FROM traffic_dismissals WHERE workspace_id = ? AND target_key = ?",
    )
    .bind(ws)
    .bind(&text)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(r) = existing {
        return Ok(Added::Existing(row(&r)?));
    }
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM traffic_dismissals WHERE workspace_id = ?")
            .bind(ws)
            .fetch_one(&mut *tx)
            .await?;
    if usize::try_from(n).unwrap_or(usize::MAX) >= MAX_DISMISSALS {
        return Ok(Added::Full);
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO traffic_dismissals (workspace_id, target_key, author, created_at) VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(ws)
    .bind(&text)
    .bind(by)
    .bind(ts(at))
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Added::New(TrafficDismissal {
        id,
        target: key.clone(),
        author: by.to_owned(),
        at,
    }))
}

/// Undo: the dismissal's row, if it is in this workspace.
pub async fn remove(db: &SqlitePool, ws: &str, id: i64) -> Result<Option<TrafficDismissal>> {
    let r = sqlx::query(
        "DELETE FROM traffic_dismissals WHERE workspace_id = ? AND id = ? RETURNING id, target_key, author, created_at",
    )
    .bind(ws)
    .bind(id)
    .fetch_optional(db)
    .await?;
    r.as_ref().map(row).transpose()
}

/// Whether the key names things the merged map can name: declared part ids at part ends and
/// a declared seam for a quiet seam. A ghost is any identity within the limits.
fn known(map: &rubato_protocol::SystemMap, key: &DiscrepancyKey) -> bool {
    let end = |e: &TrafficEnd| match e {
        TrafficEnd::Part { id } => map.node(id).is_some(),
        TrafficEnd::Ghost { .. } => true,
    };
    key.fits()
        && match key {
            DiscrepancyKey::Ghost { .. } => true,
            DiscrepancyKey::Edge { from, to } => end(from) && end(to) && from != to,
            DiscrepancyKey::QuietSeam { seam } => map.seam(seam).is_some(),
        }
}

/// `POST /api/traffic/dismissals`: 201 with the new dismissal, 200 with the one already there.
pub async fn dismiss(
    State(app): State<App>,
    ws: Ws,
    CurrentUser(user): CurrentUser,
    Json(req): Json<DismissRequest>,
) -> Result<(StatusCode, Json<TrafficDismissal>), (StatusCode, String)> {
    let map = app
        .map_of(ws.as_str())
        .await
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))?;
    if !known(&map.map, &req.target) {
        app.reject(routes::TRAFFIC_DISMISSALS, reason::UNKNOWN_TARGET, &user);
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("that discrepancy names nothing map {} has", map.version),
        ));
    }
    let added = add(&app.db, ws.as_str(), &req.target, &user, Utc::now())
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?;
    match added {
        Added::New(d) => {
            tracing::info!(rubato.user = %user, rubato.workspace = ws.as_str(), target = %serde_json::to_string(&d.target).unwrap_or_default(), "traffic dismissal added");
            app.notify_ws("traffic", ws.as_str());
            Ok((StatusCode::CREATED, Json(d)))
        }
        Added::Existing(d) => Ok((StatusCode::OK, Json(d))),
        Added::Full => {
            app.reject(routes::TRAFFIC_DISMISSALS, reason::TOO_MANY, &user);
            Err((
                StatusCode::CONFLICT,
                format!("this workspace already has {MAX_DISMISSALS} dismissals; undo some first"),
            ))
        }
    }
}

/// The `{id}` of `DELETE …/traffic/dismissals/{id}`, by name: under `/api/w/{ws}` the path
/// holds both.
#[derive(serde::Deserialize)]
pub struct IdParam {
    id: i64,
}

/// `DELETE /api/traffic/dismissals/{id}`: undo. 204, or 404 when it is not in this workspace.
pub async fn undismiss(
    State(app): State<App>,
    ws: Ws,
    CurrentUser(user): CurrentUser,
    Path(IdParam { id }): Path<IdParam>,
) -> Result<StatusCode, StatusCode> {
    match remove(&app.db, ws.as_str(), id)
        .await
        .map_err(|e| internal(SLICE, e))?
    {
        Some(d) => {
            tracing::info!(rubato.user = %user, rubato.workspace = ws.as_str(), target = %serde_json::to_string(&d.target).unwrap_or_default(), "traffic dismissal removed");
            app.notify_ws("traffic", ws.as_str());
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}
