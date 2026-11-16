//! Table `session_messages`. Every state change is one `UPDATE … WHERE state = ?` (or a single
//! statement that picks and marks), so two dispatches or a late result cannot move a row twice.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{IssueRef, SessionMessageKind, SessionMessageState, SessionMessageView};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use uuid::Uuid;

/// One stored message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub id: Uuid,
    pub launch_id: Uuid,
    pub session_id: Option<String>,
    pub companion_id: Uuid,
    pub owner: String,
    pub issue: IssueRef,
    pub kind: SessionMessageKind,
    pub author: String,
    pub priority: i64,
    pub state: SessionMessageState,
    pub created_at: DateTime<Utc>,
    pub pushed_at: Option<DateTime<Utc>>,
    pub outcome_at: Option<DateTime<Utc>>,
    pub detail: Option<String>,
    /// The lead agent that sent it for its owner (ADR 0072).
    pub via_launch: Option<Uuid>,
}

impl MessageRow {
    pub fn view(&self) -> SessionMessageView {
        SessionMessageView {
            id: self.id,
            launch_id: self.launch_id,
            kind: self.kind.name().to_owned(),
            text: self.kind.text().to_owned(),
            author: self.author.clone(),
            state: self.state,
            created_at: self.created_at,
            pushed_at: self.pushed_at,
            outcome_at: self.outcome_at,
            detail: self.detail.clone(),
            via_launch: self.via_launch,
        }
    }
}

fn row(r: &SqliteRow) -> Result<MessageRow> {
    let opt_ts = |c: &str| r.get::<Option<String>, _>(c).map(|s| parse_ts(&s));
    Ok(MessageRow {
        id: r.get::<String, _>("id").parse()?,
        launch_id: r.get::<String, _>("launch_id").parse()?,
        session_id: r.get("session_id"),
        companion_id: r.get::<String, _>("companion_id").parse()?,
        owner: r.get("owner"),
        issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
        kind: serde_json::from_str(&r.get::<String, _>("body"))?,
        author: r.get("author"),
        priority: r.get("priority"),
        state: SessionMessageState::parse(&r.get::<String, _>("state"))
            .ok_or_else(|| anyhow::anyhow!("unknown message state"))?,
        created_at: parse_ts(&r.get::<String, _>("created_at")),
        pushed_at: opt_ts("pushed_at"),
        outcome_at: opt_ts("outcome_at"),
        detail: r.get("detail"),
        via_launch: r
            .get::<Option<String>, _>("via_launch")
            .and_then(|v| v.parse().ok()),
    })
}

const COLUMNS: &str = "id, launch_id, session_id, companion_id, owner, repo, number, kind, body, author, priority, state, created_at, pushed_at, outcome_at, detail, via_launch";

/// Whether a `plan_approved` for this approval is queued, in flight, typed, or of unknown
/// outcome (ADR 0055): a later launch must not get a second one. A refused or failed one is not.
pub async fn plan_approved_open(db: &SqlitePool, approval_id: Uuid) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM session_messages WHERE kind = 'plan_approved' AND json_extract(body, '$.approval_id') = ? \
         AND state IN ('queued', 'sent', 'delivered', 'delivery_unknown') LIMIT 1",
    )
    .bind(approval_id.to_string())
    .fetch_optional(db)
    .await?
    .is_some())
}

pub async fn insert(db: &SqlitePool, m: &MessageRow) -> Result<()> {
    sqlx::query(&format!(
        "INSERT INTO session_messages ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    ))
    .bind(m.id.to_string())
    .bind(m.launch_id.to_string())
    .bind(&m.session_id)
    .bind(m.companion_id.to_string())
    .bind(&m.owner)
    .bind(&m.issue.repo)
    .bind(m.issue.number as i64)
    .bind(m.kind.name())
    .bind(serde_json::to_string(&m.kind)?)
    .bind(&m.author)
    .bind(m.priority)
    .bind(m.state.name())
    .bind(ts(m.created_at))
    .bind(m.pushed_at.map(ts))
    .bind(m.outcome_at.map(ts))
    .bind(&m.detail)
    .bind(m.via_launch.map(|v| v.to_string()))
    .execute(db)
    .await?;
    Ok(())
}

pub async fn get(db: &SqlitePool, id: Uuid) -> Result<Option<MessageRow>> {
    let r = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM session_messages WHERE id = ?"
    ))
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    r.as_ref().map(row).transpose()
}

