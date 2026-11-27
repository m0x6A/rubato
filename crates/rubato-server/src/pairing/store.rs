//! Tables `pairing_codes` and `pairings`. `approve_pairing_code` writes both in one transaction.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::pairing::{PairPollResponse, PairStartRequest};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

pub fn token_hash(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone)]
pub struct PairingRow {
    pub companion_id: Uuid,
    pub user: String,
    pub hostname: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
    pub server_seq: u64,
    pub last_companion_seq: u64,
    /// Interactive or headless, from its last Hello (ADR 0062, migration 0023).
    pub client: rubato_protocol::ClientKind,
}

fn pairing_row(r: &sqlx::sqlite::SqliteRow) -> PairingRow {
    PairingRow {
        companion_id: Uuid::parse_str(&r.get::<String, _>("companion_id")).unwrap_or_default(),
        user: r.get("user"),
        hostname: r.get("hostname"),
        created_at: parse_ts(&r.get::<String, _>("created_at")),
        revoked_at: r
            .get::<Option<String>, _>("revoked_at")
            .map(|s| parse_ts(&s)),
        last_seen: r
            .get::<Option<String>, _>("last_seen")
            .map(|s| parse_ts(&s)),
        server_seq: r.get::<i64, _>("server_seq") as u64,
        last_companion_seq: r.get::<i64, _>("last_companion_seq") as u64,
        client: if r.try_get::<String, _>("client").ok().as_deref() == Some("headless") {
            rubato_protocol::ClientKind::Headless
        } else {
            rubato_protocol::ClientKind::Interactive
        },
    }
}

pub async fn create_pairing_code(
    db: &SqlitePool,
    req: &PairStartRequest,
    device_code: &str,
    user_code: &str,
    ttl_secs: i64,
    max_pending: i64,
) -> Result<bool> {
    let now = Utc::now();
    // The cap is checked in the insert itself, so concurrent starts cannot overshoot it (#168).
    let r = sqlx::query("INSERT INTO pairing_codes (device_code, user_code, companion_id, hostname, companion_version, status, created_at, expires_at) SELECT ?, ?, ?, ?, ?, 'pending', ?, ? WHERE (SELECT COUNT(*) FROM pairing_codes WHERE status = 'pending' AND expires_at > ?) < ?")
        .bind(device_code).bind(user_code).bind(req.companion_id.to_string()).bind(&req.hostname).bind(&req.companion_version).bind(ts(now)).bind(ts(now + chrono::Duration::seconds(ttl_secs)))
        .bind(ts(now)).bind(max_pending)
        .execute(db).await?;
    Ok(r.rows_affected() == 1)
}

/// How long after its expiry a code is kept, so a late poll still reads `Expired` (#168).
pub const PAIRING_CODE_KEEP_SECS: i64 = 3600;

/// Delete the codes that expired more than [`PAIRING_CODE_KEEP_SECS`] before `now`, whatever
/// their status. Returns how many went.
pub async fn sweep_codes(db: &SqlitePool, now: DateTime<Utc>) -> Result<u64> {
    let cutoff = ts(now - chrono::Duration::seconds(PAIRING_CODE_KEEP_SECS));
    Ok(
        sqlx::query("DELETE FROM pairing_codes WHERE expires_at < ?")
            .bind(cutoff)
            .execute(db)
            .await?
            .rows_affected(),
    )
}

/// Claim a code for `user`: only a pending, unexpired code is claimed, so of two approvals that
/// both read it pending, one wins and the other changes nothing (#168).
async fn claim_code(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    device_code: &str,
    user: &str,
    token: &str,
) -> Result<bool> {
    let r = sqlx::query(
        "UPDATE pairing_codes SET status = 'approved', user = ?, token = ? WHERE device_code = ? AND status = 'pending' AND expires_at > ?",
    )
    .bind(user)
    .bind(token)
    .bind(device_code)
    .bind(ts(Utc::now()))
    .execute(&mut **tx)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// What an approval did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// The pairing was created, or the approver's own pairing replaced.
    Approved(Uuid),
    /// No pending, unexpired code with that user code.
    NotFound,
    /// The companion id is paired to another person (live or revoked). Nothing about that
    /// pairing changed, and the code is now `denied` (#160).
    Taken(Uuid),
}

