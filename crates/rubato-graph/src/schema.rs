//! The graph file's schema. It is derived data and is never migrated: a file whose
//! `user_version` is not [`USER_VERSION`] is emptied and created again, which costs the server
//! one tick and a companion one fetch.

use crate::Result;
use sqlx::{Row, SqlitePool};

/// Bump on any change to the DDL below.
pub const USER_VERSION: i64 = 1;

const TABLES: &[&str] = &["nodes_fts", "edges", "nodes", "sources"];

const DDL: &[&str] = &[
    "CREATE TABLE sources (
        source TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        generation INTEGER NOT NULL,
        head_sha TEXT,
        map_version TEXT,
        generator INTEGER NOT NULL DEFAULT 0,
        content_hash TEXT,
        generated_at TEXT,
        checked_at TEXT,
        attempted_at TEXT,
        error TEXT,
        nodes INTEGER NOT NULL DEFAULT 0,
        edges INTEGER NOT NULL DEFAULT 0,
        truncated INTEGER NOT NULL DEFAULT 0
    )",
    // An id is produced by one source (design 322 §3.2). The key still names the source, so a
    // generator's mistake shows as two rows, not as a build that can never be stored.
    "CREATE TABLE nodes (
        source TEXT NOT NULL,
        id TEXT NOT NULL,
        kind TEXT NOT NULL,
        name TEXT NOT NULL,
        repo TEXT,
        path TEXT,
        line INTEGER,
        end_line INTEGER,
        summary TEXT,
        detail TEXT NOT NULL,
        PRIMARY KEY (source, id)
    )",
    "CREATE INDEX nodes_by_id ON nodes (id)",
    "CREATE INDEX nodes_by_path ON nodes (repo, path)",
    "CREATE TABLE edges (
        source TEXT NOT NULL,
        from_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        to_id TEXT NOT NULL,
        detail TEXT NOT NULL
    )",
    "CREATE INDEX edges_by_source ON edges (source)",
    "CREATE INDEX edges_out ON edges (from_id, kind)",
    "CREATE INDEX edges_in ON edges (to_id, kind)",
    // An ordinary FTS5 table, deleted from and filled by the replace itself: no triggers.
    "CREATE VIRTUAL TABLE nodes_fts USING fts5 (name, path, summary, source UNINDEXED, id UNINDEXED)",
];

pub(crate) async fn user_version(pool: &SqlitePool) -> Result<i64> {
    Ok(sqlx::query("PRAGMA user_version")
        .fetch_one(pool)
        .await?
        .get::<i64, _>(0))
}

/// Create the schema, or empty a file of another version and create it again. Returns whether
/// an older file was emptied.
pub(crate) async fn ensure(pool: &SqlitePool) -> Result<bool> {
    let found = user_version(pool).await?;
    if found == USER_VERSION {
        return Ok(false);
    }
    let had_tables = sqlx::query("SELECT 1 FROM sqlite_master WHERE name = 'sources'")
        .fetch_optional(pool)
        .await?
        .is_some();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    for t in TABLES {
        sqlx::query(&format!("DROP TABLE IF EXISTS {t}"))
            .execute(&mut *tx)
            .await?;
    }
    for ddl in DDL {
        sqlx::query(ddl).execute(&mut *tx).await?;
    }
    sqlx::query(&format!("PRAGMA user_version = {USER_VERSION}"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(had_tables || found != 0)
}
