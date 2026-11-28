//! Table `pr_merges` (migration 0025): one merge intent per (repo, pull request, head).

use crate::kernel::db::{parse_ts, ts};
use crate::kernel::tenancy::TenantId;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::pulls::{MergeMethod, MergeState};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub repo: String,
    pub number: u64,
    pub head_sha: String,
    pub login: String,
    pub method: MergeMethod,
    pub state: MergeState,
    pub merge_sha: Option<String>,
    pub listings_open: u32,
    pub opened_at: DateTime<Utc>,
}

fn method_of(s: &str) -> MergeMethod {
    match s {
        "merge" => MergeMethod::Merge,
        "rebase" => MergeMethod::Rebase,
        _ => MergeMethod::Squash,
    }
}

fn row(r: &sqlx::sqlite::SqliteRow) -> Intent {
    Intent {
        repo: r.get("repo"),
        number: r.get::<i64, _>("number") as u64,
        head_sha: r.get("head_sha"),
        login: r.get("login"),
        method: method_of(r.get::<String, _>("method").as_str()),
        state: if r.get::<String, _>("state") == "merged" {
            MergeState::Merged
        } else {
            MergeState::Opening
        },
        merge_sha: r.get("merge_sha"),
        listings_open: r.get::<i64, _>("listings_open") as u32,
        opened_at: parse_ts(&r.get::<String, _>("opened_at")),
    }
}

const COLUMNS: &str =
    "repo, number, head_sha, login, method, state, merge_sha, listings_open, opened_at";

pub async fn find(
    db: &SqlitePool,
    t: &TenantId,
    repo: &str,
    number: u64,
    head: &str,
) -> Result<Option<Intent>> {
    Ok(sqlx::query(&format!(
        "SELECT {COLUMNS} FROM pr_merges WHERE tenant_id = ? AND repo = ? AND number = ? AND head_sha = ?"
    ))
    .bind(t.as_str()).bind(repo)
    .bind(number as i64)
    .bind(head)
    .fetch_optional(db)
    .await?
    .as_ref()
    .map(row))
}

/// Insert the `opening` row; false when another request holds this head already.
pub async fn open(
    db: &SqlitePool,
    t: &TenantId,
    repo: &str,
    number: u64,
    head: &str,
    login: &str,
    method: MergeMethod,
) -> Result<bool> {
    let now = ts(Utc::now());
    let done = sqlx::query("INSERT OR IGNORE INTO pr_merges (tenant_id, repo, number, head_sha, login, method, state, opened_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 'opening', ?, ?)")
        .bind(t.as_str()).bind(repo).bind(number as i64).bind(head).bind(login).bind(method.as_str()).bind(&now).bind(&now)
        .execute(db).await?;
    Ok(done.rows_affected() == 1)
}

pub async fn merged(
    db: &SqlitePool,
    t: &TenantId,
    i: &Intent,
    merge_sha: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE pr_merges SET state = 'merged', merge_sha = ?, updated_at = ? WHERE tenant_id = ? AND repo = ? AND number = ? AND head_sha = ?")
        .bind(merge_sha).bind(ts(Utc::now())).bind(t.as_str()).bind(&i.repo).bind(i.number as i64).bind(&i.head_sha)
        .execute(db).await?;
    Ok(())
}

pub async fn forget(
    db: &SqlitePool,
    t: &TenantId,
    repo: &str,
    number: u64,
    head: &str,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM pr_merges WHERE tenant_id = ? AND repo = ? AND number = ? AND head_sha = ?",
    )
    .bind(t.as_str())
    .bind(repo)
    .bind(number as i64)
    .bind(head)
    .execute(db)
    .await?;
    Ok(())
}

/// One more complete listing showed it open at this head; the new count.
pub async fn seen_open(db: &SqlitePool, t: &TenantId, i: &Intent) -> Result<u32> {
    sqlx::query("UPDATE pr_merges SET listings_open = listings_open + 1, updated_at = ? WHERE tenant_id = ? AND repo = ? AND number = ? AND head_sha = ?")
        .bind(ts(Utc::now())).bind(t.as_str()).bind(&i.repo).bind(i.number as i64).bind(&i.head_sha)
        .execute(db).await?;
    Ok(i.listings_open + 1)
}

pub async fn opening(db: &SqlitePool, t: &TenantId) -> Result<Vec<Intent>> {
    Ok(sqlx::query(&format!(
        "SELECT {COLUMNS} FROM pr_merges WHERE tenant_id = ? AND state = 'opening' ORDER BY opened_at"
    ))
    .bind(t.as_str()).fetch_all(db)
    .await?
    .iter()
    .map(row)
    .collect())
}

/// Drop `merged` rows older than `before` (design 82 A9); how many.
pub async fn prune_merged(db: &SqlitePool, t: &TenantId, before: DateTime<Utc>) -> Result<u64> {
    Ok(sqlx::query(
        "DELETE FROM pr_merges WHERE tenant_id = ? AND state = 'merged' AND updated_at < ?",
    )
    .bind(t.as_str())
    .bind(ts(before))
    .execute(db)
    .await?
    .rows_affected())
}