/// The human approved a code. Creates the pairing, or replaces the approver's own, and parks the
/// token for one poll. A companion id stays with the user who first paired it (ADR 0011).
pub async fn approve_pairing_code(
    db: &SqlitePool,
    user_code: &str,
    user: &str,
    token: &str,
) -> Result<Approval> {
    let Some(row) = sqlx::query("SELECT device_code, companion_id, hostname, expires_at, status FROM pairing_codes WHERE user_code = ?").bind(user_code).fetch_optional(db).await? else {
        return Ok(Approval::NotFound);
    };
    let status: String = row.get("status");
    let expires = parse_ts(&row.get::<String, _>("expires_at"));
    if status != "pending" || expires < Utc::now() {
        return Ok(Approval::NotFound);
    }
    let companion_id: String = row.get("companion_id");
    let Ok(id) = Uuid::parse_str(&companion_id) else {
        return Ok(Approval::NotFound);
    };
    let hostname: String = row.get("hostname");
    let device_code: String = row.get("device_code");
    let mut tx = db.begin().await?;
    if !claim_code(&mut tx, &device_code, user, token).await? {
        // Another approval claimed it first, or it expired since the read (#168).
        return Ok(Approval::NotFound);
    }
    // One statement decides ownership: the update applies only to a row of the same user, so
    // two approvals cannot both pass a read and then write.
    let upsert = sqlx::query("INSERT INTO pairings (companion_id, user, token_hash, hostname, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(companion_id) DO UPDATE SET token_hash = excluded.token_hash, hostname = excluded.hostname, revoked_at = NULL WHERE pairings.user = excluded.user")
        .bind(&companion_id).bind(user).bind(token_hash(token)).bind(&hostname).bind(ts(Utc::now())).execute(&mut *tx).await?;
    if upsert.rows_affected() == 0 {
        // Taken (#160): the claim above parked a token; a denied code keeps none.
        sqlx::query(
            "UPDATE pairing_codes SET status = 'denied', user = ?, token = NULL WHERE device_code = ?",
        )
        .bind(user)
        .bind(&device_code)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(Approval::Taken(id));
    }
    tx.commit().await?;
    Ok(Approval::Approved(id))
}

pub async fn poll_pairing_code(db: &SqlitePool, device_code: &str) -> Result<PairPollResponse> {
    let Some(row) = sqlx::query(
        "SELECT status, user, token, expires_at FROM pairing_codes WHERE device_code = ?",
    )
    .bind(device_code)
    .fetch_optional(db)
    .await?
    else {
        return Ok(PairPollResponse::Denied);
    };
    let status: String = row.get("status");
    if parse_ts(&row.get::<String, _>("expires_at")) < Utc::now() && status == "pending" {
        sqlx::query("UPDATE pairing_codes SET status = 'expired' WHERE device_code = ?")
            .bind(device_code)
            .execute(db)
            .await?;
        return Ok(PairPollResponse::Expired);
    }
    match status.as_str() {
        "pending" => Ok(PairPollResponse::Pending),
        "approved" => {
            let token: Option<String> = row.get("token");
            let user: Option<String> = row.get("user");
            sqlx::query(
                "UPDATE pairing_codes SET status = 'consumed', token = NULL WHERE device_code = ?",
            )
            .bind(device_code)
            .execute(db)
            .await?;
            match (token, user) {
                (Some(token), Some(user)) => Ok(PairPollResponse::Approved { token, user }),
                _ => Ok(PairPollResponse::Denied),
            }
        }
        "expired" => Ok(PairPollResponse::Expired),
        _ => Ok(PairPollResponse::Denied),
    }
}

pub async fn pairing_by_token(db: &SqlitePool, token: &str) -> Result<Option<PairingRow>> {
    Ok(sqlx::query("SELECT * FROM pairings WHERE token_hash = ?")
        .bind(token_hash(token))
        .fetch_optional(db)
        .await?
        .map(|r| pairing_row(&r)))
}

pub async fn list_pairings(db: &SqlitePool, user: Option<&str>) -> Result<Vec<PairingRow>> {
    let rows = match user {
        Some(u) => {
            sqlx::query("SELECT * FROM pairings WHERE user = ? ORDER BY created_at")
                .bind(u)
                .fetch_all(db)
                .await?
        }
        None => {
            sqlx::query("SELECT * FROM pairings ORDER BY created_at")
                .fetch_all(db)
                .await?
        }
    };
    Ok(rows.iter().map(pairing_row).collect())
}

/// Revoke every live pairing of `user` inside the caller's transaction, and return the
/// companions they were: the login now belongs to someone else (Decision 15), or its person left
/// their last tenant (ADR 0088). A companion socket fails its next envelope's check.
pub async fn revoke_all_of(conn: &mut sqlx::SqliteConnection, user: &str) -> Result<Vec<Uuid>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT companion_id FROM pairings WHERE lower(user) = lower(?) AND revoked_at IS NULL",
    )
    .bind(user)
    .fetch_all(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE pairings SET revoked_at = ? WHERE lower(user) = lower(?) AND revoked_at IS NULL",
    )
    .bind(ts(Utc::now()))
    .bind(user)
    .execute(&mut *conn)
    .await?;
    Ok(ids.iter().filter_map(|i| Uuid::parse_str(i).ok()).collect())
}

pub async fn revoke_pairing(db: &SqlitePool, companion_id: Uuid, user: &str) -> Result<bool> {
    let r = sqlx::query("UPDATE pairings SET revoked_at = ? WHERE companion_id = ? AND user = ? AND revoked_at IS NULL").bind(ts(Utc::now())).bind(companion_id.to_string()).bind(user).execute(db).await?;
    Ok(r.rows_affected() > 0)
}

