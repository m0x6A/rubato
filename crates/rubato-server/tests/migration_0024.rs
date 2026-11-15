//! Migration 0024 (plan 144, ADR 0068): the By repo grouping gets its own table; the flat map's
//! positions are untouched.

use sqlx::{Row, SqlitePool};
use std::borrow::Cow;

async fn at(version: i64) -> (SqlitePool, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/m.db?mode=rwc", tmp.path().display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let mut m = sqlx::migrate!("./migrations");
    m.migrations = Cow::Owned(
        m.migrations
            .iter()
            .filter(|x| x.version <= version)
            .cloned()
            .collect(),
    );
    m.run(&pool).await.unwrap();
    (pool, tmp)
}

const T: &str = "2026-11-15T08:00:00+00:00";

#[tokio::test]
async fn flat_positions_survive_and_the_grouping_starts_empty() {
    let (pool, _tmp) = at(23).await;
    sqlx::query(&format!(
        "INSERT INTO map_layout (workspace_id, node_id, x, y, updated_by, updated_at) VALUES ('default', 'web', 1.5, -2, 'u', '{T}'), ('shop', 'cart', 3, 4, 'u', '{T}')"
    ))
    .execute(&pool)
    .await
    .unwrap();
    let rows = |pool: SqlitePool| async move {
        sqlx::query("SELECT workspace_id, node_id, x, y FROM map_layout ORDER BY workspace_id")
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("workspace_id"),
                    r.get::<String, _>("node_id"),
                    r.get::<f64, _>("x"),
                    r.get::<f64, _>("y"),
                )
            })
            .collect::<Vec<_>>()
    };
    let before = rows(pool.clone()).await;

    let mut m = sqlx::migrate!("./migrations");
    m.migrations = Cow::Owned(
        m.migrations
            .iter()
            .filter(|x| x.version <= 24)
            .cloned()
            .collect(),
    );
    m.run(&pool).await.unwrap();

    assert_eq!(
        rows(pool.clone()).await,
        before,
        "the flat layout is untouched"
    );
    let grouped: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM map_layout_grouped")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(grouped, 0);
    // Only parts and boxes; one row per id per kind per workspace.
    let bad = sqlx::query(&format!(
        "INSERT INTO map_layout_grouped VALUES ('default', 'lane', 'x', 0, 0, 'u', '{T}')"
    ))
    .execute(&pool)
    .await;
    assert!(bad.is_err(), "kind is part or box");
    for kind in ["part", "box"] {
        sqlx::query(&format!(
            "INSERT INTO map_layout_grouped VALUES ('default', '{kind}', 'web', 0, 0, 'u', '{T}')"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    println!("checked {} flat rows and 3 grouped inserts", before.len());
}
