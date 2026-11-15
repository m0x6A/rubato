//! Tables `room_agents` and `room_requests` (ADR 0057): who is in a room, called by whom, since
//! when; and the requests to call someone else's agent in.

use crate::kernel::db::{parse_ts, ts};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use rubato_protocol::{LeftReason, RequestState, RoomAgent, RoomKey, RoomRequest, agent_handle};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// One stay of an agent in a room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    pub id: Uuid,
    pub room: RoomKey,
    pub agent: RoomAgent,
    pub fork_session_id: Option<String>,
}

impl AgentRow {
    pub fn present(&self) -> bool {
        self.agent.left_at.is_none()
    }

    /// When the agent was last addressed, or joined: a turn carries the room since then.
    pub fn since(&self) -> DateTime<Utc> {
        self.agent.last_turn_at.unwrap_or(self.agent.joined_at)
    }
}

fn reason_name(r: LeftReason) -> &'static str {
    match r {
        LeftReason::Dismissed => "dismissed",
        LeftReason::TimedOut => "timed_out",
        LeftReason::SessionEnded => "session_ended",
    }
}

fn reason_of(s: &str) -> Option<LeftReason> {
    match s {
        "dismissed" => Some(LeftReason::Dismissed),
        "timed_out" => Some(LeftReason::TimedOut),
        "session_ended" => Some(LeftReason::SessionEnded),
        _ => None,
    }
}

fn state_name(s: RequestState) -> &'static str {
    match s {
        RequestState::Pending => "pending",
        RequestState::Accepted => "accepted",
        RequestState::Declined => "declined",
        RequestState::Withdrawn => "withdrawn",
    }
}

fn state_of(s: &str) -> Result<RequestState> {
    Ok(match s {
        "pending" => RequestState::Pending,
        "accepted" => RequestState::Accepted,
        "declined" => RequestState::Declined,
        "withdrawn" => RequestState::Withdrawn,
        other => return Err(anyhow!("unknown request state {other:?}")),
    })
}

fn opt_ts(r: &SqliteRow, c: &str) -> Option<DateTime<Utc>> {
    r.get::<Option<String>, _>(c).map(|t| parse_ts(&t))
}

fn key(r: &SqliteRow) -> RoomKey {
    RoomKey {
        repo: r.get("repo"),
        number: r.get::<i64, _>("number") as u64,
        branch: r.get("branch"),
    }
}

fn agent(r: &SqliteRow) -> Result<AgentRow> {
    let owner: String = r.get("owner");
    Ok(AgentRow {
        id: r.get::<String, _>("id").parse()?,
        room: key(r),
        agent: RoomAgent {
            handle: agent_handle(&owner),
            launch_id: r.get::<String, _>("launch_id").parse()?,
            called_by: r.get("called_by"),
            joined_at: parse_ts(&r.get::<String, _>("joined_at")),
            left_at: opt_ts(r, "left_at"),
            left_reason: r
                .get::<Option<String>, _>("left_reason")
                .as_deref()
                .and_then(reason_of),
            last_turn_at: opt_ts(r, "last_turn_at"),
            owner,
        },
        fork_session_id: r.get("fork_session_id"),
    })
}

const AGENT_COLUMNS: &str = "id, repo, number, branch, launch_id, owner, called_by, joined_at, left_at, left_reason, last_turn_at, fork_session_id";

/// The agent joins: its row, or None when the owner's agent is in this room already.
pub async fn join(db: &SqlitePool, room: &RoomKey, a: &RoomAgent) -> Result<Option<AgentRow>> {
    let id = Uuid::new_v4();
    let n = sqlx::query(
        "INSERT OR IGNORE INTO room_agents (id, repo, number, branch, launch_id, owner, called_by, joined_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&room.repo)
    .bind(room.number as i64)
    .bind(&room.branch)
    .bind(a.launch_id.to_string())
    .bind(&a.owner)
    .bind(&a.called_by)
    .bind(ts(a.joined_at))
    .execute(db)
    .await?
    .rows_affected();
    if n == 0 {
        return Ok(None);
    }
    Ok(agents_in(db, room).await?.into_iter().find(|r| r.id == id))
}

/// Every stay in the room, present ones first, then the latest to leave.
pub async fn agents_in(db: &SqlitePool, room: &RoomKey) -> Result<Vec<AgentRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {AGENT_COLUMNS} FROM room_agents WHERE repo = ? AND number = ? AND branch = ? \
         ORDER BY left_at IS NOT NULL, left_at DESC, joined_at"
    ))
    .bind(&room.repo)
    .bind(room.number as i64)
    .bind(&room.branch)
    .fetch_all(db)
    .await?;
    rows.iter().map(agent).collect()
}

