//! Tables `companion_events`, `session_snapshots` and `stale_events`.
//!
//! - `companion_events` is read by its own insert: `(companion_id, seq)` is the replay dedupe
//!   key (ADR 0012), so `record_event` answering false is how a replayed envelope is recognised.
//!   Nothing else selects from it.
//! - `session_snapshots` is read by `all_sessions`.
//! - `stale_events` has no reader. It is kept as the audit record of a never-silent event
//!   (`rubato.session.stale`); the live signal is the log event and the alert, not this table
//!   (plan 29 S6, D2).

use crate::kernel::db::ts;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{SessionState, StaleSession};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// Returns false when this sequence was already recorded (a replay).
pub async fn record_event(
    db: &SqlitePool,
    companion_id: Uuid,
    seq: u64,
    kind: &str,
    payload: &serde_json::Value,
    at: DateTime<Utc>,
) -> Result<bool> {
    let r = sqlx::query("INSERT OR IGNORE INTO companion_events (companion_id, seq, kind, payload, received_at) VALUES (?, ?, ?, ?, ?)")
        .bind(companion_id.to_string()).bind(seq as i64).bind(kind).bind(payload.to_string()).bind(ts(at)).execute(db).await?;
    Ok(r.rows_affected() > 0)
}

pub async fn store_snapshot(
    db: &SqlitePool,
    companion_id: Uuid,
    sessions: &[SessionState],
    at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query("INSERT INTO session_snapshots (companion_id, sessions, updated_at) VALUES (?, ?, ?) ON CONFLICT(companion_id) DO UPDATE SET sessions = excluded.sessions, updated_at = excluded.updated_at")
        .bind(companion_id.to_string()).bind(serde_json::to_string(sessions)?).bind(ts(at)).execute(db).await?;
    Ok(())
}

pub async fn all_sessions(db: &SqlitePool) -> Result<Vec<SessionState>> {
    let rows = sqlx::query("SELECT sessions FROM session_snapshots")
        .fetch_all(db)
        .await?;
    let mut out = Vec::new();
    for r in rows {
        let v: Vec<SessionState> =
            serde_json::from_str(&r.get::<String, _>("sessions")).unwrap_or_default();
        out.extend(v);
    }
    Ok(out)
}

pub async fn store_stale(db: &SqlitePool, s: &StaleSession) -> Result<()> {
    sqlx::query("INSERT INTO stale_events (companion_id, payload, at) VALUES (?, ?, ?)")
        .bind(s.companion_id.to_string())
        .bind(serde_json::to_string(s)?)
        .bind(ts(s.at))
        .execute(db)
        .await?;
    Ok(())
}
