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

fn read(r: &sqlx::sqlite::SqliteRow) -> Row {
    Row {
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
    }
}

/// Every row, in switcher order.
pub async fn all(db: &SqlitePool) -> Result<Vec<Row>> {
    let rows = sqlx::query("SELECT * FROM workspace_settings ORDER BY position, workspace_id")
        .fetch_all(db)
        .await?;
    Ok(rows.iter().map(read).collect())
}

pub async fn get(db: &SqlitePool, id: &str) -> Result<Option<Row>> {
    Ok(
        sqlx::query("SELECT * FROM workspace_settings WHERE workspace_id = ?")
            .bind(id)
            .fetch_optional(db)
            .await?
            .as_ref()
            .map(read),
    )
}

/// A new workspace, last in the switcher. The unique index on the map answers a second
/// workspace on one map with an error.
pub async fn create(db: &SqlitePool, r: &Row) -> Result<Row> {
    let mut tx = db.begin().await?;
    let position: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(position), -1) + 1 FROM workspace_settings")
            .fetch_one(&mut *tx)
            .await?;
    let row = Row {
        position,
        revision: 1,
        ..r.clone()
    };
    insert(&mut tx, &row).await?;
    tx.commit().await?;
    Ok(row)
}

/// Store `r` over the row at `read_revision`, one revision on: the compare-and-set of an edit.
/// `false` when the row is gone or at another revision; nothing is written then. The map
/// source's repo and path are never written: they are fixed at insert.
pub async fn update(db: &SqlitePool, r: &Row, read_revision: i64) -> Result<bool> {
    let done = sqlx::query(
        "UPDATE workspace_settings SET revision = ?, name = ?, map_ref = ?, traffic_source = ?, traffic_fields = ?, traffic_secrets_sealed = ?, default_spend_cap_usd = ?, default_attempt_limit = ?, updated_by = ?, updated_at = ? WHERE workspace_id = ? AND revision = ?",
    )
    .bind(read_revision + 1)
    .bind(&r.name)
    .bind(&r.map_ref)
    .bind(&r.traffic_source)
    .bind(&r.traffic_fields)
    .bind(&r.traffic_secrets_sealed)
    .bind(r.default_spend_cap_usd)
    .bind(r.default_attempt_limit)
    .bind(&r.updated_by)
    .bind(&r.updated_at)
    .bind(&r.workspace_id)
    .bind(read_revision)
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Remove a workspace's settings row. Every other row of the workspace is kept.
pub async fn delete(db: &SqlitePool, id: &str) -> Result<bool> {
    let done = sqlx::query("DELETE FROM workspace_settings WHERE workspace_id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(done.rows_affected() == 1)
}
