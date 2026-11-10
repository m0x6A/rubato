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
    }
}

pub async fn create_pairing_code(
    db: &SqlitePool,
    req: &PairStartRequest,
    device_code: &str,
    user_code: &str,
    ttl_secs: i64,
) -> Result<()> {
    let now = Utc::now();
    sqlx::query("INSERT INTO pairing_codes (device_code, user_code, companion_id, hostname, companion_version, status, created_at, expires_at) VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)")
        .bind(device_code).bind(user_code).bind(req.companion_id.to_string()).bind(&req.hostname).bind(&req.companion_version).bind(ts(now)).bind(ts(now + chrono::Duration::seconds(ttl_secs)))
        .execute(db).await?;
    Ok(())
}

/// The human approved a code. Creates (or replaces) the pairing and parks the token for one poll.
pub async fn approve_pairing_code(
    db: &SqlitePool,
    user_code: &str,
    user: &str,
    token: &str,
) -> Result<Option<Uuid>> {
    let Some(row) = sqlx::query("SELECT device_code, companion_id, hostname, expires_at, status FROM pairing_codes WHERE user_code = ?").bind(user_code).fetch_optional(db).await? else {
        return Ok(None);
    };
    let status: String = row.get("status");
    let expires = parse_ts(&row.get::<String, _>("expires_at"));
    if status != "pending" || expires < Utc::now() {
        return Ok(None);
    }
    let companion_id: String = row.get("companion_id");
    let hostname: String = row.get("hostname");
    let device_code: String = row.get("device_code");
    let mut tx = db.begin().await?;
    sqlx::query(
        "UPDATE pairing_codes SET status = 'approved', user = ?, token = ? WHERE device_code = ?",
    )
    .bind(user)
    .bind(token)
    .bind(&device_code)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO pairings (companion_id, user, token_hash, hostname, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(companion_id) DO UPDATE SET user = excluded.user, token_hash = excluded.token_hash, hostname = excluded.hostname, revoked_at = NULL")
        .bind(&companion_id).bind(user).bind(token_hash(token)).bind(&hostname).bind(ts(Utc::now())).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Uuid::parse_str(&companion_id).ok())
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

pub async fn set_last_companion_seq(db: &SqlitePool, companion_id: Uuid, seq: u64) -> Result<()> {
    sqlx::query("UPDATE pairings SET last_companion_seq = MAX(last_companion_seq, ?) WHERE companion_id = ?").bind(seq as i64).bind(companion_id.to_string()).execute(db).await?;
    Ok(())
}
