//! Table `map_layout`, per workspace (ADR 0047).

use crate::kernel::db::ts;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::layout::{MapLayout, Position};
use sqlx::{Row, SqliteConnection, SqlitePool};

async fn read(conn: &mut SqliteConnection, ws: &str) -> Result<MapLayout> {
    let rows = sqlx::query("SELECT node_id, x, y FROM map_layout WHERE workspace_id = ?")
        .bind(ws)
        .fetch_all(conn)
        .await?;
    Ok(MapLayout {
        positions: rows
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("node_id"),
                    Position {
                        x: r.get("x"),
                        y: r.get("y"),
                    },
                )
            })
            .collect(),
    })
}

/// Every stored position, including rows for parts the map no longer has; the reader filters.
pub async fn all(db: &SqlitePool, ws: &str) -> Result<MapLayout> {
    read(&mut *db.acquire().await?, ws).await
}

/// Store the moved parts, all or none, and drop rows for ids that are not in `known`.
pub async fn put(
    db: &SqlitePool,
    ws: &str,
    moved: &MapLayout,
    known: &[&str],
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
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT node_id FROM map_layout WHERE workspace_id = ?")
            .bind(ws)
            .fetch_all(&mut *tx)
            .await?;
    for id in stored.iter().filter(|id| !known.contains(&id.as_str())) {
        sqlx::query("DELETE FROM map_layout WHERE workspace_id = ? AND node_id = ?")
            .bind(ws)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Remove every position and return what was removed.
pub async fn clear(db: &SqlitePool, ws: &str) -> Result<MapLayout> {
    let mut tx = db.begin().await?;
    let removed = read(&mut tx, ws).await?;
    sqlx::query("DELETE FROM map_layout WHERE workspace_id = ?")
        .bind(ws)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(removed)
}
