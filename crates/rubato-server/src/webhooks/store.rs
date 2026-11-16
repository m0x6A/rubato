//! Table `webhook_deliveries` (migration 0026): one row per verified delivery, by its id and by
//! its body's hash, with what it names and no payload (design 208 D4, D9).

use super::event::Delivery;
use anyhow::Result;
use sqlx::{Row, SqlitePool};

/// Refreshes tried for a row before it is left to the catch-up (D9).
pub const MAX_ATTEMPTS: i64 = 3;
/// Rows are kept this long.
pub const KEEP_SECS: i64 = 7 * 24 * 3600;

/// Store a verified delivery once. `false`: its id or its body was seen before.
pub async fn insert(
    db: &SqlitePool,
    id: &str,
    body_sha256: &str,
    d: &Delivery,
    now: i64,
) -> Result<bool> {
    let outcome = if d.ignored.is_some() {
        "ignored"
    } else {
        "accepted"
    };
    let refreshed = d.ignored.is_some().then_some(now);
    let r = sqlx::query(
        "INSERT OR IGNORE INTO webhook_deliveries \
         (delivery_id, body_sha256, event, action, repo, issue_number, branch, received_at, outcome, refreshed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(body_sha256)
    .bind(&d.event)
    .bind(&d.action)
    .bind(&d.repo)
    .bind(d.issue_number.map(|n| n as i64))
    .bind(&d.branch)
    .bind(now)
    .bind(outcome)
    .bind(refreshed)
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// An accepted delivery not yet refreshed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub delivery_id: String,
    pub event: String,
    pub repo: Option<String>,
    pub issue_number: Option<u64>,
    pub branch: Option<String>,
    pub attempts: i64,
}

/// Oldest first, at most `limit`.
pub async fn pending(db: &SqlitePool, limit: i64) -> Result<Vec<Pending>> {
    let rows = sqlx::query(
        "SELECT delivery_id, event, repo, issue_number, branch, attempts FROM webhook_deliveries \
         WHERE refreshed_at IS NULL ORDER BY received_at, rowid LIMIT ?",
    )
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Pending {
            delivery_id: r.get("delivery_id"),
            event: r.get("event"),
            repo: r.get("repo"),
            issue_number: r.get::<Option<i64>, _>("issue_number").map(|n| n as u64),
            branch: r.get("branch"),
            attempts: r.get("attempts"),
        })
        .collect())
}

/// Mark these rows refreshed.
pub async fn mark(db: &SqlitePool, ids: &[String], now: i64) -> Result<()> {
    for id in ids {
        sqlx::query(
            "UPDATE webhook_deliveries SET refreshed_at = ? WHERE delivery_id = ? AND refreshed_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(db)
        .await?;
    }
    Ok(())
}

/// One more failed refresh for this row; at `MAX_ATTEMPTS` it is marked and left to the
/// catch-up. Returns whether it was given up.
pub async fn failed(db: &SqlitePool, id: &str, now: i64) -> Result<bool> {
    let row = sqlx::query(
        "UPDATE webhook_deliveries SET attempts = attempts + 1, \
         refreshed_at = CASE WHEN attempts + 1 >= ? THEN ? ELSE NULL END \
         WHERE delivery_id = ? AND refreshed_at IS NULL RETURNING refreshed_at",
    )
    .bind(MAX_ATTEMPTS)
    .bind(now)
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(row.is_some_and(|r| r.get::<Option<i64>, _>("refreshed_at").is_some()))
}

/// Delete rows received before `before`. Returns how many.
pub async fn prune(db: &SqlitePool, before: i64) -> Result<u64> {
    let r = sqlx::query("DELETE FROM webhook_deliveries WHERE received_at < ?")
        .bind(before)
        .execute(db)
        .await?;
    Ok(r.rows_affected())
}

/// Unix seconds of the newest accepted delivery, if any.
pub async fn last_accepted_at(db: &SqlitePool) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT MAX(received_at) FROM webhook_deliveries WHERE outcome = 'accepted'",
    )
    .fetch_one(db)
    .await?)
}

/// Unix seconds the oldest pending delivery was received, if any.
pub async fn oldest_pending_at(db: &SqlitePool) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT MIN(received_at) FROM webhook_deliveries WHERE refreshed_at IS NULL",
    )
    .fetch_one(db)
    .await?)
}

/// Rows stored, for tests and the refresh log line.
#[cfg(test)]
pub async fn count(db: &SqlitePool) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM webhook_deliveries")
            .fetch_one(db)
            .await?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(event: &str, issue: Option<u64>, ignored: Option<&'static str>) -> Delivery {
        Delivery {
            event: event.into(),
            action: Some("opened".into()),
            repo: Some("m0x6A/rubato".into()),
            issue_number: issue,
            branch: None,
            ignored,
        }
    }

    #[tokio::test]
    async fn rows_are_stored_once_marked_and_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let db = crate::kernel::db::connect(&url).await.unwrap();
        let d = delivery("issues", Some(5), None);
        assert!(insert(&db, "a", "h1", &d, 100).await.unwrap());
        assert!(!insert(&db, "a", "h2", &d, 101).await.unwrap(), "same id");
        assert!(!insert(&db, "b", "h1", &d, 101).await.unwrap(), "same body");
        assert!(insert(&db, "c", "h3", &d, 102).await.unwrap());
        let ignored = delivery("ping", None, Some("ping"));
        assert!(insert(&db, "p", "h4", &ignored, 103).await.unwrap());
        assert_eq!(count(&db).await.unwrap(), 3);
        let p = pending(&db, 200).await.unwrap();
        assert_eq!(
            p.iter().map(|r| r.delivery_id.as_str()).collect::<Vec<_>>(),
            ["a", "c"],
            "oldest first; an ignored row is never pending"
        );
        assert_eq!(last_accepted_at(&db).await.unwrap(), Some(102));
        assert_eq!(oldest_pending_at(&db).await.unwrap(), Some(100));
        mark(&db, &["a".into()], 110).await.unwrap();
        assert_eq!(oldest_pending_at(&db).await.unwrap(), Some(102));
        assert!(!failed(&db, "c", 111).await.unwrap());
        assert!(!failed(&db, "c", 112).await.unwrap());
        assert!(
            failed(&db, "c", 113).await.unwrap(),
            "third failure gives up"
        );
        assert_eq!(oldest_pending_at(&db).await.unwrap(), None);
        assert_eq!(prune(&db, 101).await.unwrap(), 1);
        assert_eq!(count(&db).await.unwrap(), 2);
        println!("checked 14 store steps");
    }
}
