//! The SQLite connection and the helpers every slice's `store.rs` shares. Rows hold signals;
//! the progress state is never written (each table's SQL lives in its owning slice, design §7).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

pub async fn connect(url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)
        .context("database url")?
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .context("connecting to sqlite")?;
    let purged_before = applied(&pool, PURGE_MIGRATION).await;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    if !purged_before && applied(&pool, PURGE_MIGRATION).await {
        vacuum_after_purge(&pool).await;
    }
    Ok(pool)
}

/// Migration 0015 removes the raw hook payloads (ADR 0052). An UPDATE leaves the old bytes in
/// free pages, and VACUUM cannot run inside a migration's transaction, so it runs here, once,
/// right after that migration was applied.
const PURGE_MIGRATION: i64 = 15;

async fn applied(pool: &SqlitePool, version: i64) -> bool {
    sqlx::query("SELECT 1 FROM _sqlx_migrations WHERE version = ? AND success = 1")
        .bind(version)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_some()
}

async fn vacuum_after_purge(pool: &SqlitePool) {
    let pages = |sql: &'static str| async move {
        sqlx::query(sql)
            .fetch_one(pool)
            .await
            .map(|r| r.get::<i64, _>(0))
            .unwrap_or(-1)
    };
    let free_before = pages("PRAGMA freelist_count").await;
    match sqlx::query("VACUUM").execute(pool).await {
        Ok(_) => tracing::info!(
            free_pages_before = free_before,
            free_pages_after = pages("PRAGMA freelist_count").await,
            "vacuum after hook payload purge"
        ),
        Err(e) => {
            tracing::warn!(error = %e, "vacuum after hook payload purge failed; run VACUUM by hand")
        }
    }
}

pub fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339()
}

pub fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

/// For the test that proves progress is never stored: every column of every table.
pub async fn all_columns(db: &SqlitePool) -> Result<Vec<(String, String)>> {
    let tables = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%'").fetch_all(db).await?;
    let mut out = Vec::new();
    for t in tables {
        let name: String = t.get("name");
        let cols = sqlx::query(&format!("PRAGMA table_info({name})"))
            .fetch_all(db)
            .await?;
        for c in cols {
            out.push((name.clone(), c.get::<String, _>("name")));
        }
    }
    Ok(out)
}
