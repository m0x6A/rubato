//! Tables `companion_events`, `session_snapshots` and `stale_events`.
//!
//! - `companion_events` is read by its own insert: `(companion_id, seq)` is the replay dedupe
//!   key (ADR 0012), so `record_event` answering false is how a replayed envelope is recognised.
//!   Nothing else selects from it.
//! - `session_snapshots` is read by `all_sessions` and `companion_has_session_on`.
//! - `stale_events` has no reader. It is kept as the audit record of a never-silent event
//!   (`rubato.session.stale`); the live signal is the log event and the alert, not this table
//!   (plan 29 S6, D2).

use crate::kernel::db::ts;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{IssueRef, SessionState, StaleSession};
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

/// Whether this companion's stored snapshot has a session on `issue` (repo without case).
pub async fn companion_has_session_on(
    db: &SqlitePool,
    companion_id: Uuid,
    issue: &IssueRef,
) -> Result<bool> {
    let row = sqlx::query("SELECT sessions FROM session_snapshots WHERE companion_id = ?")
        .bind(companion_id.to_string())
        .fetch_optional(db)
        .await?;
    let Some(row) = row else { return Ok(false) };
    let sessions: Vec<SessionState> =
        serde_json::from_str(&row.get::<String, _>("sessions")).unwrap_or_default();
    Ok(sessions.iter().any(|s| {
        s.issue
            .as_ref()
            .is_some_and(|i| i.number == issue.number && i.repo.eq_ignore_ascii_case(&issue.repo))
    }))
}

/// Each companion's stored snapshot, by the pairing's companion id: what the kernel's session
/// cache starts from.
pub async fn session_snapshots(
    db: &SqlitePool,
) -> Result<std::collections::HashMap<Uuid, Vec<SessionState>>> {
    let rows = sqlx::query("SELECT companion_id, sessions FROM session_snapshots")
        .fetch_all(db)
        .await?;
    let mut out = std::collections::HashMap::new();
    for r in rows {
        let Ok(id) = r.get::<String, _>("companion_id").parse::<Uuid>() else {
            continue;
        };
        let v: Vec<SessionState> =
            serde_json::from_str(&r.get::<String, _>("sessions")).unwrap_or_default();
        out.insert(id, v);
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

#[cfg(test)]
mod tests {
    use super::*;
    use rubato_protocol::StateSource;

    /// Plan 147: a snapshot a tmux-ide companion stored (`source: tmux_ide`) still loads after
    /// tmux-ide is retired (ADR 0070, Q2). The read path turns a row that does not parse into
    /// an empty list, so this asserts the session itself comes back, not just that nothing fails.
    #[tokio::test]
    async fn a_snapshot_stored_by_a_tmux_ide_companion_still_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let db = crate::kernel::db::connect(&url).await.unwrap();
        let companion = "6f1c3a2e-9b1d-4c8e-8f7a-2d3b4c5d6e7f";
        let stored = serde_json::json!([{
            "session_id": "0f8fad5b-d9cb-469f-a165-70867728950e",
            "companion_id": companion,
            "user": "m0x6A",
            "tmux": { "session": "rubato@issue-7-restock", "window": 1, "pane": "%5" },
            "cwd": "/w/pantry-worktrees/issue/7-restock",
            "branch": "issue/7-restock",
            "repo": "acme/pantry",
            "issue": { "repo": "acme/pantry", "number": 7 },
            "status": "working",
            "source": "tmux_ide",
            "last_seen": "2026-11-14T12:00:00Z"
        }]);
        sqlx::query(
            "INSERT INTO session_snapshots (companion_id, sessions, updated_at) VALUES (?, ?, ?)",
        )
        .bind(companion)
        .bind(stored.to_string())
        .bind(ts(Utc::now()))
        .execute(&db)
        .await
        .unwrap();
        let sessions = all_sessions(&db).await.unwrap();
        assert_eq!(sessions.len(), 1, "the stored session comes back");
        assert_eq!(sessions[0].source, StateSource::TmuxIde);
        assert_eq!(sessions[0].issue, Some(IssueRef::new("acme/pantry", 7)));
    }
}
