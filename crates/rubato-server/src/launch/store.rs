//! Tables `launches` and `cap_hits`. `issues_with_live_cap_hit` reads both.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{CapHit, CapKind, IssueRef, LaunchCommand, LaunchResult, LaunchRole};
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
    // `role` and `lead_launch_id` as columns too (ADR 0072): the fleet reads them by index.
    sqlx::query("INSERT INTO launches (workspace_id, id, companion_id, user, repo, number, command, created_at, role, lead_launch_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(ws).bind(cmd.id.to_string()).bind(companion_id.to_string()).bind(&cmd.user).bind(&cmd.issue.repo).bind(cmd.issue.number as i64).bind(serde_json::to_string(cmd)?).bind(ts(Utc::now()))
        .bind((!cmd.role.is_worker()).then(|| cmd.role.name()))
        .bind(cmd.lead_launch_id.map(|l| l.to_string()))
        .execute(db).await?;
    Ok(())
}

/// What another slice needs to address a launch: its companion, its paired user, its issue and the
/// session id it was started with (plan 122, ADR 0051).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTarget {
    pub launch_id: Uuid,
    pub companion_id: Uuid,
    pub user: String,
    pub issue: IssueRef,
    pub session_id: Option<String>,
    /// The branch and plan path the launch was sent with.
    pub branch: Option<String>,
    pub plan_path: Option<String>,
    /// The workspace the launch was started from (ADR 0047): whose map its parts are checked
    /// against (ADR 0071).
    pub workspace: String,
    /// A person's lead agent, or a worker (ADR 0072).
    pub role: LaunchRole,
    /// The lead that asked for this worker.
    pub lead_launch_id: Option<Uuid>,
}

/// The launch with this id, if Rubato started it.
pub async fn launch_target(db: &SqlitePool, launch_id: Uuid) -> Result<Option<LaunchTarget>> {
    let row = sqlx::query(
        "SELECT companion_id, user, repo, number, command, workspace_id, role, lead_launch_id FROM launches WHERE id = ?",
    )
    .bind(launch_id.to_string())
    .fetch_optional(db)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let command: Option<LaunchCommand> = serde_json::from_str(&r.get::<String, _>("command")).ok();
    Ok(Some(LaunchTarget {
        launch_id,
        companion_id: r.get::<String, _>("companion_id").parse()?,
        user: r.get("user"),
        issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
        session_id: command.as_ref().map(|c| c.session_id.to_string()),
        branch: command.as_ref().map(|c| c.branch.clone()),
        plan_path: command.and_then(|c| c.plan_path),
        workspace: r.get("workspace_id"),
        role: role_of(r.get("role")),
        lead_launch_id: r
            .get::<Option<String>, _>("lead_launch_id")
            .and_then(|l| l.parse().ok()),
    }))
}

/// The result of a launch sent to `companion_id`; `None` when no such launch went to that
/// companion, and then nothing is written (#162).
pub async fn store_launch_result(
    db: &SqlitePool,
    companion_id: Uuid,
    result: &LaunchResult,
) -> Result<Option<IssueRef>> {
    let id = match result {
        LaunchResult::Started { command_id, .. }
        | LaunchResult::Refused { command_id, .. }
        | LaunchResult::Failed { command_id, .. } => command_id.to_string(),
    };
    let companion = companion_id.to_string();
    sqlx::query(
        "UPDATE launches SET result = ?, resulted_at = ? WHERE id = ? AND companion_id = ?",
    )
    .bind(serde_json::to_string(result)?)
    .bind(ts(Utc::now()))
    .bind(&id)
    .bind(&companion)
    .execute(db)
    .await?;
    let row = sqlx::query("SELECT repo, number FROM launches WHERE id = ? AND companion_id = ?")
        .bind(&id)
        .bind(&companion)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64)))
}

