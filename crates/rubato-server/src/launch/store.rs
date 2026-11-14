//! Tables `launches` and `cap_hits`. `issues_with_live_cap_hit` reads both.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{CapHit, CapKind, IssueRef, LaunchCommand, LaunchResult};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// `ws` is the workspace the launch was started from (ADR 0047).
pub async fn store_launch(
    db: &SqlitePool,
    ws: &str,
    companion_id: Uuid,
    cmd: &LaunchCommand,
) -> Result<()> {
    sqlx::query("INSERT INTO launches (workspace_id, id, companion_id, user, repo, number, command, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(ws).bind(cmd.id.to_string()).bind(companion_id.to_string()).bind(&cmd.user).bind(&cmd.issue.repo).bind(cmd.issue.number as i64).bind(serde_json::to_string(cmd)?).bind(ts(Utc::now()))
        .execute(db).await?;
    Ok(())
}

pub async fn store_launch_result(
    db: &SqlitePool,
    result: &LaunchResult,
) -> Result<Option<IssueRef>> {
    let id = match result {
        LaunchResult::Started { command_id, .. }
        | LaunchResult::Refused { command_id, .. }
        | LaunchResult::Failed { command_id, .. } => command_id.to_string(),
    };
    sqlx::query("UPDATE launches SET result = ?, resulted_at = ? WHERE id = ?")
        .bind(serde_json::to_string(result)?)
        .bind(ts(Utc::now()))
        .bind(&id)
        .execute(db)
        .await?;
    let row = sqlx::query("SELECT repo, number FROM launches WHERE id = ?")
        .bind(&id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64)))
}

/// Issues with at least one launch that started.
pub async fn launched_issues(db: &SqlitePool) -> Result<HashSet<IssueRef>> {
    let rows = sqlx::query(
        "SELECT repo, number FROM launches WHERE result LIKE '%\"outcome\":\"started\"%'",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .map(|r| IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64))
        .collect())
}

pub async fn launch_count(db: &SqlitePool, issue: &IssueRef) -> Result<u32> {
    let r = sqlx::query("SELECT COUNT(*) AS n FROM launches WHERE repo = ? AND number = ? AND (result IS NULL OR result LIKE '%\"outcome\":\"started\"%')").bind(&issue.repo).bind(issue.number as i64).fetch_one(db).await?;
    Ok(r.get::<i64, _>("n") as u32)
}

/// The map version each started launch of this workspace carried, compared with that
/// workspace's map only (`MapVersionMismatch`).
pub async fn launch_map_versions(db: &SqlitePool, ws: &str) -> Result<Vec<(IssueRef, String)>> {
    let rows = sqlx::query(
        "SELECT repo, number, command FROM launches WHERE workspace_id = ? AND result LIKE '%\"outcome\":\"started\"%'",
    )
    .bind(ws)
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let cmd: LaunchCommand = serde_json::from_str(&r.get::<String, _>("command")).ok()?;
            Some((
                IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
                cmd.map_version?,
            ))
        })
        .collect())
}

/// `at` is when the server received the hit, on its own clock, so it compares with `launches.created_at`
/// whatever the companion clock says; the companion's own time stays in the payload.
pub async fn store_cap_hit(db: &SqlitePool, hit: &CapHit, at: DateTime<Utc>) -> Result<()> {
    sqlx::query("INSERT INTO cap_hits (repo, number, payload, at) VALUES (?, ?, ?, ?)")
        .bind(&hit.issue.repo)
        .bind(hit.issue.number as i64)
        .bind(serde_json::to_string(hit)?)
        .bind(ts(at))
        .execute(db)
        .await?;
    Ok(())
}

/// One stored launch, as far as cap-hit liveness needs it.
struct LaunchRow {
    created_at: DateTime<Utc>,
    session_id: Option<String>,
    started: bool,
}

/// Issues with a live cap hit, each with the kind of that hit. A hit is live when the issue was
/// never launched, or when no newer run superseded it:
///
/// - a hit naming a session whose launch the server stored is live unless a launch *started*
///   after that one (a second Start refused as `already_running` does not supersede the run);
/// - any other hit (attempts, or a session the server never launched) is live when the server
///   received it at or after the issue's latest launch was stored.
///
/// Both rules use the server's own records, never the companion's clock (#44). When hits of
/// both kinds are live the issue reads as `Spend`, the kind that blocks whatever the sessions
/// say; so does a payload that does not parse.
pub async fn issues_with_live_cap_hit(db: &SqlitePool) -> Result<HashMap<IssueRef, CapKind>> {
    let rows = sqlx::query("SELECT repo, number, created_at, command, (result LIKE '%\"outcome\":\"started\"%') AS started FROM launches").fetch_all(db).await?;
    let mut launches: HashMap<IssueRef, Vec<LaunchRow>> = HashMap::new();
    for r in &rows {
        let issue = IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64);
        launches.entry(issue).or_default().push(LaunchRow {
            created_at: parse_ts(&r.get::<String, _>("created_at")),
            session_id: serde_json::from_str::<LaunchCommand>(&r.get::<String, _>("command"))
                .ok()
                .map(|c| c.session_id.to_string()),
            started: r.get::<Option<bool>, _>("started").unwrap_or(false),
        });
    }
    let rows = sqlx::query("SELECT repo, number, payload, at FROM cap_hits")
        .fetch_all(db)
        .await?;
    let mut live = HashMap::new();
    for r in &rows {
        let issue = IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64);
        let hit = serde_json::from_str::<CapHit>(&r.get::<String, _>("payload")).ok();
        let received = parse_ts(&r.get::<String, _>("at"));
        if !hit_is_live(launches.get(&issue), hit.as_ref(), received) {
            continue;
        }
        let kind = hit.map_or(CapKind::Spend, |hit| hit.kind);
        let entry = live.entry(issue).or_insert(kind);
        if kind == CapKind::Spend {
            *entry = CapKind::Spend;
        }
    }
    Ok(live)
}

fn hit_is_live(
    launches: Option<&Vec<LaunchRow>>,
    hit: Option<&CapHit>,
    received: DateTime<Utc>,
) -> bool {
    let Some(launches) = launches.filter(|l| !l.is_empty()) else {
        return true;
    };
    let own = hit
        .and_then(|h| h.session_id.as_deref())
        .and_then(|s| launches.iter().find(|l| l.session_id.as_deref() == Some(s)));
    match own {
        Some(own) => !launches
            .iter()
            .any(|l| l.started && l.created_at > own.created_at),
        None => launches.iter().all(|l| received >= l.created_at),
    }
}
