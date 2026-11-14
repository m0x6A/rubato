//! Table `map_cache`: the last good map of each workspace (ADR 0047).

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

pub async fn store_map(
    db: &SqlitePool,
    ws: &str,
    version: &str,
    yaml: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query("INSERT INTO map_cache (workspace_id, version, yaml, loaded_at) VALUES (?, ?, ?, ?) ON CONFLICT(workspace_id) DO UPDATE SET version = excluded.version, yaml = excluded.yaml, loaded_at = excluded.loaded_at")
        .bind(ws).bind(version).bind(yaml).bind(ts(at)).execute(db).await?;
    Ok(())
}

pub async fn load_map(
    db: &SqlitePool,
    ws: &str,
) -> Result<Option<(String, String, DateTime<Utc>)>> {
    Ok(
        sqlx::query("SELECT version, yaml, loaded_at FROM map_cache WHERE workspace_id = ?")
            .bind(ws)
            .fetch_optional(db)
            .await?
            .map(|r| {
                (
                    r.get("version"),
                    r.get("yaml"),
                    parse_ts(&r.get::<String, _>("loaded_at")),
                )
            }),
    )
}
