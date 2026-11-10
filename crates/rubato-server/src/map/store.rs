//! Table `map_cache`.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

pub async fn store_map(
    db: &SqlitePool,
    version: &str,
    yaml: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query("INSERT INTO map_cache (id, version, yaml, loaded_at) VALUES (1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET version = excluded.version, yaml = excluded.yaml, loaded_at = excluded.loaded_at")
        .bind(version).bind(yaml).bind(ts(at)).execute(db).await?;
    Ok(())
}

pub async fn load_map(db: &SqlitePool) -> Result<Option<(String, String, DateTime<Utc>)>> {
    Ok(
        sqlx::query("SELECT version, yaml, loaded_at FROM map_cache WHERE id = 1")
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