/// Every agent in some room now.
pub async fn present_everywhere(db: &SqlitePool) -> Result<Vec<AgentRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {AGENT_COLUMNS} FROM room_agents WHERE left_at IS NULL"
    ))
    .fetch_all(db)
    .await?;
    rows.iter().map(agent).collect()
}

/// The agent leaves. False when it had left already.
pub async fn leave(
    db: &SqlitePool,
    id: Uuid,
    reason: LeftReason,
    at: DateTime<Utc>,
) -> Result<bool> {
    let n = sqlx::query(
        "UPDATE room_agents SET left_at = ?, left_reason = ? WHERE id = ? AND left_at IS NULL",
    )
    .bind(ts(at))
    .bind(reason_name(reason))
    .bind(id.to_string())
    .execute(db)
    .await?
    .rows_affected();
    Ok(n == 1)
}

pub async fn addressed(db: &SqlitePool, id: Uuid, at: DateTime<Utc>) -> Result<()> {
    sqlx::query("UPDATE room_agents SET last_turn_at = ? WHERE id = ?")
        .bind(ts(at))
        .bind(id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

/// The fork a building-time turn ran in, for the launch's latest stay in the room.
pub async fn set_fork_session(
    db: &SqlitePool,
    room: &RoomKey,
    launch_id: Uuid,
    fork_session_id: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE room_agents SET fork_session_id = ? WHERE id = (SELECT id FROM room_agents \
         WHERE repo = ? AND number = ? AND branch = ? AND launch_id = ? ORDER BY joined_at DESC LIMIT 1)",
    )
    .bind(fork_session_id)
    .bind(&room.repo)
    .bind(room.number as i64)
    .bind(&room.branch)
    .bind(launch_id.to_string())
    .execute(db)
    .await?;
    Ok(())
}

fn request(r: &SqliteRow) -> Result<(RoomKey, RoomRequest)> {
    Ok((
        key(r),
        RoomRequest {
            id: r.get::<String, _>("id").parse()?,
            owner: r.get("owner"),
            launch_id: r.get::<String, _>("launch_id").parse()?,
            requested_by: r.get("requested_by"),
            at: parse_ts(&r.get::<String, _>("at")),
            state: state_of(&r.get::<String, _>("state"))?,
            answered_at: opt_ts(r, "answered_at"),
        },
    ))
}

const REQUEST_COLUMNS: &str =
    "id, repo, number, branch, owner, launch_id, requested_by, at, state, answered_at";

pub async fn add_request(db: &SqlitePool, room: &RoomKey, q: &RoomRequest) -> Result<()> {
    sqlx::query(&format!(
        "INSERT INTO room_requests ({REQUEST_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    ))
    .bind(q.id.to_string())
    .bind(&room.repo)
    .bind(room.number as i64)
    .bind(&room.branch)
    .bind(&q.owner)
    .bind(q.launch_id.to_string())
    .bind(&q.requested_by)
    .bind(ts(q.at))
    .bind(state_name(q.state))
    .bind(q.answered_at.map(ts))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn request_of(db: &SqlitePool, id: Uuid) -> Result<Option<(RoomKey, RoomRequest)>> {
    let r = sqlx::query(&format!(
        "SELECT {REQUEST_COLUMNS} FROM room_requests WHERE id = ?"
    ))
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    r.as_ref().map(request).transpose()
}

/// The room's requests, pending ones first, then the newest.
pub async fn requests_in(db: &SqlitePool, room: &RoomKey) -> Result<Vec<RoomRequest>> {
    let rows = sqlx::query(&format!(
        "SELECT {REQUEST_COLUMNS} FROM room_requests WHERE repo = ? AND number = ? AND branch = ? \
         ORDER BY state <> 'pending', at DESC"
    ))
    .bind(&room.repo)
    .bind(room.number as i64)
    .bind(&room.branch)
    .fetch_all(db)
    .await?;
    rows.iter().map(|r| request(r).map(|(_, q)| q)).collect()
}

/// Answer a pending request. False when it was answered already.
pub async fn answer(
    db: &SqlitePool,
    id: Uuid,
    to: RequestState,
    at: DateTime<Utc>,
) -> Result<bool> {
    let n = sqlx::query(
        "UPDATE room_requests SET state = ?, answered_at = ? WHERE id = ? AND state = 'pending'",
    )
    .bind(state_name(to))
    .bind(ts(at))
    .bind(id.to_string())
    .execute(db)
    .await?
    .rows_affected();
    Ok(n == 1)
}
