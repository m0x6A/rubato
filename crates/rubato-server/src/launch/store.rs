//! Tables `launches` and `cap_hits`. `issues_with_live_cap_hit` reads both.

use crate::kernel::db::ts;
use anyhow::Result;
use chrono::Utc;
use rubato_protocol::{CapHit, CapKind, IssueRef, LaunchCommand, LaunchResult};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub async fn store_launch(db: &SqlitePool, companion_id: Uuid, cmd: &LaunchCommand) -> Result<()> {
    sqlx::query("INSERT INTO launches (id, companion_id, user, repo, number, command, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(cmd.id.to_string()).bind(companion_id.to_string()).bind(&cmd.user).bind(&cmd.issue.repo).bind(cmd.issue.number as i64).bind(serde_json::to_string(cmd)?).bind(ts(Utc::now()))
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

pub async fn launch_map_versions(db: &SqlitePool) -> Result<Vec<(IssueRef, String)>> {
    let rows = sqlx::query(
        "SELECT repo, number, command FROM launches WHERE result LIKE '%\"outcome\":\"started\"%'",
    )
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

pub async fn store_cap_hit(db: &SqlitePool, hit: &CapHit) -> Result<()> {
    sqlx::query("INSERT INTO cap_hits (repo, number, payload, at) VALUES (?, ?, ?, ?)")
        .bind(&hit.issue.repo)
        .bind(hit.issue.number as i64)
        .bind(serde_json::to_string(hit)?)
        .bind(ts(hit.at))
        .execute(db)
        .await?;
    Ok(())
}

/// Issues with a cap hit newer than their latest launch (or any cap hit when never launched),
/// each with the kind of that hit. When hits of both kinds are live the issue reads as `Spend`,
/// the kind that blocks whatever the sessions say; so does a payload that does not parse.
pub async fn issues_with_live_cap_hit(db: &SqlitePool) -> Result<HashMap<IssueRef, CapKind>> {
    let rows = sqlx::query("SELECT c.repo, c.number, c.payload FROM cap_hits c WHERE c.at >= COALESCE((SELECT MAX(created_at) FROM launches l WHERE l.repo = c.repo AND l.number = c.number), '')").fetch_all(db).await?;
    let mut live = HashMap::new();
    for r in &rows {
        let issue = IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64);
        let kind = serde_json::from_str::<CapHit>(&r.get::<String, _>("payload"))
            .map_or(CapKind::Spend, |hit| hit.kind);
        let entry = live.entry(issue).or_insert(kind);
        if kind == CapKind::Spend {
            *entry = CapKind::Spend;
        }
    }
    Ok(live)
}
