//! Tables `map_layout` (the flat map, ADR 0016) and `map_layout_grouped` (the By repo grouping,
//! ADR 0068), per workspace (ADR 0047).

use crate::kernel::db::ts;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::layout::{MapLayout, Position};
use sqlx::{Row, SqliteConnection, SqlitePool};
use std::collections::BTreeMap;

const PART: &str = "part";
const BOX: &str = "box";

fn position(r: &sqlx::sqlite::SqliteRow) -> Position {
    Position {
        x: r.get("x"),
        y: r.get("y"),
    }
}

async fn read_flat(conn: &mut SqliteConnection, ws: &str) -> Result<BTreeMap<String, Position>> {
    let rows = sqlx::query("SELECT node_id, x, y FROM map_layout WHERE workspace_id = ?")
        .bind(ws)
        .fetch_all(conn)
        .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<String, _>("node_id"), position(r)))
        .collect())
}

/// The grouping's parts and boxes.
async fn read_grouped(conn: &mut SqliteConnection, ws: &str) -> Result<MapLayout> {
    let rows = sqlx::query("SELECT kind, id, x, y FROM map_layout_grouped WHERE workspace_id = ?")
        .bind(ws)
        .fetch_all(conn)
        .await?;
    let mut out = MapLayout::default();
    for r in &rows {
        let id: String = r.get("id");
        match r.get::<String, _>("kind").as_str() {
            PART => out.grouped.insert(id, position(r)),
            _ => out.boxes.insert(id, position(r)),
        };
    }
    Ok(out)
}

/// Every stored position of both arrangements, including rows for parts or repos the map no
/// longer has; the reader filters.
pub async fn all(db: &SqlitePool, ws: &str) -> Result<MapLayout> {
    let mut conn = db.acquire().await?;
    let mut out = read_grouped(&mut conn, ws).await?;
    out.positions = read_flat(&mut conn, ws).await?;
    Ok(out)
}

/// What a write may name: the parts of the map and the draft, and the repos they name.
pub struct Known<'a> {
    pub parts: &'a [&'a str],
    pub repos: &'a [&'a str],
}

/// Store what moved, in both arrangements, all or none, and drop rows for parts and repos that
/// are not in `known`.
pub async fn put(
    db: &SqlitePool,
    ws: &str,
    moved: &MapLayout,
    known: &Known<'_>,
    by: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    let mut tx = db.begin().await?;
    for (id, p) in &moved.positions {
        sqlx::query(
            "INSERT INTO map_layout (workspace_id, node_id, x, y, updated_by, updated_at) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (workspace_id, node_id) DO UPDATE SET x = excluded.x, y = excluded.y,
                 updated_by = excluded.updated_by, updated_at = excluded.updated_at",
        )
        .bind(ws)
        .bind(id)
        .bind(p.x)
        .bind(p.y)
        .bind(by)
        .bind(ts(at))
        .execute(&mut *tx)
        .await?;
    }
    let grouped = moved
        .grouped
        .iter()
        .map(|(id, p)| (PART, id, p))
        .chain(moved.boxes.iter().map(|(id, p)| (BOX, id, p)));
    for (kind, id, p) in grouped {
        sqlx::query(
            "INSERT INTO map_layout_grouped (workspace_id, kind, id, x, y, updated_by, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (workspace_id, kind, id) DO UPDATE SET x = excluded.x, y = excluded.y,
                 updated_by = excluded.updated_by, updated_at = excluded.updated_at",
        )
        .bind(ws)
        .bind(kind)
        .bind(id)
        .bind(p.x)
        .bind(p.y)
        .bind(by)
        .bind(ts(at))
        .execute(&mut *tx)
        .await?;
    }
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT node_id FROM map_layout WHERE workspace_id = ?")
            .bind(ws)
            .fetch_all(&mut *tx)
            .await?;
    for id in stored
        .iter()
        .filter(|id| !known.parts.contains(&id.as_str()))
    {
        sqlx::query("DELETE FROM map_layout WHERE workspace_id = ? AND node_id = ?")
            .bind(ws)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let stored = sqlx::query("SELECT kind, id FROM map_layout_grouped WHERE workspace_id = ?")
        .bind(ws)
        .fetch_all(&mut *tx)
        .await?;
    for r in &stored {
        let (kind, id): (String, String) = (r.get("kind"), r.get("id"));
        let keep = if kind == PART {
            known.parts.contains(&id.as_str())
        } else {
            known.repos.contains(&id.as_str())
        };
        if !keep {
            sqlx::query(
                "DELETE FROM map_layout_grouped WHERE workspace_id = ? AND kind = ? AND id = ?",
            )
            .bind(ws)
            .bind(&kind)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Remove every flat position and return what was removed. The grouping stays.
pub async fn clear(db: &SqlitePool, ws: &str) -> Result<MapLayout> {
    let mut tx = db.begin().await?;
    let removed = MapLayout {
        positions: read_flat(&mut tx, ws).await?,
        ..Default::default()
    };
    sqlx::query("DELETE FROM map_layout WHERE workspace_id = ?")
        .bind(ws)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(removed)
}

/// Remove the By repo grouping, parts and boxes, and return what was removed. The flat map stays.
pub async fn clear_grouped(db: &SqlitePool, ws: &str) -> Result<MapLayout> {
    let mut tx = db.begin().await?;
    let removed = read_grouped(&mut tx, ws).await?;
    sqlx::query("DELETE FROM map_layout_grouped WHERE workspace_id = ?")
        .bind(ws)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(removed)
}
