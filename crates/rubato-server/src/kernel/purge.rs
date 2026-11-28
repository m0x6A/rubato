//! Purging a tenant's rows of repositories that left it (ADR 0087, design §3.3): what an
//! operator runs on a detached installation. The rows were kept, hidden, since the detach; a
//! purge counts them, and then deletes them, table by table in one transaction of the caller's.
//!
//! Which tables: every table that has both a `tenant_id` and a `repo` column, read from the
//! schema itself, so a table added later is purged without being listed here; and the tables
//! in [`BY_REFERENCE`], whose rows hang on one of those by an id. A test compares the two with
//! the schema, so a new tenant-owned table with neither a `repo` column nor an entry there
//! fails.

use anyhow::Result;
use sqlx::SqliteConnection;
use std::collections::BTreeMap;

/// Tenant-owned tables without a `repo` column whose rows belong to a row that has one:
/// `(table, its column, the parent table, the parent's id column)`. Deleted before the parents.
pub const BY_REFERENCE: &[(&str, &str, &str, &str)] = &[
    ("plan_draft_revisions", "draft_id", "plan_drafts", "id"),
    ("plan_links", "approval_id", "rubato_approvals", "id"),
    ("agent_reports", "launch_id", "launches", "id"),
    ("fleet_requests", "lead_launch_id", "launches", "id"),
];

/// The tenancy tables themselves: their rows are the tombstones, removed last by the caller.
pub const TENANCY_TABLES: &[&str] = &["tenant_installations", "tenant_members", "tenant_repos"];

/// Every table with a `tenant_id` and a `repo` column, the tenancy tables left out.
pub async fn repo_tables(conn: &mut SqliteConnection) -> Result<Vec<String>> {
    let all: Vec<String> = sqlx::query_scalar(
        "SELECT m.name FROM sqlite_master m WHERE m.type = 'table' \
         AND EXISTS (SELECT 1 FROM pragma_table_info(m.name) WHERE name = 'tenant_id') \
         AND EXISTS (SELECT 1 FROM pragma_table_info(m.name) WHERE name = 'repo') \
         ORDER BY m.name",
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(all
        .into_iter()
        .filter(|t| !TENANCY_TABLES.contains(&t.as_str()))
        .collect())
}

/// The rows `tenant` holds of `repos`, per table; with `delete`, deleted as they are counted.
/// Tables with no such row are left out of the answer. The caller holds the transaction.
pub async fn rows_of_repos(
    conn: &mut SqliteConnection,
    tenant: &str,
    repos: &[String],
    delete: bool,
) -> Result<BTreeMap<String, u64>> {
    let mut out: BTreeMap<String, u64> = BTreeMap::new();
    let tables = repo_tables(conn).await?;
    for repo in repos {
        // The rows that hang on a parent go first, while the parent still says which they are.
        for (table, column, parent, id) in BY_REFERENCE {
            let of_repo = format!(
                "tenant_id = ? AND {column} IN (SELECT {id} FROM {parent} WHERE tenant_id = ? AND repo = ? COLLATE NOCASE)"
            );
            let n: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE {of_repo}"))
                    .bind(tenant)
                    .bind(tenant)
                    .bind(repo)
                    .fetch_one(&mut *conn)
                    .await?;
            if n > 0 {
                *out.entry((*table).to_owned()).or_default() += n as u64;
            }
            if delete && n > 0 {
                sqlx::query(&format!("DELETE FROM {table} WHERE {of_repo}"))
                    .bind(tenant)
                    .bind(tenant)
                    .bind(repo)
                    .execute(&mut *conn)
                    .await?;
            }
        }
        for table in &tables {
            let n: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE tenant_id = ? AND repo = ? COLLATE NOCASE"
            ))
            .bind(tenant)
            .bind(repo)
            .fetch_one(&mut *conn)
            .await?;
            if n > 0 {
                *out.entry(table.clone()).or_default() += n as u64;
            }
            if delete && n > 0 {
                sqlx::query(&format!(
                    "DELETE FROM {table} WHERE tenant_id = ? AND repo = ? COLLATE NOCASE"
                ))
                .bind(tenant)
                .bind(repo)
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::db::connect;

    /// Every table with a `tenant_id` is purged by its `repo` column, hangs on one that is
    /// ([`BY_REFERENCE`]), is a tenancy table, or is a workspace's (deleted with the workspace,
    /// `kernel::workspaces` and `workspace_settings`). A table that is none of these fails here.
    #[tokio::test]
    async fn every_tenant_owned_table_has_a_way_out() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = connect(&format!("sqlite://{}/t.db?mode=rwc", tmp.path().display()))
            .await
            .unwrap();
        let mut conn = pool.acquire().await.unwrap();
        let by_repo = repo_tables(&mut conn).await.unwrap();
        let with_tenant: Vec<String> = sqlx::query_scalar(
            "SELECT m.name FROM sqlite_master m WHERE m.type = 'table' \
             AND EXISTS (SELECT 1 FROM pragma_table_info(m.name) WHERE name = 'tenant_id') ORDER BY m.name",
        )
        .fetch_all(&mut *conn)
        .await
        .unwrap();
        let workspace_only: Vec<&str> = crate::kernel::workspaces::SCOPED_TABLES
            .iter()
            .copied()
            // A workspace's settings row goes when the workspace is deleted.
            .chain(["workspace_settings"])
            .collect();
        let mut homeless = Vec::new();
        for t in &with_tenant {
            let known = by_repo.contains(t)
                || BY_REFERENCE.iter().any(|(table, ..)| table == t)
                || TENANCY_TABLES.contains(&t.as_str())
                || workspace_only.contains(&t.as_str());
            if !known {
                homeless.push(t.clone());
            }
        }
        assert!(homeless.is_empty(), "no purge reaches {homeless:?}");
        for (table, column, parent, id) in BY_REFERENCE {
            for (t, c) in [(table, column), (parent, id)] {
                let has: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?")
                        .bind(t)
                        .bind(c)
                        .fetch_one(&mut *conn)
                        .await
                        .unwrap();
                assert_eq!(has, 1, "{t}.{c}");
            }
            assert!(by_repo.iter().any(|t| t == parent), "{parent} has no repo");
        }
        println!(
            "checked {} tables with a tenant: {} by repository, {} by reference",
            with_tenant.len(),
            by_repo.len(),
            BY_REFERENCE.len()
        );
    }
}