/// Whether the server ever sent a launch for `issue` to `companion_id`. The repo compares
/// without case: a spend hit names the repo as the session's remote spells it.
pub async fn companion_launched(
    db: &SqlitePool,
    companion_id: Uuid,
    issue: &IssueRef,
) -> Result<bool> {
    let r = sqlx::query("SELECT EXISTS (SELECT 1 FROM launches WHERE companion_id = ? AND repo = ? COLLATE NOCASE AND number = ?) AS yes")
        .bind(companion_id.to_string())
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .fetch_one(db)
        .await?;
    Ok(r.get::<i64, _>("yes") != 0)
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

/// Issues with a started launch from workspace `ws` (ADR 0045: launched there, so it belongs).
pub async fn launched_issues_in(db: &SqlitePool, ws: &str) -> Result<HashSet<IssueRef>> {
    let rows = sqlx::query(
        "SELECT repo, number FROM launches WHERE workspace_id = ? AND result LIKE '%\"outcome\":\"started\"%'",
    )
    .bind(ws)
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

fn role_of(v: Option<String>) -> LaunchRole {
    match v.as_deref() {
        Some("lead") => LaunchRole::Lead,
        _ => LaunchRole::Worker,
    }
}

/// One launch as the fleet reads it (ADR 0072).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetLaunch {
    pub launch_id: Uuid,
    pub companion_id: Uuid,
    pub issue: IssueRef,
    pub branch: Option<String>,
    pub role: LaunchRole,
    pub lead_launch_id: Option<Uuid>,
    /// `started`, `refused`, `failed`, or `None` while the companion has not answered.
    pub outcome: Option<String>,
    pub created_at: DateTime<Utc>,
}

fn launch_row(r: &sqlx::sqlite::SqliteRow) -> Result<FleetLaunch> {
    let command: Option<LaunchCommand> = serde_json::from_str(&r.get::<String, _>("command")).ok();
    let outcome = r
        .get::<Option<String>, _>("result")
        .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
        .and_then(|v| v.get("outcome").and_then(|o| o.as_str()).map(str::to_owned));
    Ok(FleetLaunch {
        launch_id: r.get::<String, _>("id").parse()?,
        companion_id: r.get::<String, _>("companion_id").parse()?,
        issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
        branch: command.map(|c| c.branch),
        role: role_of(r.get("role")),
        lead_launch_id: r
            .get::<Option<String>, _>("lead_launch_id")
            .and_then(|l| l.parse().ok()),
        outcome,
        created_at: parse_ts(&r.get::<String, _>("created_at")),
    })
}

const ROW_COLUMNS: &str =
    "id, companion_id, repo, number, command, role, lead_launch_id, result, created_at";

/// The workers lead `lead` asked for, newest first (index `launches_by_lead`).
pub async fn workers_of(db: &SqlitePool, lead: Uuid) -> Result<Vec<FleetLaunch>> {
    let rows = sqlx::query(&format!(
        "SELECT {ROW_COLUMNS} FROM launches WHERE lead_launch_id = ? ORDER BY created_at DESC"
    ))
    .bind(lead.to_string())
    .fetch_all(db)
    .await?;
    rows.iter().map(launch_row).collect()
}

/// `user`'s launches in workspace `ws` since `since`, newest first, at most `limit` (index
/// `launches_by_user`).
pub async fn launches_of(
    db: &SqlitePool,
    user: &str,
    ws: &str,
    since: DateTime<Utc>,
    limit: u32,
) -> Result<Vec<FleetLaunch>> {
    let rows = sqlx::query(&format!(
        "SELECT {ROW_COLUMNS} FROM launches WHERE user = ? AND workspace_id = ? AND created_at >= ? ORDER BY created_at DESC LIMIT ?"
    ))
    .bind(user)
    .bind(ws)
    .bind(ts(since))
    .bind(i64::from(limit))
    .fetch_all(db)
    .await?;
    rows.iter().map(launch_row).collect()
}

/// `user`'s launches with these ids (the live ones a list adds), in no order.
pub async fn launches_by_id(db: &SqlitePool, user: &str, ids: &[Uuid]) -> Result<Vec<FleetLaunch>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let marks = vec!["?"; ids.len()].join(", ");
    let sql = format!("SELECT {ROW_COLUMNS} FROM launches WHERE user = ? AND id IN ({marks})");
    let mut q = sqlx::query(&sql).bind(user);
    for id in ids {
        q = q.bind(id.to_string());
    }
    q.fetch_all(db).await?.iter().map(launch_row).collect()
}

/// Whether anyone but `user` launched `issue` since `since`: a lead may not take it (ADR 0072).
pub async fn launched_by_another(
    db: &SqlitePool,
    issue: &IssueRef,
    user: &str,
    since: DateTime<Utc>,
) -> Result<bool> {
    let r = sqlx::query("SELECT EXISTS (SELECT 1 FROM launches WHERE repo = ? COLLATE NOCASE AND number = ? AND user <> ? COLLATE NOCASE AND created_at >= ?) AS yes")
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .bind(user)
        .bind(ts(since))
        .fetch_one(db)
        .await?;
    Ok(r.get::<i64, _>("yes") != 0)
}