/// The launch's messages, oldest first.
pub async fn for_launch(db: &SqlitePool, launch_id: Uuid) -> Result<Vec<MessageRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM session_messages WHERE launch_id = ? ORDER BY created_at, id"
    ))
    .bind(launch_id.to_string())
    .fetch_all(db)
    .await?;
    rows.iter().map(row).collect()
}

/// Mark the launch's next queued message `sent` and return it, but only when none of the launch's
/// messages is `sent` already: at most one in flight per launch, `plan_approved` first
/// (`priority`), then oldest first. One statement, so two dispatches cannot both send. Without
/// `plan_handoff` a queued `plan_approved` is passed over and waits for a companion that can
/// type it, and without `room_turns` so is a `room_turn`; the chat behind it still goes.
pub async fn take_next(
    db: &SqlitePool,
    launch_id: Uuid,
    now: DateTime<Utc>,
    plan_handoff: bool,
    room_turns: bool,
) -> Result<Option<MessageRow>> {
    let l = launch_id.to_string();
    let r = sqlx::query(&format!(
        "UPDATE session_messages SET state = 'sent', pushed_at = ? \
         WHERE id = (SELECT id FROM session_messages WHERE launch_id = ? AND state = 'queued' \
                       AND (kind <> 'plan_approved' OR ?) AND (kind <> 'room_turn' OR ?) \
                     ORDER BY priority, created_at, id LIMIT 1) \
           AND NOT EXISTS (SELECT 1 FROM session_messages WHERE launch_id = ? AND state = 'sent') \
         RETURNING {COLUMNS}"
    ))
    .bind(ts(now))
    .bind(&l)
    .bind(plan_handoff)
    .bind(room_turns)
    .bind(&l)
    .fetch_optional(db)
    .await?;
    r.as_ref().map(row).transpose()
}

/// When each approval's `plan_approved` was typed into its session (`delivered`), by approval.
pub async fn plan_approved_delivered(db: &SqlitePool) -> Result<HashMap<Uuid, DateTime<Utc>>> {
    let rows = sqlx::query(
        "SELECT body, outcome_at FROM session_messages WHERE kind = 'plan_approved' AND state = 'delivered'",
    )
    .fetch_all(db)
    .await?;
    let mut out = HashMap::new();
    for r in rows {
        let Ok(SessionMessageKind::PlanApproved { approval_id, .. }) =
            serde_json::from_str(&r.get::<String, _>("body"))
        else {
            continue;
        };
        if let Some(at) = r.get::<Option<String>, _>("outcome_at") {
            let at = parse_ts(&at);
            out.entry(approval_id)
                .and_modify(|t: &mut DateTime<Utc>| *t = (*t).min(at))
                .or_insert(at);
        }
    }
    Ok(out)
}

/// A pushed row whose push did not reach a socket goes back to the queue.
pub async fn unsend(db: &SqlitePool, id: Uuid) -> Result<bool> {
    let n = sqlx::query("UPDATE session_messages SET state = 'queued', pushed_at = NULL WHERE id = ? AND state = 'sent'")
        .bind(id.to_string())
        .execute(db)
        .await?
        .rows_affected();
    Ok(n == 1)
}

/// The companion's `sent` rows, oldest first: pushed again with the same id at its Hello.
pub async fn sent_for_companion(db: &SqlitePool, companion_id: Uuid) -> Result<Vec<MessageRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM session_messages WHERE companion_id = ? AND state = 'sent' ORDER BY created_at, id"
    ))
    .bind(companion_id.to_string())
    .fetch_all(db)
    .await?;
    rows.iter().map(row).collect()
}

/// Launches of this companion with something queued.
pub async fn queued_launches(db: &SqlitePool, companion_id: Uuid) -> Result<Vec<Uuid>> {
    let rows = sqlx::query("SELECT DISTINCT launch_id FROM session_messages WHERE companion_id = ? AND state = 'queued'")
        .bind(companion_id.to_string())
        .fetch_all(db)
        .await?;
    rows.iter()
        .map(|r| Ok(r.get::<String, _>("launch_id").parse()?))
        .collect()
}