pub async fn touch_pairing(db: &SqlitePool, companion_id: Uuid, at: DateTime<Utc>) -> Result<()> {
    sqlx::query("UPDATE pairings SET last_seen = ? WHERE companion_id = ?")
        .bind(ts(at))
        .bind(companion_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

pub async fn next_server_seq(db: &SqlitePool, companion_id: Uuid) -> Result<u64> {
    let r = sqlx::query("UPDATE pairings SET server_seq = server_seq + 1 WHERE companion_id = ? RETURNING server_seq").bind(companion_id.to_string()).fetch_one(db).await?;
    Ok(r.get::<i64, _>("server_seq") as u64)
}

/// What kind of client the companion said it is in its Hello (ADR 0062).
pub async fn set_client(
    db: &SqlitePool,
    companion_id: Uuid,
    client: rubato_protocol::ClientKind,
) -> Result<()> {
    sqlx::query("UPDATE pairings SET client = ? WHERE companion_id = ?")
        .bind(client.name())
        .bind(companion_id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

pub async fn set_last_companion_seq(db: &SqlitePool, companion_id: Uuid, seq: u64) -> Result<()> {
    sqlx::query("UPDATE pairings SET last_companion_seq = MAX(last_companion_seq, ?) WHERE companion_id = ?").bind(seq as i64).bind(companion_id.to_string()).execute(db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> (SqlitePool, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        (crate::kernel::db::connect(&url).await.unwrap(), tmp)
    }

    fn start() -> PairStartRequest {
        PairStartRequest {
            companion_id: Uuid::new_v4(),
            hostname: "h".into(),
            companion_version: "t".into(),
        }
    }

    async fn status(db: &SqlitePool, device_code: &str) -> Option<(String, Option<String>)> {
        sqlx::query("SELECT status, user FROM pairing_codes WHERE device_code = ?")
            .bind(device_code)
            .fetch_optional(db)
            .await
            .unwrap()
            .map(|r| (r.get("status"), r.get("user")))
    }

    /// #168: codes that expired more than the grace ago are deleted, whatever their status;
    /// a code that just expired and a pending one stay.
    #[tokio::test]
    async fn sweep_deletes_codes_long_expired() {
        let (db, _tmp) = pool().await;
        for (device, user_code, ttl) in [
            ("old", "OLD", -7200),
            ("recent", "RECENT", -60),
            ("live", "LIVE", 900),
        ] {
            assert!(
                create_pairing_code(&db, &start(), device, user_code, ttl, 50)
                    .await
                    .unwrap()
            );
        }
        sqlx::query("UPDATE pairing_codes SET status = 'consumed' WHERE device_code = 'old'")
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(sweep_codes(&db, Utc::now()).await.unwrap(), 1);
        assert_eq!(status(&db, "old").await, None);
        assert!(status(&db, "recent").await.is_some());
        assert!(status(&db, "live").await.is_some());
        println!("checked 3 codes: 1 swept, 2 kept");
    }

    /// #168: the claim step on a code another approval already claimed (what a racing
    /// approver meets after its read) changes nothing.
    #[tokio::test]
    async fn claim_refuses_a_code_already_claimed() {
        let (db, _tmp) = pool().await;
        assert!(
            create_pairing_code(&db, &start(), "dev", "CODE", 900, 50)
                .await
                .unwrap()
        );
        let mut tx = db.begin().await.unwrap();
        assert!(claim_code(&mut tx, "dev", "alice", "t1").await.unwrap());
        tx.commit().await.unwrap();
        let mut tx = db.begin().await.unwrap();
        assert!(!claim_code(&mut tx, "dev", "bob", "t2").await.unwrap());
        tx.commit().await.unwrap();
        assert_eq!(
            status(&db, "dev").await,
            Some(("approved".into(), Some("alice".into())))
        );
        let token: Option<String> =
            sqlx::query_scalar("SELECT token FROM pairing_codes WHERE device_code = 'dev'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(token.as_deref(), Some("t1"));
        println!("checked 2 claims of one code: first wins, second changes nothing");
    }

    /// #168: the cap counts only pending, unexpired codes, and is checked in the insert.
    #[tokio::test]
    async fn create_stops_at_the_pending_cap() {
        let (db, _tmp) = pool().await;
        let mut stored = Vec::new();
        for (device, code, ttl) in [
            ("expired", "E", -1),
            ("a", "A", 900),
            ("b", "B", 900),
            ("c", "C", 900),
        ] {
            stored.push(
                create_pairing_code(&db, &start(), device, code, ttl, 2)
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(stored, [true, true, true, false]);
        assert_eq!(status(&db, "c").await, None, "nothing stored over the cap");
        println!("checked 4 starts against a cap of 2: 3 stored, 1 refused");
    }
}
