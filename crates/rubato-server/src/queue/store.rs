//! Table `companion_queue`.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{IssueRef, QueuedIssue};
use sqlx::{Row, SqliteConnection, SqlitePool};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Every queued issue, lane by lane, each lane in its order.
pub async fn all(db: &SqlitePool) -> Result<Vec<QueuedIssue>> {
    let rows = sqlx::query("SELECT * FROM companion_queue ORDER BY companion_id, position")
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .map(|r| QueuedIssue {
            issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
            companion_id: Uuid::parse_str(&r.get::<String, _>("companion_id")).unwrap_or_default(),
            queued_by: r.get("queued_by"),
            queued_at: parse_ts(&r.get::<String, _>("queued_at")),
            position: r.get::<i64, _>("position") as u32,
            last_start_error: r.get("last_start_error"),
        })
        .collect())
}

async fn lane(conn: &mut SqliteConnection, companion_id: Uuid) -> Result<Vec<IssueRef>> {
    let rows = sqlx::query(
        "SELECT repo, number FROM companion_queue WHERE companion_id = ? ORDER BY position",
    )
    .bind(companion_id.to_string())
    .fetch_all(conn)
    .await?;
    Ok(rows
        .iter()
        .map(|r| IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64))
        .collect())
}

/// Write `order` as positions 0.. of the lane.
async fn renumber(
    conn: &mut SqliteConnection,
    companion_id: Uuid,
    order: &[IssueRef],
) -> Result<()> {
    for (i, issue) in order.iter().enumerate() {
        sqlx::query(
            "UPDATE companion_queue SET position = ? WHERE companion_id = ? AND repo = ? AND number = ?",
        )
        .bind(i as i64)
        .bind(companion_id.to_string())
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Drop the rows of lanes that no longer take writes (revoked companions).
async fn forget(conn: &mut SqliteConnection, gone: &[Uuid]) -> Result<()> {
    for c in gone {
        sqlx::query("DELETE FROM companion_queue WHERE companion_id = ?")
            .bind(c.to_string())
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Added {
    Yes,
    /// It was queued there already: nothing changed.
    Already,
    /// The lane holds `max` entries.
    Full,
}

/// Queue `issue` at the end of the lane. Count and insert run in one `BEGIN IMMEDIATE`
/// transaction: a plain (deferred) one would let two adds both read 49 and the loser fail with
/// `SQLITE_BUSY` on its write.
pub async fn add(
    db: &SqlitePool,
    companion_id: Uuid,
    issue: &IssueRef,
    by: &str,
    at: DateTime<Utc>,
    max: usize,
    revoked: &[Uuid],
) -> Result<Added> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    forget(&mut tx, revoked).await?;
    let now = lane(&mut tx, companion_id).await?;
    let outcome = if now.contains(issue) {
        Added::Already
    } else if now.len() >= max {
        Added::Full
    } else {
        sqlx::query("INSERT INTO companion_queue (companion_id, repo, number, position, queued_by, queued_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(companion_id.to_string())
            .bind(&issue.repo)
            .bind(issue.number as i64)
            .bind(now.len() as i64)
            .bind(by)
            .bind(ts(at))
            .execute(&mut *tx)
            .await?;
        Added::Yes
    };
    tx.commit().await?;
    Ok(outcome)
}

/// Put the lane in `order`. False, and nothing written, when `order` is not exactly the stored
/// set: an entry added in another tab is never dropped by a reorder.
pub async fn reorder(
    db: &SqlitePool,
    companion_id: Uuid,
    order: &[IssueRef],
    revoked: &[Uuid],
) -> Result<bool> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    forget(&mut tx, revoked).await?;
    let stored: BTreeSet<IssueRef> = lane(&mut tx, companion_id).await?.into_iter().collect();
    let wanted: BTreeSet<IssueRef> = order.iter().cloned().collect();
    let same = wanted == stored && wanted.len() == order.len();
    if same {
        renumber(&mut tx, companion_id, order).await?;
    }
    tx.commit().await?;
    Ok(same)
}

/// Take `issue` off the lane and close the gap. False when it was not queued there.
pub async fn remove(
    db: &SqlitePool,
    companion_id: Uuid,
    issue: &IssueRef,
    revoked: &[Uuid],
) -> Result<bool> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    forget(&mut tx, revoked).await?;
    let removed = delete(&mut tx, companion_id, issue).await?;
    tx.commit().await?;
    Ok(removed)
}

async fn delete(conn: &mut SqliteConnection, companion_id: Uuid, issue: &IssueRef) -> Result<bool> {
    let r = sqlx::query(
        "DELETE FROM companion_queue WHERE companion_id = ? AND repo = ? AND number = ?",
    )
    .bind(companion_id.to_string())
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .execute(&mut *conn)
    .await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    let rest = lane(conn, companion_id).await?;
    renumber(conn, companion_id, &rest).await?;
    Ok(true)
}

/// The companion reported a launch of `issue` as started: the entry leaves its queue. False
/// when the issue was not queued there (a start from the issue panel).
pub async fn launch_started(db: &SqlitePool, companion_id: Uuid, issue: &IssueRef) -> Result<bool> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let removed = delete(&mut tx, companion_id, issue).await?;
    tx.commit().await?;
    Ok(removed)
}

/// The companion refused the launch or failed it: the entry stays and keeps the reason. False
/// when the issue was not queued there.
pub async fn launch_not_started(
    db: &SqlitePool,
    companion_id: Uuid,
    issue: &IssueRef,
    why: &str,
) -> Result<bool> {
    let r = sqlx::query("UPDATE companion_queue SET last_start_error = ? WHERE companion_id = ? AND repo = ? AND number = ?")
        .bind(why)
        .bind(companion_id.to_string())
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .execute(db)
        .await?;
    Ok(r.rows_affected() > 0)
}
