//! Migration 0014 (plan 122, ADR 0052): stored hook events lose Claude Code's raw payload, every
//! row survives, and rows of other kinds are untouched.

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

const T: &str = "2026-11-14T08:00:00+00:00";

#[tokio::test]
async fn hook_payloads_are_purged_and_every_row_survives() {
    let (pool, _tmp) = at(13).await;
    let hook = |seq: i64, raw: &str| {
        format!(
            r#"INSERT INTO companion_events (companion_id, seq, kind, payload, received_at) VALUES ('c', {seq}, 'hook', '{{"type":"hook","session_id":"s","hook_event_name":"PostToolUse","cwd":"/w","tool_name":"Bash","received_at":"{T}"{raw}}}', '{T}')"#
        )
    };
    let seed = [
        hook(
            1,
            r#","payload":{"tool_input":{"command":"cat .env"},"tool_response":{"stdout":"TOKEN=abc"}}"#,
        ),
        hook(2, r#","payload":{"prompt":"the secret plan"}"#),
        // Already without a payload: left as it is.
        hook(3, ""),
        // Another kind that happens to have a `payload` key is not a hook: untouched.
        format!(
            r#"INSERT INTO companion_events (companion_id, seq, kind, payload, received_at) VALUES ('c', 4, 'launch', '{{"type":"launch","payload":{{"x":1}}}}', '{T}')"#
        ),
    ];
    for sql in &seed {
        sqlx::query(sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    let count = |sql: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(sql)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get::<i64, _>(0)
        }
    };
    let rows_before = count("SELECT COUNT(*) FROM companion_events").await;
    let with_payload_before = count(
        "SELECT COUNT(*) FROM companion_events WHERE kind = 'hook' AND json_type(payload, '$.payload') IS NOT NULL",
    )
    .await;
    assert_eq!(with_payload_before, 2, "the seed must hold hook payloads");

    // Run every migration from 0014 on.
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    assert_eq!(
        count("SELECT COUNT(*) FROM companion_events").await,
        rows_before,
        "a row was lost"
    );
    assert_eq!(
        count(
            "SELECT COUNT(*) FROM companion_events WHERE kind = 'hook' AND json_type(payload, '$.payload') IS NOT NULL"
        )
        .await,
        0,
        "a hook row kept its raw payload"
    );
    let text: String =
        sqlx::query("SELECT group_concat(payload, ' ') FROM companion_events WHERE kind = 'hook'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
    for leaked in ["TOKEN=abc", "cat .env", "the secret plan"] {
        assert!(!text.contains(leaked), "{leaked} is still stored");
    }
    assert!(
        text.contains(r#""tool_name":"Bash""#),
        "the lifted fields must stay"
    );
    let launch: String = sqlx::query("SELECT payload FROM companion_events WHERE kind = 'launch'")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert!(
        launch.contains(r#""payload""#),
        "a non-hook row was changed"
    );
    println!(
        "checked {rows_before} rows: {with_payload_before} hook payloads purged, other kinds untouched"
    );
}

/// The server's own start runs the migration and then VACUUM, so the purged bytes do not linger in
/// free pages (ADR 0052).
#[tokio::test]
async fn the_server_start_vacuums_after_the_purge() {
    let (pool, tmp) = at(13).await;
    let big = "x".repeat(200_000);
    for seq in 1..=20 {
        sqlx::query(&format!(
            r#"INSERT INTO companion_events (companion_id, seq, kind, payload, received_at) VALUES ('c', {seq}, 'hook', '{{"type":"hook","session_id":"s","hook_event_name":"PostToolUse","cwd":"/w","received_at":"{T}","payload":{{"tool_response":"{big}"}}}}', '{T}')"#
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;
    let url = format!("sqlite://{}/m.db?mode=rwc", tmp.path().display());
    let pool = rubato_server::kernel::db::connect(&url).await.unwrap();
    let free: i64 = sqlx::query("PRAGMA freelist_count")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    let leaked: i64 =
        sqlx::query("SELECT COUNT(*) FROM companion_events WHERE payload LIKE '%xxxxxxxxxx%'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
    assert_eq!(leaked, 0, "a payload survived");
    assert_eq!(
        free, 0,
        "free pages left after the purge: VACUUM did not run"
    );
    let bytes = std::fs::read(tmp.path().join("m.db")).unwrap();
    assert!(
        !bytes.windows(64).any(|w| w.iter().all(|b| *b == b'x')),
        "the purged text is still in the database file"
    );
    println!("checked 20 purged rows, {free} free pages, no purged bytes in the file");
}
