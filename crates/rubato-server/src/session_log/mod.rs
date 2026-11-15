//! Slice `session_log`: a launched session's prompts and last replies, from its hooks, visible
//! to the team (ADR 0052, plan 122 design §D2). Owns table `session_log_entries`. Only from
//! companions that announce `hook_allowlist`, only for sessions with a launch.

mod handlers;
mod intake;
mod store;

#[cfg(test)]
mod tests;

pub use handlers::log;
pub use intake::on_hook;

/// The session message a launched session's prompt carried, if it was typed from one: how a
/// `Stop` is tied back to the message it answers (ADR 0051, ADR 0057).
pub async fn prompt_message(
    db: &sqlx::SqlitePool,
    launch_id: uuid::Uuid,
    session_id: &str,
    prompt_id: &str,
) -> anyhow::Result<Option<uuid::Uuid>> {
    store::prompt_message(db, launch_id, session_id, prompt_id).await
}
