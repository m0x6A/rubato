//! SQLite access. Rows hold signals (pins, snapshots, plans, PRs, comments, launches); the
//! progress state is never written here.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rubato_protocol::pairing::{PairPollResponse, PairStartRequest};
use rubato_protocol::{
    CapHit, DecisionRecord, IssuePin, IssueRef, LaunchCommand, LaunchResult, MapRef, Plan,
    PullRequestState, SessionState, StaleSession,
};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use uuid::Uuid;

pub async fn connect(url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)
        .context("database url")?
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .context("connecting to sqlite")?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    Ok(pool)
}

pub fn token_hash(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339()
}

fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

// ----- users and web sessions ----------------------------------------------------------------

pub async fn upsert_user(
    db: &SqlitePool,
    login: &str,
    github_id: Option<i64>,
    avatar_url: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO users (login, github_id, avatar_url, created_at) VALUES (?, ?, ?, ?) ON CONFLICT(login) DO UPDATE SET github_id = excluded.github_id, avatar_url = excluded.avatar_url")
        .bind(login).bind(github_id).bind(avatar_url).bind(ts(Utc::now())).execute(db).await?;
    Ok(())
}

pub async fn create_web_session(db: &SqlitePool, login: &str) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO web_sessions (id, login, created_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(login)
        .bind(ts(Utc::now()))
        .execute(db)
        .await?;
    Ok(id)
}

pub async fn web_session_login(db: &SqlitePool, id: &str) -> Result<Option<String>> {
    Ok(sqlx::query("SELECT login FROM web_sessions WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?
        .map(|r| r.get::<String, _>("login")))
}

pub async fn delete_web_session(db: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM web_sessions WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

// ----- pairing -------------------------------------------------------------------------------

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

// ----- events and snapshots ------------------------------------------------------------------

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

// ----- pins ----------------------------------------------------------------------------------

fn map_ref_parts(m: &MapRef) -> (&'static str, &str) {
    match m {
        MapRef::Node { id } => ("node", id),
        MapRef::Seam { id } => ("seam", id),
    }
}

pub async fn add_pin(db: &SqlitePool, pin: &IssuePin) -> Result<()> {
    let (kind, target) = map_ref_parts(&pin.target);
    sqlx::query("INSERT OR IGNORE INTO pins (repo, number, kind, target, pinned_by, pinned_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&pin.issue.repo).bind(pin.issue.number as i64).bind(kind).bind(target).bind(&pin.pinned_by).bind(ts(pin.pinned_at)).execute(db).await?;
    Ok(())
}

pub async fn remove_pin(db: &SqlitePool, issue: &IssueRef, target: &MapRef) -> Result<bool> {
    let (kind, t) = map_ref_parts(target);
    let r =
        sqlx::query("DELETE FROM pins WHERE repo = ? AND number = ? AND kind = ? AND target = ?")
            .bind(&issue.repo)
            .bind(issue.number as i64)
            .bind(kind)
            .bind(t)
            .execute(db)
            .await?;
    Ok(r.rows_affected() > 0)
}

pub async fn list_pins(db: &SqlitePool) -> Result<Vec<IssuePin>> {
    let rows = sqlx::query("SELECT * FROM pins ORDER BY pinned_at")
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let kind: String = r.get("kind");
            let target: String = r.get("target");
            IssuePin {
                issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
                target: if kind == "seam" {
                    MapRef::seam(target)
                } else {
                    MapRef::node(target)
                },
                pinned_by: r.get("pinned_by"),
                pinned_at: parse_ts(&r.get::<String, _>("pinned_at")),
            }
        })
        .collect())
}

// ----- launches, caps, stale -------------------------------------------------------------------

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

