//! Tables `map_draft_ops` and `map_proposal`: one draft and at most one proposal per workspace
//! (ADR 0047). `seq` is global; every query binds the workspace.

use super::apply;
use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::SystemMap;
use rubato_protocol::map_draft::{
    DraftOp, DraftOpRequest, MapDraft, MapOp, Proposal, ProposalState,
};
use sqlx::{Row, SqliteConnection, SqlitePool};
use uuid::Uuid;

/// The proposal row: what the wire carries plus what the server needs to resume and to clear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalRow {
    pub branch: String,
    pub base_sha: String,
    pub file_hash: String,
    pub last_seq: u64,
    pub state: ProposalState,
    pub state_since: DateTime<Utc>,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub proposed_by: String,
}

impl ProposalRow {
    pub fn to_wire(&self) -> Proposal {
        Proposal {
            state: self.state,
            state_since: self.state_since,
            branch: self.branch.clone(),
            proposed_by: self.proposed_by.clone(),
            pr_number: self.pr_number,
            pr_url: self.pr_url.clone(),
        }
    }
}

pub fn state_name(s: ProposalState) -> &'static str {
    match s {
        ProposalState::Opening => "opening",
        ProposalState::Proposed => "proposed",
        ProposalState::Merged => "merged",
    }
}

fn parse_state(s: &str) -> ProposalState {
    match s {
        "proposed" => ProposalState::Proposed,
        "merged" => ProposalState::Merged,
        _ => ProposalState::Opening,
    }
}

async fn live(conn: &mut SqliteConnection, ws: &str) -> Result<Vec<DraftOp>> {
    let rows = sqlx::query("SELECT seq, op_id, op, author, created_at FROM map_draft_ops WHERE workspace_id = ? AND discarded_at IS NULL ORDER BY seq")
        .bind(ws)
        .fetch_all(conn)
        .await?;
    rows.iter()
        .map(|r| {
            Ok(DraftOp {
                seq: r.get::<i64, _>("seq") as u64,
                op_id: Uuid::parse_str(&r.get::<String, _>("op_id")).unwrap_or_default(),
                op: serde_json::from_str::<MapOp>(&r.get::<String, _>("op"))?,
                author: r.get("author"),
                created_at: parse_ts(&r.get::<String, _>("created_at")),
                stale: None,
            })
        })
        .collect()
}

async fn proposal_of(conn: &mut SqliteConnection, ws: &str) -> Result<Option<ProposalRow>> {
    let row = sqlx::query("SELECT * FROM map_proposal WHERE workspace_id = ?")
        .bind(ws)
        .fetch_optional(conn)
        .await?;
    Ok(row.map(|r| ProposalRow {
        branch: r.get("branch"),
        base_sha: r.get("base_sha"),
        file_hash: r.get("file_hash"),
        last_seq: r.get::<i64, _>("last_seq") as u64,
        state: parse_state(&r.get::<String, _>("state")),
        state_since: parse_ts(&r.get::<String, _>("state_since")),
        pr_number: r.get::<Option<i64>, _>("pr_number").map(|n| n as u64),
        pr_url: r.get("pr_url"),
        proposed_by: r.get("proposed_by"),
    }))
}

pub async fn proposal(db: &SqlitePool, ws: &str) -> Result<Option<ProposalRow>> {
    proposal_of(&mut *db.acquire().await?, ws).await
}

/// The live operations, each checked against `base` with the ones before it.
pub async fn ops(db: &SqlitePool, ws: &str, base: &SystemMap) -> Result<(Vec<DraftOp>, SystemMap)> {
    let mut ops = live(&mut *db.acquire().await?, ws).await?;
    let preview = apply::replay(base, &mut ops);
    Ok((ops, preview))
}

