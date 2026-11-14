//! Slice `session_messages`: messages a person sends from Rubato to a session they launched,
//! queued here and typed by their own companion when the session waits for input (ADR 0051,
//! plan 122 design §D1). Owns table `session_messages`. At most once: a message that was pushed
//! and went unanswered while its companion stayed live is *delivery unknown*, never pushed again.

mod dispatch;
mod enqueue;
mod handlers;
mod store;

#[cfg(test)]
mod tests;

pub use dispatch::{
    dispatch, on_hello, on_hook, on_result, on_sessions, record_gauges, run_sweep, sweep,
};
pub use enqueue::{CHAT_PRIORITY, PLAN_APPROVED_PRIORITY, enqueue};
pub use handlers::{list, send};
pub use store::plan_approved_open;

/// A queued room turn (ADR 0057), found by its session message: where it goes and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomTurnRef {
    pub message_id: uuid::Uuid,
    pub launch_id: uuid::Uuid,
    pub companion_id: uuid::Uuid,
    pub owner: String,
    pub room: rubato_protocol::RoomKey,
    pub turn_id: uuid::Uuid,
    pub mode: rubato_protocol::RoomTurnMode,
}

/// The room turn this session message carries, if it is one.
pub async fn room_turn(
    db: &sqlx::SqlitePool,
    message_id: uuid::Uuid,
) -> anyhow::Result<Option<RoomTurnRef>> {
    let Some(row) = store::get(db, message_id).await? else {
        return Ok(None);
    };
    Ok(match row.kind {
        rubato_protocol::SessionMessageKind::RoomTurn {
            room,
            turn_id,
            mode,
            ..
        } => Some(RoomTurnRef {
            message_id,
            launch_id: row.launch_id,
            companion_id: row.companion_id,
            owner: row.owner,
            room,
            turn_id,
            mode,
        }),
        _ => None,
    })
}

/// When each approval's `plan_approved` was typed into its session, by approval id.
pub async fn plan_approved_delivered(
    db: &sqlx::SqlitePool,
) -> anyhow::Result<std::collections::HashMap<uuid::Uuid, chrono::DateTime<chrono::Utc>>> {
    store::plan_approved_delivered(db).await
}