pub async fn latest_launch_at(db: &SqlitePool, issue: &IssueRef) -> Result<Option<DateTime<Utc>>> {
    let r = sqlx::query("SELECT MAX(created_at) AS t FROM launches WHERE repo = ? AND number = ?")
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .fetch_one(db)
        .await?;
    Ok(r.get::<Option<String>, _>("t").map(|s| parse_ts(&s)))
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

/// Issues with a cap hit newer than their latest launch (or any cap hit when never launched).
pub async fn issues_with_live_cap_hit(db: &SqlitePool) -> Result<HashSet<IssueRef>> {
    let rows = sqlx::query("SELECT c.repo, c.number, MAX(c.at) AS hit_at, (SELECT MAX(created_at) FROM launches l WHERE l.repo = c.repo AND l.number = c.number) AS launch_at FROM cap_hits c GROUP BY c.repo, c.number").fetch_all(db).await?;
    Ok(rows
        .iter()
        .filter(|r| {
            let hit: String = r.get("hit_at");
            match r.get::<Option<String>, _>("launch_at") {
                Some(l) => hit >= l,
                None => true,
            }
        })
        .map(|r| IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64))
        .collect())
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

// ----- plans, PRs, decisions, issue metadata -------------------------------------------------

#[derive(Debug, Clone)]
pub struct PlanRow {
    pub issue: IssueRef,
    pub branch: String,
    pub path: String,
    pub plan: Option<Plan>,
    pub parse_error: Option<String>,
}

pub async fn upsert_plan(
    db: &SqlitePool,
    issue: &IssueRef,
    branch: &str,
    path: &str,
    plan: Option<&Plan>,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO plans (repo, number, branch, path, plan, parse_error, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(repo, number) DO UPDATE SET branch = excluded.branch, path = excluded.path, plan = excluded.plan, parse_error = excluded.parse_error, fetched_at = excluded.fetched_at")
        .bind(&issue.repo).bind(issue.number as i64).bind(branch).bind(path).bind(plan.map(serde_json::to_string).transpose()?).bind(error).bind(ts(Utc::now()))
        .execute(db).await?;
    Ok(())
}

pub async fn delete_plan(db: &SqlitePool, issue: &IssueRef) -> Result<()> {
    sqlx::query("DELETE FROM plans WHERE repo = ? AND number = ?")
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn all_plans(db: &SqlitePool) -> Result<Vec<PlanRow>> {
    let rows = sqlx::query("SELECT * FROM plans").fetch_all(db).await?;
    Ok(rows
        .iter()
        .map(|r| PlanRow {
            issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
            branch: r.get("branch"),
            path: r.get("path"),
            plan: r
                .get::<Option<String>, _>("plan")
                .and_then(|s| serde_json::from_str(&s).ok()),
            parse_error: r.get("parse_error"),
        })
        .collect())
}

pub async fn upsert_pr(db: &SqlitePool, issue: &IssueRef, pr: &PullRequestState) -> Result<()> {
    sqlx::query("INSERT INTO pull_requests (repo, number, pr, fetched_at) VALUES (?, ?, ?, ?) ON CONFLICT(repo, number) DO UPDATE SET pr = excluded.pr, fetched_at = excluded.fetched_at")
        .bind(&issue.repo).bind(issue.number as i64).bind(serde_json::to_string(pr)?).bind(ts(Utc::now())).execute(db).await?;
    Ok(())
}

pub async fn all_prs(db: &SqlitePool) -> Result<HashMap<IssueRef, PullRequestState>> {
    let rows = sqlx::query("SELECT repo, number, pr FROM pull_requests")
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some((
                IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
                serde_json::from_str(&r.get::<String, _>("pr")).ok()?,
            ))
        })
        .collect())
}

pub async fn upsert_decision(db: &SqlitePool, rec: &DecisionRecord) -> Result<()> {
    sqlx::query("INSERT INTO decisions (repo, number, comment_id, record) VALUES (?, ?, ?, ?) ON CONFLICT(repo, number, comment_id) DO UPDATE SET record = excluded.record")
        .bind(&rec.issue.repo).bind(rec.issue.number as i64).bind(rec.comment_id as i64).bind(serde_json::to_string(rec)?).execute(db).await?;
    Ok(())
}

pub async fn all_decisions(db: &SqlitePool) -> Result<Vec<DecisionRecord>> {
    let rows = sqlx::query("SELECT record FROM decisions ORDER BY comment_id")
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| serde_json::from_str(&r.get::<String, _>("record")).ok())
        .collect())
}

#[derive(Debug, Clone, Default)]
pub struct IssueMeta {
    pub title: Option<String>,
    pub url: Option<String>,
    pub github_state: Option<String>,
}

pub async fn upsert_issue_meta(db: &SqlitePool, issue: &IssueRef, meta: &IssueMeta) -> Result<()> {
    sqlx::query("INSERT INTO issue_meta (repo, number, title, github_state, url, fetched_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(repo, number) DO UPDATE SET title = excluded.title, github_state = excluded.github_state, url = excluded.url, fetched_at = excluded.fetched_at")
        .bind(&issue.repo).bind(issue.number as i64).bind(&meta.title).bind(&meta.github_state).bind(&meta.url).bind(ts(Utc::now())).execute(db).await?;
    Ok(())
}

pub async fn all_issue_meta(db: &SqlitePool) -> Result<HashMap<IssueRef, IssueMeta>> {
    let rows = sqlx::query("SELECT repo, number, title, github_state, url FROM issue_meta")
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
                IssueMeta {
                    title: r.get("title"),
                    url: r.get("url"),
                    github_state: r.get("github_state"),
                },
            )
        })
        .collect())
}

// ----- map cache -----------------------------------------------------------------------------

pub async fn store_map(
    db: &SqlitePool,
    version: &str,
    yaml: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query("INSERT INTO map_cache (id, version, yaml, loaded_at) VALUES (1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET version = excluded.version, yaml = excluded.yaml, loaded_at = excluded.loaded_at")
        .bind(version).bind(yaml).bind(ts(at)).execute(db).await?;
    Ok(())
}

pub async fn load_map(db: &SqlitePool) -> Result<Option<(String, String, DateTime<Utc>)>> {
    Ok(
        sqlx::query("SELECT version, yaml, loaded_at FROM map_cache WHERE id = 1")
            .fetch_optional(db)
            .await?
            .map(|r| {
                (
                    r.get("version"),
                    r.get("yaml"),
                    parse_ts(&r.get::<String, _>("loaded_at")),
                )
            }),
    )
}

/// For the test that proves progress is never stored: every column of every table.
pub async fn all_columns(db: &SqlitePool) -> Result<Vec<(String, String)>> {
    let tables = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%'").fetch_all(db).await?;
    let mut out = Vec::new();
    for t in tables {
        let name: String = t.get("name");
        let cols = sqlx::query(&format!("PRAGMA table_info({name})"))
            .fetch_all(db)
            .await?;
        for c in cols {
            out.push((name.clone(), c.get::<String, _>("name")));
        }
    }
    Ok(out)
}