/// The draft as every browser sees it.
pub async fn snapshot(db: &SqlitePool, ws: &str, base: &SystemMap) -> Result<MapDraft> {
    let mut conn = db.acquire().await?;
    let mut ops = live(&mut conn, ws).await?;
    let preview = apply::replay(base, &mut ops);
    let discarded: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM map_draft_ops WHERE workspace_id = ? AND discarded_at IS NOT NULL",
    )
    .bind(ws)
    .fetch_one(&mut *conn)
    .await?;
    Ok(MapDraft {
        preview: (!ops.is_empty()).then_some(preview),
        // A discarded draft can come back only while nothing new was written.
        can_restore: ops.is_empty() && discarded > 0,
        ops,
        proposal: proposal_of(&mut conn, ws).await?.map(|p| p.to_wire()),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Appended {
    Yes(u64),
    /// The same `op_id` was stored before: a retried request.
    Duplicate,
    Frozen,
    Full,
    Invalid(String),
}

/// Add an operation at the end of the draft. Reading the earlier operations, checking the new
/// one against them and inserting it happen in one `BEGIN IMMEDIATE` transaction, so two
/// operations that conflict cannot both land.
pub async fn append(
    db: &SqlitePool,
    ws: &str,
    base: &SystemMap,
    req: &DraftOpRequest,
    author: &str,
    at: DateTime<Utc>,
    max: usize,
) -> Result<Appended> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if proposal_of(&mut tx, ws).await?.is_some() {
        return Ok(Appended::Frozen);
    }
    let known: Option<i64> = sqlx::query_scalar("SELECT seq FROM map_draft_ops WHERE op_id = ?")
        .bind(req.op_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
    if known.is_some() {
        return Ok(Appended::Duplicate);
    }
    let mut earlier = live(&mut tx, ws).await?;
    if earlier.len() >= max {
        return Ok(Appended::Full);
    }
    let so_far = apply::replay(base, &mut earlier);
    if let Err(why) = apply::apply(&so_far, &req.op) {
        return Ok(Appended::Invalid(why));
    }
    // A discarded draft can be restored only until the next operation is written.
    sqlx::query("DELETE FROM map_draft_ops WHERE workspace_id = ? AND discarded_at IS NOT NULL")
        .bind(ws)
        .execute(&mut *tx)
        .await?;
    let seq = sqlx::query(
        "INSERT INTO map_draft_ops (workspace_id, op_id, op, author, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(ws)
    .bind(req.op_id.to_string())
    .bind(serde_json::to_string(&req.op)?)
    .bind(author)
    .bind(ts(at))
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    tx.commit().await?;
    Ok(Appended::Yes(seq as u64))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Changed {
    Yes(usize),
    /// Nothing there to act on.
    Nothing,
    Frozen,
}

/// Take one operation out of the draft, anyone's.
pub async fn remove_op(db: &SqlitePool, ws: &str, seq: u64) -> Result<Changed> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if proposal_of(&mut tx, ws).await?.is_some() {
        return Ok(Changed::Frozen);
    }
    let r = sqlx::query(
        "DELETE FROM map_draft_ops WHERE workspace_id = ? AND seq = ? AND discarded_at IS NULL",
    )
    .bind(ws)
    .bind(seq as i64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(if r.rows_affected() > 0 {
        Changed::Yes(1)
    } else {
        Changed::Nothing
    })
}

/// Empty the draft for everyone, keeping the operations so it can be undone.
pub async fn discard(db: &SqlitePool, ws: &str, at: DateTime<Utc>) -> Result<Changed> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if proposal_of(&mut tx, ws).await?.is_some() {
        return Ok(Changed::Frozen);
    }
    // An older discarded draft gives way to this one.
    sqlx::query("DELETE FROM map_draft_ops WHERE workspace_id = ? AND discarded_at IS NOT NULL")
        .bind(ws)
        .execute(&mut *tx)
        .await?;
    let r = sqlx::query("UPDATE map_draft_ops SET discarded_at = ? WHERE workspace_id = ?")
        .bind(ts(at))
        .bind(ws)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(match r.rows_affected() {
        0 => Changed::Nothing,
        n => Changed::Yes(n as usize),
    })
}

/// Undo a discard: every operation comes back, with its author and its place.
pub async fn restore(db: &SqlitePool, ws: &str) -> Result<Changed> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM map_draft_ops WHERE workspace_id = ? AND discarded_at IS NULL",
    )
    .bind(ws)
    .fetch_one(&mut *tx)
    .await?;
    if live > 0 {
        return Ok(Changed::Nothing);
    }
    let r =
        sqlx::query("UPDATE map_draft_ops SET discarded_at = NULL WHERE workspace_id = ? AND discarded_at IS NOT NULL")
            .bind(ws)
            .execute(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(match r.rows_affected() {
        0 => Changed::Nothing,
        n => Changed::Yes(n as usize),
    })
}

/// Start a proposal of the live operations: the one row there can be, written in the same
/// transaction that reads the operations, so what is proposed and what is frozen are one set.
/// `Err(existing)` when a proposal is already there.
pub async fn start_proposal(
    db: &SqlitePool,
    ws: &str,
    branch: &str,
    base_sha: &str,
    file_hash: &str,
    by: &str,
    at: DateTime<Utc>,
) -> Result<std::result::Result<ProposalRow, ProposalRow>> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(existing) = proposal_of(&mut tx, ws).await? {
        return Ok(Err(existing));
    }
    let last_seq = live(&mut tx, ws).await?.last().map(|o| o.seq).unwrap_or(0);
    sqlx::query("INSERT INTO map_proposal (workspace_id, branch, base_sha, file_hash, last_seq, state, state_since, proposed_by) VALUES (?, ?, ?, ?, ?, 'opening', ?, ?)")
        .bind(ws)
        .bind(branch)
        .bind(base_sha)
        .bind(file_hash)
        .bind(last_seq as i64)
        .bind(ts(at))
        .bind(by)
        .execute(&mut *tx)
        .await?;
    let row = proposal_of(&mut tx, ws).await?.expect("just inserted");
    tx.commit().await?;
    Ok(Ok(row))
}

/// Move the proposal to `state`, with the pull request when it is known.
pub async fn set_state(
    db: &SqlitePool,
    ws: &str,
    state: ProposalState,
    at: DateTime<Utc>,
    pr: Option<(u64, &str)>,
) -> Result<()> {
    sqlx::query("UPDATE map_proposal SET state = ?, state_since = ?, pr_number = COALESCE(?, pr_number), pr_url = COALESCE(?, pr_url) WHERE workspace_id = ?")
        .bind(state_name(state))
        .bind(ts(at))
        .bind(pr.map(|(n, _)| n as i64))
        .bind(pr.map(|(_, u)| u))
        .bind(ws)
        .execute(db)
        .await?;
    Ok(())
}

/// The proposal is over without a merge: the draft is editable again, every operation kept.
pub async fn drop_proposal(db: &SqlitePool, ws: &str) -> Result<()> {
    sqlx::query("DELETE FROM map_proposal WHERE workspace_id = ?")
        .bind(ws)
        .execute(db)
        .await?;
    Ok(())
}

/// The merged proposal is in the loaded map: its operations and the proposal go.
pub async fn clear_merged(db: &SqlitePool, ws: &str, last_seq: u64) -> Result<()> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    // Only this workspace's operations: `seq` is shared, so another draft's older ones stay.
    sqlx::query("DELETE FROM map_draft_ops WHERE workspace_id = ? AND seq <= ?")
        .bind(ws)
        .bind(last_seq as i64)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM map_proposal WHERE workspace_id = ?")
        .bind(ws)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// How many live operations the draft holds.
pub async fn live_count(db: &SqlitePool, ws: &str) -> Result<u64> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM map_draft_ops WHERE workspace_id = ? AND discarded_at IS NULL",
    )
    .bind(ws)
    .fetch_one(db)
    .await?;
    Ok(n as u64)
}
