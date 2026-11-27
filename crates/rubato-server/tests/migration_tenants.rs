//! Plan 345 (design §8, §11.3): the migrations that give tenant-owned tables their `tenant_id`.
//! From a database as it was before them, every row lands in tenant `default`, nothing is lost,
//! and no column keeps a default: an insert that forgets the tenant fails instead of landing in
//! the first team's tenant.

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

/// One table of migration 0034: its name, the insert that fills it at 0033 (two workspaces where
/// the key allows), and an insert after the migration that leaves `tenant_id` out.
struct Table {
    name: &'static str,
    before: String,
    rows: i64,
    without_tenant: String,
}

fn workspace_tables() -> Vec<Table> {
    let t = |name: &'static str, before: String, rows: i64, without_tenant: String| Table {
        name,
        before,
        rows,
        without_tenant,
    };
    vec![
        t(
            "workspace_settings",
            format!("INSERT INTO workspace_settings (workspace_id, position, revision, map_repo, map_path, map_ref, updated_by, updated_at) VALUES ('default', 0, 3, 'a/b', 'm.yml', 'main', 'u', '{T}'), ('shop', 1, 1, 'a/c', 'm.yml', 'main', 'u', '{T}')"),
            2,
            format!("INSERT INTO workspace_settings (workspace_id, position, map_repo, map_path, updated_by, updated_at) VALUES ('new', 9, 'a/d', 'm.yml', 'u', '{T}')"),
        ),
        t(
            "map_layout",
            format!("INSERT INTO map_layout (workspace_id, node_id, x, y, updated_by, updated_at) VALUES ('default', 'web', 1.5, -2, 'u', '{T}'), ('shop', 'web', 3, 4, 'u', '{T}')"),
            2,
            format!("INSERT INTO map_layout (workspace_id, node_id, x, y, updated_by, updated_at) VALUES ('new', 'web', 1, 1, 'u', '{T}')"),
        ),
        t(
            "map_layout_grouped",
            format!("INSERT INTO map_layout_grouped (workspace_id, kind, id, x, y, updated_by, updated_at) VALUES ('default', 'part', 'web', 1, 2, 'u', '{T}'), ('default', 'box', 'a/b', 3, 4, 'u', '{T}')"),
            2,
            format!("INSERT INTO map_layout_grouped (workspace_id, kind, id, x, y, updated_by, updated_at) VALUES ('new', 'part', 'web', 1, 1, 'u', '{T}')"),
        ),
        t(
            "map_cache",
            format!("INSERT INTO map_cache (workspace_id, version, yaml, loaded_at) VALUES ('default', 'sha', 'name: x', '{T}'), ('shop', 'sha2', 'name: y', '{T}')"),
            2,
            format!("INSERT INTO map_cache (workspace_id, version, yaml, loaded_at) VALUES ('new', 's', 'y', '{T}')"),
        ),
        t(
            "map_proposal",
            format!("INSERT INTO map_proposal (workspace_id, branch, base_sha, file_hash, last_seq, state, state_since, proposed_by) VALUES ('default', 'b', 's', 'h', 1, 'proposed', '{T}', 'u')"),
            1,
            format!("INSERT INTO map_proposal (workspace_id, branch, base_sha, file_hash, last_seq, state, state_since, proposed_by) VALUES ('new', 'b', 's', 'h', 1, 'opening', '{T}', 'u')"),
        ),
        t(
            "map_draft_ops",
            format!("INSERT INTO map_draft_ops (op_id, op, author, created_at, workspace_id) VALUES ('o1', '{{}}', 'u', '{T}', 'default'), ('o2', '{{}}', 'u', '{T}', 'shop')"),
            2,
            format!("INSERT INTO map_draft_ops (op_id, op, author, created_at, workspace_id) VALUES ('o9', '{{}}', 'u', '{T}', 'new')"),
        ),
        t(
            "pins",
            format!("INSERT INTO pins (workspace_id, repo, number, kind, target, pinned_by, pinned_at) VALUES ('default', 'a/b', 1, 'node', 'x', 'u', '{T}'), ('shop', 'a/b', 1, 'node', 'x', 'u', '{T}')"),
            2,
            format!("INSERT INTO pins (workspace_id, repo, number, kind, target, pinned_by, pinned_at) VALUES ('new', 'a/b', 2, 'node', 'x', 'u', '{T}')"),
        ),
        t(
            "map_line_pins",
            format!("INSERT INTO map_line_pins (workspace_id, repo, number, kind, target, outcome, applied_at) VALUES ('default', 'a/b', 3, 'node', 'x', 'pinned', '{T}')"),
            1,
            format!("INSERT INTO map_line_pins (workspace_id, repo, number, kind, target, outcome, applied_at) VALUES ('new', 'a/b', 4, 'node', 'x', 'pinned', '{T}')"),
        ),
        t(
            "issue_creations",
            format!("INSERT INTO issue_creations (request_id, repo, kind, target, title, created_by, created_at, state, updated_at, workspace_id) VALUES ('r1', 'a/b', 'node', 'x', 't', 'u', '{T}', 'opening', '{T}', 'shop')"),
            1,
            format!("INSERT INTO issue_creations (request_id, repo, kind, target, title, created_by, created_at, state, updated_at, workspace_id) VALUES ('r9', 'a/b', 'node', 'x', 't', 'u', '{T}', 'opening', '{T}', 'new')"),
        ),
        t(
            "issue_suggestions",
            "INSERT INTO issue_suggestions (workspace_id, repo, number, suggestion) VALUES ('default', 'a/b', 9, '{}'), ('shop', 'a/b', 9, '{}')".to_owned(),
            2,
            "INSERT INTO issue_suggestions (workspace_id, repo, number, suggestion) VALUES ('new', 'a/b', 8, '{}')".to_owned(),
        ),
        t(
            "traffic_dismissals",
            format!("INSERT INTO traffic_dismissals (workspace_id, target_key, author, created_at) VALUES ('default', 'k1', 'u', '{T}'), ('shop', 'k1', 'u', '{T}')"),
            2,
            format!("INSERT INTO traffic_dismissals (workspace_id, target_key, author, created_at) VALUES ('new', 'k9', 'u', '{T}')"),
        ),
    ]
}

