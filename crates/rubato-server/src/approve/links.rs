//! Table `plan_links`: the link comment of each approval (ADR 0055), at most one per approval.

use crate::kernel::db::{parse_ts, ts};
use crate::kernel::tenancy::TenantId;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// Sent with no answer, or about to be sent: found by the comments pass, never sent again.
    Opening,
    Posted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRow {
    pub approval_id: Uuid,
    pub commit_sha: String,
    pub state: LinkState,
    pub comment_id: Option<u64>,
    pub url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub posted_at: Option<DateTime<Utc>>,
}

fn row(r: &SqliteRow) -> Result<LinkRow> {
    Ok(LinkRow {
        approval_id: r.get::<String, _>("approval_id").parse()?,
        commit_sha: r.get("commit_sha"),
        state: if r.get::<String, _>("state") == "posted" {
            LinkState::Posted
        } else {
            LinkState::Opening
        },
        comment_id: r.get::<Option<i64>, _>("comment_id").map(|i| i as u64),
        url: r.get("url"),
        created_at: parse_ts(&r.get::<String, _>("created_at")),
        posted_at: r
            .get::<Option<String>, _>("posted_at")
            .map(|t| parse_ts(&t)),
    })
}

/// Every link row, by approval.
pub async fn all(db: &SqlitePool, t: &TenantId) -> Result<HashMap<Uuid, LinkRow>> {
    let rows = sqlx::query(
        "SELECT approval_id, commit_sha, state, comment_id, url, created_at, posted_at FROM plan_links WHERE tenant_id = ?",
    )
    .bind(t.as_str()).fetch_all(db)
    .await?;
    rows.iter()
        .map(|r| row(r).map(|l| (l.approval_id, l)))
        .collect()
}

/// Claim the approval's link with an `opening` row before posting. False when a row exists:
/// another tick or task claimed it, and nothing is posted twice.
pub async fn open(
    db: &SqlitePool,
    t: &TenantId,
    approval_id: Uuid,
    commit_sha: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let n = sqlx::query(
        "INSERT OR IGNORE INTO plan_links (tenant_id, approval_id, commit_sha, state, created_at) VALUES (?, ?, ?, 'opening', ?)",
    )
    .bind(t.as_str()).bind(approval_id.to_string())
    .bind(commit_sha)
    .bind(ts(now))
    .execute(db)
    .await?
    .rows_affected();
    Ok(n == 1)
}

pub async fn posted(
    db: &SqlitePool,
    t: &TenantId,
    approval_id: Uuid,
    comment_id: u64,
    url: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "UPDATE plan_links SET state = 'posted', comment_id = ?, url = ?, posted_at = ? WHERE tenant_id = ? AND approval_id = ?",
    )
    .bind(comment_id as i64)
    .bind(url)
    .bind(ts(now))
    .bind(t.as_str()).bind(approval_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

/// GitHub refused the post: nothing exists on the issue, so the claim goes and a later tick
/// tries again.
pub async fn forget(db: &SqlitePool, t: &TenantId, approval_id: Uuid) -> Result<()> {
    sqlx::query(
        "DELETE FROM plan_links WHERE tenant_id = ? AND approval_id = ? AND state = 'opening'",
    )
    .bind(t.as_str())
    .bind(approval_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}
