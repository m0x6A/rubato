//! Table `pins`.

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

pub async fn add_pin(db: &SqlitePool, pin: &IssuePin) -> Result<()> {
    let (kind, target) = map_ref_parts(&pin.target);
    sqlx::query("INSERT OR IGNORE INTO pins (repo, number, kind, target, pinned_by, pinned_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&pin.issue.repo).bind(pin.issue.number as i64).bind(kind).bind(target).bind(&pin.pinned_by).bind(ts(pin.pinned_at)).execute(db).await?;
    Ok(())
}

pub async fn remove_pin(db: &SqlitePool, issue: &IssueRef, target: &MapRef) -> Result<bool> {
    let (kind, t) = map_ref_parts(target);
    let r =
        sqlx::query("DELETE FROM pins WHERE repo = ? AND number = ? AND kind = ? AND target = ?")
            .bind(&issue.repo)
            .bind(issue.number as i64)
            .bind(kind)
            .bind(t)
            .execute(db)
            .await?;
    Ok(r.rows_affected() > 0)
}

pub async fn list_pins(db: &SqlitePool) -> Result<Vec<IssuePin>> {
    let rows = sqlx::query("SELECT * FROM pins ORDER BY pinned_at")
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
