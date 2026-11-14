//! Tables `pins` and `map_line_pins`, both per workspace (ADR 0047).

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use rubato_protocol::{IssuePin, IssueRef, MapRef};
use sqlx::{Row, SqlitePool};

fn map_ref_parts(m: &MapRef) -> (&'static str, &str) {
    match m {
        MapRef::Node { id } => ("node", id),
        MapRef::Seam { id } => ("seam", id),
    }
}

pub async fn add_pin(db: &SqlitePool, ws: &str, pin: &IssuePin) -> Result<()> {
    let (kind, target) = map_ref_parts(&pin.target);
    sqlx::query("INSERT OR IGNORE INTO pins (workspace_id, repo, number, kind, target, pinned_by, pinned_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(ws).bind(&pin.issue.repo).bind(pin.issue.number as i64).bind(kind).bind(target).bind(&pin.pinned_by).bind(ts(pin.pinned_at)).execute(db).await?;
    Ok(())
}

pub async fn remove_pin(
    db: &SqlitePool,
    ws: &str,
    issue: &IssueRef,
    target: &MapRef,
) -> Result<bool> {
    let (kind, t) = map_ref_parts(target);
    let r =
        sqlx::query("DELETE FROM pins WHERE workspace_id = ? AND repo = ? AND number = ? AND kind = ? AND target = ?")
            .bind(ws)
            .bind(&issue.repo)
            .bind(issue.number as i64)
            .bind(kind)
            .bind(t)
            .execute(db)
            .await?;
    Ok(r.rows_affected() > 0)
}

pub async fn list_pins(db: &SqlitePool, ws: &str) -> Result<Vec<IssuePin>> {
    let rows = sqlx::query("SELECT * FROM pins WHERE workspace_id = ? ORDER BY pinned_at")
        .bind(ws)
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let kind: String = r.get("kind");
            let target: String = r.get("target");
            IssuePin {
                issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
                target: if kind == "seam" {
                    MapRef::seam(target)
                } else {
                    MapRef::node(target)
                },
                pinned_by: r.get("pinned_by"),
                pinned_at: parse_ts(&r.get::<String, _>("pinned_at")),
            }
        })
        .collect())
}

/// Whether the issue is pinned to anything.
pub async fn is_pinned(db: &SqlitePool, ws: &str, issue: &IssueRef) -> Result<bool> {
    Ok(
        sqlx::query(
            "SELECT 1 FROM pins WHERE workspace_id = ? AND repo = ? AND number = ? LIMIT 1",
        )
        .bind(ws)
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .fetch_optional(db)
        .await?
        .is_some(),
    )
}

/// Whether a `Map:` line for this (issue, target) has been applied before (ADR 0026).
pub async fn map_line_applied(
    db: &SqlitePool,
    ws: &str,
    issue: &IssueRef,
    target: &MapRef,
) -> Result<bool> {
    let (kind, t) = map_ref_parts(target);
    Ok(sqlx::query(
        "SELECT 1 FROM map_line_pins WHERE workspace_id = ? AND repo = ? AND number = ? AND kind = ? AND target = ?",
    )
    .bind(ws)
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(kind)
    .bind(t)
    .fetch_optional(db)
    .await?
    .is_some())
}

/// Record that a `Map:` line for this (issue, target) is applied, so it is not applied again.
pub async fn record_map_line(
    db: &SqlitePool,
    ws: &str,
    issue: &IssueRef,
    target: &MapRef,
    outcome: &str,
) -> Result<()> {
    let (kind, t) = map_ref_parts(target);
    sqlx::query("INSERT OR IGNORE INTO map_line_pins (workspace_id, repo, number, kind, target, outcome, applied_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(ws).bind(&issue.repo).bind(issue.number as i64).bind(kind).bind(t).bind(outcome).bind(ts(chrono::Utc::now())).execute(db).await?;
    Ok(())
}
