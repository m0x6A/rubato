//! Table `fleet_requests`: one row per lead request, claimed before the request acts (plan 134
//! design §3, step 2), so a repeat never acts twice and the same id with another action is
//! refused.

use crate::kernel::db::{parse_ts, ts};
use crate::kernel::tenancy::TenantId;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use rubato_protocol::fleet::limits;
use rubato_protocol::{FleetRequest, FleetResult};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// What claiming a request found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The first time: act on it.
    New,
    /// Done before: answer this result again.
    Done(Box<FleetResult>),
    /// Claimed and not finished, since this time.
    Running(DateTime<Utc>),
    /// The same id with another action.
    Reused,
}

/// Claim `req` for `companion_id`, as `owner`. Deletes rows older than the retention first.
pub async fn claim(
    db: &SqlitePool,
    t: &TenantId,
    companion_id: Uuid,
    owner: &str,
    req: &FleetRequest,
    now: DateTime<Utc>,
) -> Result<Claim> {
    sqlx::query("DELETE FROM fleet_requests WHERE claimed_at < ?")
        .bind(ts(now - Duration::days(limits::RETENTION_DAYS)))
        .execute(db)
        .await?;
    let hash = req.action.hash();
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO fleet_requests (tenant_id, companion_id, lead_launch_id, request_id, action_hash, action, owner, target_launch_id, state, claimed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'in_progress', ?)",
    )
    .bind(t.as_str()).bind(companion_id.to_string())
    .bind(req.lead_launch_id.to_string())
    .bind(req.request_id.to_string())
    .bind(&hash)
    .bind(req.action.name())
    .bind(owner)
    .bind(req.action.target().map(|t| t.to_string()))
    .bind(ts(now))
    .execute(db)
    .await?
    .rows_affected()
        == 1;
    if inserted {
        return Ok(Claim::New);
    }
    let r = sqlx::query(
        "SELECT action_hash, state, result, claimed_at FROM fleet_requests WHERE companion_id = ? AND lead_launch_id = ? AND request_id = ?",
    )
    .bind(companion_id.to_string())
    .bind(req.lead_launch_id.to_string())
    .bind(req.request_id.to_string())
    .fetch_one(db)
    .await?;
    if r.get::<String, _>("action_hash") != hash {
        return Ok(Claim::Reused);
    }
    match (
        r.get::<String, _>("state").as_str(),
        r.get::<Option<String>, _>("result"),
    ) {
        ("done", Some(json)) => Ok(Claim::Done(Box::new(serde_json::from_str(&json)?))),
        _ => Ok(Claim::Running(parse_ts(&r.get::<String, _>("claimed_at")))),
    }
}

/// Record what a claimed request came to.
pub async fn finish(
    db: &SqlitePool,
    companion_id: Uuid,
    r: &FleetResult,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "UPDATE fleet_requests SET state = 'done', outcome = ?, reason = ?, result = ?, done_at = ? \
         WHERE companion_id = ? AND lead_launch_id = ? AND request_id = ?",
    )
    .bind(r.outcome.name())
    .bind(&r.reason)
    .bind(serde_json::to_string(r)?)
    .bind(ts(now))
    .bind(companion_id.to_string())
    .bind(r.lead_launch_id.to_string())
    .bind(r.request_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

/// The stored answer to a request already done, for a replayed envelope.
pub async fn stored(
    db: &SqlitePool,
    companion_id: Uuid,
    req: &FleetRequest,
) -> Result<Option<FleetResult>> {
    let r = sqlx::query(
        "SELECT result FROM fleet_requests WHERE companion_id = ? AND lead_launch_id = ? AND request_id = ? AND state = 'done'",
    )
    .bind(companion_id.to_string())
    .bind(req.lead_launch_id.to_string())
    .bind(req.request_id.to_string())
    .fetch_optional(db)
    .await?;
    r.and_then(|r| r.get::<Option<String>, _>("result"))
        .map(|j| serde_json::from_str(&j).map_err(Into::into))
        .transpose()
}

/// How many requests a table holds, for tests.
#[cfg(test)]
pub async fn count(db: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM fleet_requests")
        .fetch_one(db)
        .await
        .unwrap()
}
