//! Table `room_messages`.

use crate::kernel::db::{parse_ts, ts};
use crate::kernel::tenancy::TenantId;
use anyhow::{Result, anyhow};
use rubato_protocol::{IssueRef, RoomAuthorKind, RoomKey, RoomMessage, SeamNotice};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

fn kind(text: &str) -> Result<RoomAuthorKind> {
    match text {
        "person" => Ok(RoomAuthorKind::Person),
        "agent" => Ok(RoomAuthorKind::Agent),
        "system" => Ok(RoomAuthorKind::System),
        other => Err(anyhow!("unknown room author kind {other:?}")),
    }
}

fn message(r: &SqliteRow) -> Result<RoomMessage> {
    let notice = match (
        r.get::<Option<String>, _>("notice_seam"),
        r.get::<Option<String>, _>("notice_repo"),
        r.get::<Option<i64>, _>("notice_number"),
        r.get::<Option<String>, _>("notice_branch"),
    ) {
        (Some(seam), Some(repo), Some(number), Some(branch)) => Some(SeamNotice {
            seam,
            other: IssueRef::new(repo, number as u64),
            other_branch: branch,
        }),
        _ => None,
    };
    Ok(RoomMessage {
        id: r.get::<String, _>("id").parse()?,
        author_kind: kind(&r.get::<String, _>("author_kind"))?,
        author: r.get("author"),
        text: r.get("text"),
        at: parse_ts(&r.get::<String, _>("at")),
        notice,
        addressed: r
            .get::<Option<String>, _>("addressed")
            .map(|j| serde_json::from_str(&j))
            .transpose()?
            .unwrap_or_default(),
        turn_id: r
            .get::<Option<String>, _>("turn_id")
            .map(|t| t.parse())
            .transpose()?,
        spoken: r.get::<i64, _>("spoken") != 0,
    })
}

/// Store a message in `room`. False when it is a seam notice the room already has, or an
/// agent's reply to a turn it already answered.
pub async fn insert(
    db: &SqlitePool,
    t: &TenantId,
    room: &RoomKey,
    m: &RoomMessage,
) -> Result<bool> {
    let n = m.notice.as_ref();
    let r = sqlx::query(
        "INSERT OR IGNORE INTO room_messages (tenant_id, id, repo, number, branch, author_kind, author, text, at, notice_seam, notice_repo, notice_number, notice_branch, addressed, turn_id, spoken) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(t.as_str()).bind(m.id.to_string())
    .bind(&room.repo)
    .bind(room.number as i64)
    .bind(&room.branch)
    .bind(m.author_kind.name())
    .bind(&m.author)
    .bind(&m.text)
    .bind(ts(m.at))
    .bind(n.map(|n| n.seam.as_str()))
    .bind(n.map(|n| n.other.repo.as_str()))
    .bind(n.map(|n| n.other.number as i64))
    .bind(n.map(|n| n.other_branch.as_str()))
    .bind(if m.addressed.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&m.addressed)?)
    })
    .bind(m.turn_id.map(|t| t.to_string()))
    .bind(i64::from(m.spoken))
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// The room's messages, oldest first.
pub async fn messages(db: &SqlitePool, t: &TenantId, room: &RoomKey) -> Result<Vec<RoomMessage>> {
    let rows = sqlx::query(
        "SELECT id, author_kind, author, text, at, notice_seam, notice_repo, notice_number, notice_branch, addressed, turn_id, spoken FROM room_messages WHERE tenant_id = ? AND repo = ? AND number = ? AND branch = ? ORDER BY at, rowid",
    )
    .bind(t.as_str()).bind(&room.repo)
    .bind(room.number as i64)
    .bind(&room.branch)
    .fetch_all(db)
    .await?;
    rows.iter().map(message).collect()
}

/// One message by id, with the room it is in.
pub async fn by_id(
    db: &SqlitePool,
    t: &TenantId,
    id: uuid::Uuid,
) -> Result<Option<(RoomKey, RoomMessage)>> {
    let row = sqlx::query(
        "SELECT id, repo, number, branch, author_kind, author, text, at, notice_seam, notice_repo, notice_number, notice_branch, addressed, turn_id, spoken FROM room_messages WHERE tenant_id = ? AND id = ?",
    )
    .bind(t.as_str()).bind(id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(|r| {
        Ok((
            RoomKey {
                repo: r.get("repo"),
                number: r.get::<i64, _>("number") as u64,
                branch: r.get("branch"),
            },
            message(&r)?,
        ))
    })
    .transpose()
}

/// Whether a person's spoken message in `room` made turn `turn_id` (ADR 0073).
pub async fn turn_was_spoken(
    db: &SqlitePool,
    t: &TenantId,
    room: &RoomKey,
    turn_id: uuid::Uuid,
) -> Result<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM room_messages WHERE tenant_id = ? AND repo = ? AND number = ? AND branch = ? AND turn_id = ? AND author_kind = 'person' AND spoken = 1",
    )
    .bind(t.as_str()).bind(&room.repo)
    .bind(room.number as i64)
    .bind(&room.branch)
    .bind(turn_id.to_string())
    .fetch_one(db)
    .await?;
    Ok(n > 0)
}
