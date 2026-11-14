//! Migration 0013 (plan 89, ADR 0047): every row of every rebuilt table survives at workspace
//! `default`, the suggestions move to `issue_suggestions`, and a rebuilt table refuses a row that
//! names no workspace, so a store call that forgets to bind it fails instead of writing `default`.

use sqlx::{Row, SqlitePool};
use std::borrow::Cow;

async fn at_0012() -> (SqlitePool, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/m.db?mode=rwc", tmp.path().display());
    // One connection: the migration and the checks see one schema.
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect(&url).await.unwrap();
    let mut m = sqlx::migrate!("./migrations");
    m.migrations = Cow::Owned(
        m.migrations
            .iter()
            .filter(|x| x.version <= 12)
            .cloned()
            .collect(),
    );
    m.run(&pool).await.unwrap();
    (pool, tmp)
}

const T: &str = "2026-11-14T08:00:00+00:00";

#[tokio::test]
async fn every_row_survives_at_default() {
    let (pool, _tmp) = at_0012().await;
    let seed = [
        format!(
            "INSERT INTO pins VALUES ('a/b', 1, 'node', 'x', 'u', '{T}'), ('a/b', 2, 'seam', 's', 'u', '{T}')"
        ),
        format!("INSERT INTO map_line_pins VALUES ('a/b', 3, 'node', 'x', 'pinned', '{T}')"),
        format!("INSERT INTO map_layout VALUES ('x', 1, 2, 'u', '{T}'), ('y', 3, 4, 'u', '{T}')"),
        format!("INSERT INTO map_cache VALUES (1, 'sha', 'name: x', '{T}')"),
        format!(
            "INSERT INTO map_proposal VALUES (1, 'b', 's', 'h', 1, 'proposed', '{T}', NULL, NULL, 'u')"
        ),
        format!(
            "INSERT INTO map_draft_ops (op_id, op, author, created_at) VALUES ('o1', '{{}}', 'u', '{T}')"
        ),
        format!(
            "INSERT INTO launches VALUES ('l1', 'c', 'u', 'a/b', 1, '{{}}', NULL, '{T}', NULL)"
        ),
        format!(
            "INSERT INTO open_issues (repo, number, title, url, labels, updated_at, map_line, suggestion) VALUES ('a/b', 9, 't', 'u', '[]', '{T}', NULL, '{{\"target\":{{\"kind\":\"node\",\"id\":\"x\"}},\"score\":80}}'), ('a/b', 10, 't', 'u', '[]', '{T}', NULL, NULL)"
        ),
    ];
    for sql in &seed {
        sqlx::query(sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    let count = |t: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(&format!("SELECT COUNT(*) AS n FROM {t}"))
                .fetch_one(&pool)
                .await
                .unwrap()
                .get::<i64, _>("n")
        }
    };
    let tables = [
        "pins",
        "map_line_pins",
        "map_layout",
        "map_cache",
        "map_proposal",
        "map_draft_ops",
        "launches",
        "open_issues",
    ];
    let mut before = Vec::new();
    for t in tables {
        before.push(count(t).await);
    }

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    for (t, n) in tables.iter().zip(&before) {
        assert_eq!(count(t).await, *n, "{t} kept its rows");
        if *t != "open_issues" {
            let other: i64 = sqlx::query(&format!(
                "SELECT COUNT(*) AS n FROM {t} WHERE workspace_id <> 'default'"
            ))
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n");
            assert_eq!(other, 0, "{t}: every row is default");
        }
    }
    let moved: Vec<(String, i64)> =
        sqlx::query("SELECT workspace_id, number FROM issue_suggestions")
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get("workspace_id"), r.get("number")))
            .collect();
    assert_eq!(
        moved,
        [("default".to_string(), 9)],
        "the one suggestion moved"
    );
    let left: i64 =
        sqlx::query("SELECT COUNT(*) AS n FROM open_issues WHERE suggestion IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n");
    assert_eq!(left, 0);
    println!(
        "migration 0013: {} tables checked, {} rows",
        tables.len(),
        before.iter().sum::<i64>()
    );
}

#[tokio::test]
async fn a_rebuilt_table_refuses_a_row_without_a_workspace() {
    let (pool, _tmp) = at_0012().await;
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let missing = [
        format!(
            "INSERT INTO pins (repo, number, kind, target, pinned_by, pinned_at) VALUES ('a/b', 1, 'node', 'x', 'u', '{T}')"
        ),
        format!(
            "INSERT INTO map_line_pins (repo, number, kind, target, outcome, applied_at) VALUES ('a/b', 1, 'node', 'x', 'pinned', '{T}')"
        ),
        format!(
            "INSERT INTO map_layout (node_id, x, y, updated_by, updated_at) VALUES ('x', 1, 2, 'u', '{T}')"
        ),
        format!("INSERT INTO map_cache (version, yaml, loaded_at) VALUES ('sha', 'y', '{T}')"),
        format!(
            "INSERT INTO map_proposal (branch, base_sha, file_hash, last_seq, state, state_since, proposed_by) VALUES ('b', 's', 'h', 1, 'opening', '{T}', 'u')"
        ),
    ];
    for sql in &missing {
        let e = sqlx::query(sql).execute(&pool).await.expect_err(sql);
        assert!(e.to_string().contains("NOT NULL"), "{sql}: {e}");
    }
    println!(
        "{} rebuilt tables refuse a row without a workspace",
        missing.len()
    );
}

#[tokio::test]
async fn rows_of_a_workspace_no_longer_configured_are_counted_and_kept() {
    use rubato_server::kernel::{config::Config, github::FakeGitHub, state::AppState, workspaces};
    let (pool, tmp) = at_0012().await;
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    sqlx::query(&format!("INSERT INTO map_layout VALUES ('gone', 'x', 1, 2, 'u', '{T}'), ('default', 'x', 1, 2, 'u', '{T}')"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(&format!(
        "INSERT INTO pins VALUES ('gone', 'a/b', 1, 'node', 'x', 'u', '{T}')"
    ))
    .execute(&pool)
    .await
    .unwrap();
    let url = format!("sqlite://{}/m.db", tmp.path().display());
    let app = AppState::new(
        pool.clone(),
        Config::for_tests(url, "u"),
        std::sync::Arc::new(FakeGitHub::new()),
    );
    assert_eq!(
        workspaces::report(&app).await.unwrap(),
        2,
        "two rows of workspace gone"
    );
    let kept: i64 = sqlx::query("SELECT COUNT(*) AS n FROM map_layout")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("n");
    assert_eq!(kept, 2, "orphans are kept, never deleted");
}
