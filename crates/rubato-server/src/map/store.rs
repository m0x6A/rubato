//! Table `map_cache`: the last good map of each workspace (ADR 0047).

use crate::kernel::db::{parse_ts, ts};
use crate::kernel::tenancy::WsKey;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

pub async fn store_map(
    db: &SqlitePool,
    ws: &WsKey,
    version: &str,
    yaml: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query("INSERT INTO map_cache (tenant_id, workspace_id, version, yaml, loaded_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(tenant_id, workspace_id) DO UPDATE SET version = excluded.version, yaml = excluded.yaml, loaded_at = excluded.loaded_at")
        .bind(ws.tenant.as_str())
        .bind(ws.ws()).bind(version).bind(yaml).bind(ts(at)).execute(db).await?;
    Ok(())
}

pub async fn load_map(
    db: &SqlitePool,
    ws: &WsKey,
) -> Result<Option<(String, String, DateTime<Utc>)>> {
    Ok(sqlx::query(
        "SELECT version, yaml, loaded_at FROM map_cache WHERE tenant_id = ? AND workspace_id = ?",
    )
    .bind(ws.tenant.as_str())
    .bind(ws.ws())
    .fetch_optional(db)
    .await?
    .map(|r| {
        (
            r.get("version"),
            r.get("yaml"),
            parse_ts(&r.get::<String, _>("loaded_at")),
        )
    }))
}