/// Check the tables of one migration: rows kept, all in `default`, the column required and
/// without a default, and an insert that leaves it out refused.
async fn check(pool: &SqlitePool, tables: &[Table]) {
    for t in tables {
        let (n, in_default): (i64, i64) = sqlx::query_as(&format!(
            "SELECT COUNT(*), COALESCE(SUM(tenant_id = 'default'), 0) FROM {}",
            t.name
        ))
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(n, t.rows, "{}: every row is kept", t.name);
        assert_eq!(
            in_default, t.rows,
            "{}: every row is in tenant default",
            t.name
        );
        let col = sqlx::query(&format!("PRAGMA table_info({})", t.name))
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.get::<String, _>("name") == "tenant_id")
            .unwrap_or_else(|| panic!("{} has no tenant_id", t.name));
        assert_eq!(col.get::<i64, _>("notnull"), 1, "{}: required", t.name);
        assert_eq!(
            col.get::<Option<String>, _>("dflt_value"),
            None,
            "{}: no default, so a forgotten tenant fails",
            t.name
        );
        let e = sqlx::query(&t.without_tenant)
            .execute(pool)
            .await
            .expect_err(t.name)
            .to_string();
        assert!(e.contains("tenant_id"), "{}: {e}", t.name);
    }
}

#[tokio::test]
async fn workspace_rows_land_in_tenant_default_and_the_column_has_no_default() {
    let (pool, _tmp) = at(33).await;
    let tables = workspace_tables();
    for t in &tables {
        sqlx::query(&t.before)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", t.name));
    }
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    check(&pool, &tables).await;

    // What the rows said is unchanged: a settings revision, a draft op's sequence number.
    let rev: i64 = sqlx::query_scalar(
        "SELECT revision FROM workspace_settings WHERE tenant_id = 'default' AND workspace_id = 'default'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rev, 3);
    let seqs: Vec<i64> = sqlx::query_scalar("SELECT seq FROM map_draft_ops ORDER BY seq")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(seqs, [1, 2]);
    // The same workspace id in another tenant is another row, not a conflict.
    sqlx::query(&format!("INSERT INTO pins (tenant_id, workspace_id, repo, number, kind, target, pinned_by, pinned_at) VALUES ('acme', 'default', 'a/b', 1, 'node', 'x', 'u', '{T}')"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(&format!("INSERT INTO workspace_settings (tenant_id, workspace_id, position, map_repo, map_path, updated_by, updated_at) VALUES ('acme', 'default', 0, 'acme/map', 'm.yml', 'u', '{T}')"))
        .execute(&pool)
        .await
        .unwrap();
    println!("checked {} tables of migration 0034", tables.len());
}
