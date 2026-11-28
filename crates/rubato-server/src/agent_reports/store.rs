//! Table `agent_reports`: one row per launch, changed only by the companion the launch went to,
//! in arrival order (the server's clock). Rows leave when their launch is out of its companion's
//! newest snapshot (`prune`) or out of every snapshot (`prune_absent`).

use crate::kernel::db::parse_ts;
use crate::kernel::state::App;
use crate::kernel::tenancy::TenantId;
use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use rubato_protocol::{AgentLine, SessionState};
use sqlx::{Row, SqlitePool};
use std::collections::HashSet;
use uuid::Uuid;

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Keep `doing` and `parts` as launch `launch_id`'s newest line, received at `now`. A row another
/// companion owns is left as it is. Says whether a row was written.
pub async fn record(
    db: &SqlitePool,
    t: &TenantId,
    companion_id: Uuid,
    launch_id: Uuid,
    doing: &str,
    parts: &[String],
    now: DateTime<Utc>,
) -> Result<bool> {
    let r = sqlx::query(
        "INSERT INTO agent_reports (tenant_id, launch_id, companion_id, doing, parts, reported_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(tenant_id, launch_id) DO UPDATE SET doing = excluded.doing, parts = excluded.parts, reported_at = excluded.reported_at WHERE agent_reports.companion_id = excluded.companion_id",
    )
    .bind(t.as_str()).bind(launch_id.to_string())
    .bind(companion_id.to_string())
    .bind(doing)
    .bind(serde_json::to_string(parts)?)
    .bind(stamp(now))
    .execute(db)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Delete `companion_id`'s rows whose launch is not in `live` (its newest snapshot). Returns how
/// many went.
pub async fn prune(db: &SqlitePool, companion_id: Uuid, live: &[Uuid]) -> Result<u64> {
    let keep: HashSet<String> = live.iter().map(Uuid::to_string).collect();
    let rows = sqlx::query("SELECT launch_id FROM agent_reports WHERE companion_id = ?")
        .bind(companion_id.to_string())
        .fetch_all(db)
        .await?;
    let mut gone = 0;
    for r in rows {
        let id: String = r.get("launch_id");
        if !keep.contains(&id) {
            gone +=
                sqlx::query("DELETE FROM agent_reports WHERE launch_id = ? AND companion_id = ?")
                    .bind(&id)
                    .bind(companion_id.to_string())
                    .execute(db)
                    .await?
                    .rows_affected();
        }
    }
    Ok(gone)
}

fn live_keys(sessions: &[SessionState]) -> HashSet<(String, String)> {
    sessions
        .iter()
        .filter_map(|s| Some((s.companion_id.to_string(), s.launch_id?.to_string())))
        .collect()
}

/// Delete every row whose `(companion_id, launch_id)` is in no companion's newest snapshot: a
/// revoked or retired companion sends no snapshot that would prune it. Returns how many went.
pub async fn prune_absent(db: &SqlitePool, t: &TenantId, sessions: &[SessionState]) -> Result<u64> {
    let live = live_keys(sessions);
    let rows = sqlx::query("SELECT launch_id, companion_id FROM agent_reports WHERE tenant_id = ?")
        .bind(t.as_str())
        .fetch_all(db)
        .await?;
    let mut gone = 0;
    for r in rows {
        let key: (String, String) = (r.get("companion_id"), r.get("launch_id"));
        if !live.contains(&key) {
            gone +=
                sqlx::query("DELETE FROM agent_reports WHERE launch_id = ? AND companion_id = ?")
                    .bind(&key.1)
                    .bind(&key.0)
                    .execute(db)
                    .await?
                    .rows_affected();
        }
    }
    Ok(gone)
}

/// The lines of launches live in some companion's newest snapshot, for the overview. Whatever
/// else the table still holds is not shown.
pub async fn all(app: &App, t: &TenantId) -> Result<Vec<AgentLine>> {
    lines(&app.db, t, &app.latest_sessions()).await
}

async fn lines(db: &SqlitePool, t: &TenantId, sessions: &[SessionState]) -> Result<Vec<AgentLine>> {
    let live = live_keys(sessions);
    let rows = sqlx::query(
        "SELECT launch_id, companion_id, doing, parts, reported_at FROM agent_reports WHERE tenant_id = ? ORDER BY launch_id",
    )
    .bind(t.as_str()).fetch_all(db)
    .await?;
    let mut out = Vec::new();
    for r in rows {
        let (launch, companion): (String, String) = (r.get("launch_id"), r.get("companion_id"));
        if !live.contains(&(companion.clone(), launch.clone())) {
            continue;
        }
        out.push(AgentLine {
            companion_id: companion.parse()?,
            launch_id: launch.parse()?,
            doing: r.get("doing"),
            parts: serde_json::from_str(&r.get::<String, _>("parts"))?,
            reported_at: parse_ts(&r.get::<String, _>("reported_at")),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rubato_protocol::{SessionStatus, StateSource};

    async fn db() -> (SqlitePool, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        (crate::kernel::db::connect(&url).await.unwrap(), tmp)
    }

    fn at(m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 16, 12, m, 0).unwrap()
    }

    fn session(companion: Uuid, launch: Uuid) -> SessionState {
        SessionState {
            session_id: Some(format!("s-{launch}")),
            companion_id: companion,
            user: "m0x6A".into(),
            tmux: None,
            cwd: "/w".into(),
            worktree: None,
            branch: None,
            repo: None,
            issue: None,
            status: SessionStatus::Idle,
            source: StateSource::Launch,
            last_seen: at(0),
            spend_usd: None,
            branch_drift: None,
            blocked_by: None,
            launch_id: Some(launch),
            hooks_lost: false,
        }
    }

    const A: Uuid = Uuid::from_u128(1);
    const B: Uuid = Uuid::from_u128(2);
    const L: Uuid = Uuid::from_u128(10);
    const M: Uuid = Uuid::from_u128(11);

    #[tokio::test]
    async fn the_newest_line_wins_and_another_companion_changes_nothing() {
        let (db, _t) = db().await;
        assert!(
            record(
                &db,
                &crate::kernel::tenancy::TenantId::default_tenant(),
                A,
                L,
                "one",
                &["rubato-server".into()],
                at(1)
            )
            .await
            .unwrap()
        );
        assert!(
            record(
                &db,
                &crate::kernel::tenancy::TenantId::default_tenant(),
                A,
                L,
                "two",
                &[],
                at(2)
            )
            .await
            .unwrap()
        );
        assert!(
            !record(
                &db,
                &crate::kernel::tenancy::TenantId::default_tenant(),
                B,
                L,
                "forged",
                &[],
                at(3)
            )
            .await
            .unwrap(),
            "not B's launch"
        );
        let got = lines(
            &db,
            &crate::kernel::tenancy::TenantId::default_tenant(),
            &[session(A, L)],
        )
        .await
        .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].doing.as_str(), got[0].companion_id), ("two", A));
        assert!(got[0].parts.is_empty());
        assert_eq!(got[0].reported_at, at(2));
    }

    #[tokio::test]
    async fn prune_deletes_a_companions_rows_not_in_its_snapshot_only() {
        let (db, _t) = db().await;
        record(
            &db,
            &crate::kernel::tenancy::TenantId::default_tenant(),
            A,
            L,
            "l",
            &[],
            at(1),
        )
        .await
        .unwrap();
        record(
            &db,
            &crate::kernel::tenancy::TenantId::default_tenant(),
            A,
            M,
            "m",
            &[],
            at(1),
        )
        .await
        .unwrap();
        record(
            &db,
            &crate::kernel::tenancy::TenantId::default_tenant(),
            B,
            Uuid::from_u128(12),
            "b",
            &[],
            at(1),
        )
        .await
        .unwrap();
        assert_eq!(prune(&db, A, &[L]).await.unwrap(), 1);
        let all = [
            session(A, L),
            session(A, M),
            session(B, Uuid::from_u128(12)),
        ];
        let left: Vec<_> = lines(
            &db,
            &crate::kernel::tenancy::TenantId::default_tenant(),
            &all,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|l| l.doing)
        .collect();
        assert_eq!(left, ["l", "b"]);
    }

    #[tokio::test]
    async fn prune_absent_deletes_rows_no_snapshot_names() {
        let (db, _t) = db().await;
        record(
            &db,
            &crate::kernel::tenancy::TenantId::default_tenant(),
            A,
            L,
            "l",
            &[],
            at(1),
        )
        .await
        .unwrap();
        record(
            &db,
            &crate::kernel::tenancy::TenantId::default_tenant(),
            B,
            M,
            "m",
            &[],
            at(1),
        )
        .await
        .unwrap();
        // B was revoked: it sends no snapshot, so only this sweep clears its row.
        assert_eq!(
            prune_absent(
                &db,
                &crate::kernel::tenancy::TenantId::default_tenant(),
                &[session(A, L)]
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            prune_absent(
                &db,
                &crate::kernel::tenancy::TenantId::default_tenant(),
                &[session(A, L)]
            )
            .await
            .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn only_live_launches_are_shown() {
        let (db, _t) = db().await;
        record(
            &db,
            &crate::kernel::tenancy::TenantId::default_tenant(),
            A,
            L,
            "l",
            &[],
            at(1),
        )
        .await
        .unwrap();
        assert!(
            lines(
                &db,
                &crate::kernel::tenancy::TenantId::default_tenant(),
                &[]
            )
            .await
            .unwrap()
            .is_empty()
        );
        // The same launch id claimed in another companion's snapshot is not this row.
        assert!(
            lines(
                &db,
                &crate::kernel::tenancy::TenantId::default_tenant(),
                &[session(B, L)]
            )
            .await
            .unwrap()
            .is_empty()
        );
        assert_eq!(
            lines(
                &db,
                &crate::kernel::tenancy::TenantId::default_tenant(),
                &[session(A, L)]
            )
            .await
            .unwrap()
            .len(),
            1
        );
    }
}
