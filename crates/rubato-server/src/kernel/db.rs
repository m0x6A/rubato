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
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    Ok(pool)
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
