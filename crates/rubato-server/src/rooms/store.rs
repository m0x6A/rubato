//! Table `room_messages`.

use crate::kernel::db::{parse_ts, ts};
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
    })
}

/// Store a message in `room`. False when it is a seam notice the room already has.
pub async fn insert(db: &SqlitePool, room: &RoomKey, m: &RoomMessage) -> Result<bool> {
    let n = m.notice.as_ref();
    let r = sqlx::query(
        "INSERT OR IGNORE INTO room_messages (id, repo, number, branch, author_kind, author, text, at, notice_seam, notice_repo, notice_number, notice_branch) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(m.id.to_string())
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
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// The room's messages, oldest first.
pub async fn messages(db: &SqlitePool, room: &RoomKey) -> Result<Vec<RoomMessage>> {
    let rows = sqlx::query(
        "SELECT id, author_kind, author, text, at, notice_seam, notice_repo, notice_number, notice_branch \
         FROM room_messages WHERE repo = ? AND number = ? AND branch = ? ORDER BY at, rowid",
    )
    .bind(&room.repo)
    .bind(room.number as i64)
    .bind(&room.branch)
    .fetch_all(db)
    .await?;
    rows.iter().map(message).collect()
}