/// Launches of this companion whose queued messages name this session (a hook without the launch
/// header, from a session started before it existed).
pub async fn launches_for_session(
    db: &SqlitePool,
    companion_id: Uuid,
    session_id: &str,
) -> Result<Vec<Uuid>> {
    let rows = sqlx::query("SELECT DISTINCT launch_id FROM session_messages WHERE companion_id = ? AND session_id = ? AND state = 'queued'")
        .bind(companion_id.to_string())
        .bind(session_id)
        .fetch_all(db)
        .await?;
    rows.iter()
        .map(|r| Ok(r.get::<String, _>("launch_id").parse()?))
        .collect()
}

/// Settle a row from the companion's answer. Applies to `sent` and to `delivery_unknown` (a late
/// answer); returns whether the row moved.
pub async fn settle(
    db: &SqlitePool,
    id: Uuid,
    companion_id: Uuid,
    to: SessionMessageState,
    detail: Option<&str>,
    now: DateTime<Utc>,
) -> Result<bool> {
    let n = sqlx::query("UPDATE session_messages SET state = ?, outcome_at = ?, detail = ? WHERE id = ? AND companion_id = ? AND state IN ('sent', 'delivery_unknown')")
        .bind(to.name())
        .bind(ts(now))
        .bind(detail)
        .bind(id.to_string())
        .bind(companion_id.to_string())
        .execute(db)
        .await?
        .rows_affected();
    Ok(n == 1)
}

/// The session was working: back to the queue, nothing was typed.
pub async fn requeue(db: &SqlitePool, id: Uuid, companion_id: Uuid) -> Result<bool> {
    let n = sqlx::query("UPDATE session_messages SET state = 'queued' WHERE id = ? AND companion_id = ? AND state = 'sent'")
        .bind(id.to_string())
        .bind(companion_id.to_string())
        .execute(db)
        .await?
        .rows_affected();
    Ok(n == 1)
}

/// `sent` rows pushed before `before`, for the sweep.
pub async fn sent_before(db: &SqlitePool, before: DateTime<Utc>) -> Result<Vec<MessageRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM session_messages WHERE state = 'sent' AND pushed_at < ?"
    ))
    .bind(ts(before))
    .fetch_all(db)
    .await?;
    rows.iter().map(row).collect()
}

/// A pushed row with no answer: never pushed again.
pub async fn mark_unknown(
    db: &SqlitePool,
    id: Uuid,
    why: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let n = sqlx::query("UPDATE session_messages SET state = 'delivery_unknown', outcome_at = ?, detail = ? WHERE id = ? AND state = 'sent'")
        .bind(ts(now))
        .bind(why)
        .bind(id.to_string())
        .execute(db)
        .await?
        .rows_affected();
    Ok(n == 1)
}

/// The launch's session changed (a `/clear`): its unsettled rows follow it.
pub async fn move_session(db: &SqlitePool, launch_id: Uuid, session_id: &str) -> Result<u64> {
    Ok(sqlx::query("UPDATE session_messages SET session_id = ? WHERE launch_id = ? AND state IN ('queued', 'sent') AND (session_id IS NULL OR session_id <> ?)")
        .bind(session_id)
        .bind(launch_id.to_string())
        .bind(session_id)
        .execute(db)
        .await?
        .rows_affected())
}

/// Every queued row, for the queue-age gauge.
pub async fn queued(db: &SqlitePool) -> Result<Vec<MessageRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM session_messages WHERE state = 'queued'"
    ))
    .fetch_all(db)
    .await?;
    rows.iter().map(row).collect()
}

/// Messages queued or in flight per launch, in one grouped query (ADR 0072's `fleet_list`).
pub async fn waiting_counts(db: &SqlitePool, launch_ids: &[Uuid]) -> Result<HashMap<Uuid, u32>> {
    if launch_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let marks = vec!["?"; launch_ids.len()].join(", ");
    let sql = format!(
        "SELECT launch_id, COUNT(*) AS n FROM session_messages WHERE state IN ('queued', 'sent') AND launch_id IN ({marks}) GROUP BY launch_id"
    );
    let mut q = sqlx::query(&sql);
    for id in launch_ids {
        q = q.bind(id.to_string());
    }
    let mut out = HashMap::new();
    for r in q.fetch_all(db).await? {
        if let Ok(id) = r.get::<String, _>("launch_id").parse() {
            out.insert(id, r.get::<i64, _>("n") as u32);
        }
    }
    Ok(out)
}
