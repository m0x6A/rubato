//! Table `workspace_settings` (migration 0027): one row per workspace, in switcher order.

use anyhow::Result;
use sqlx::{Row as _, Sqlite, SqlitePool, Transaction};

/// One workspace's stored settings, as the table holds them.
#[derive(Clone, PartialEq)]
pub struct Row {
    pub workspace_id: String,
    pub position: i64,
    pub revision: i64,
    pub name: Option<String>,
    pub map_repo: String,
    pub map_path: String,
    pub map_ref: String,
    pub traffic_source: Option<String>,
    /// A JSON object of the adapter's text fields.
    pub traffic_fields: Option<String>,
    /// A JSON object of the adapter's secrets, sealed.
    pub traffic_secrets_sealed: Option<String>,
    pub default_spend_cap_usd: Option<f64>,
    pub default_attempt_limit: Option<i64>,
    pub updated_by: String,
    pub updated_at: String,
}

/// Field names only: a row's `Debug` never prints a field value or a secret.
impl std::fmt::Debug for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Row")
            .field("workspace_id", &self.workspace_id)
            .field("revision", &self.revision)
            .field("map_repo", &self.map_repo)
            .field("map_ref", &self.map_ref)
            .field("traffic_source", &self.traffic_source)
            .field(
                "traffic_secrets",
                &self.traffic_secrets_sealed.as_ref().map(|_| "set"),
            )
            .finish()
    }
}

pub async fn count(tx: &mut Transaction<'_, Sqlite>) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM workspace_settings")
            .fetch_one(&mut **tx)
            .await?,
    )
}

pub async fn insert(tx: &mut Transaction<'_, Sqlite>, r: &Row) -> Result<()> {
    sqlx::query(
        "INSERT INTO workspace_settings (workspace_id, position, revision, name, map_repo, map_path, map_ref, traffic_source, traffic_fields, traffic_secrets_sealed, default_spend_cap_usd, default_attempt_limit, updated_by, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&r.workspace_id)
    .bind(r.position)
    .bind(r.revision)
    .bind(&r.name)
    .bind(&r.map_repo)
    .bind(&r.map_path)
    .bind(&r.map_ref)
    .bind(&r.traffic_source)
    .bind(&r.traffic_fields)
    .bind(&r.traffic_secrets_sealed)
    .bind(r.default_spend_cap_usd)
    .bind(r.default_attempt_limit)
    .bind(&r.updated_by)
    .bind(&r.updated_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Every row, in switcher order.
pub async fn all(db: &SqlitePool) -> Result<Vec<Row>> {
    let rows = sqlx::query("SELECT * FROM workspace_settings ORDER BY position, workspace_id")
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| Row {
            workspace_id: r.get("workspace_id"),
            position: r.get("position"),
            revision: r.get("revision"),
            name: r.get("name"),
            map_repo: r.get("map_repo"),
            map_path: r.get("map_path"),
            map_ref: r.get("map_ref"),
            traffic_source: r.get("traffic_source"),
            traffic_fields: r.get("traffic_fields"),
            traffic_secrets_sealed: r.get("traffic_secrets_sealed"),
            default_spend_cap_usd: r.get("default_spend_cap_usd"),
            default_attempt_limit: r.get("default_attempt_limit"),
            updated_by: r.get("updated_by"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}
