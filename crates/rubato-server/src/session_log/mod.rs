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
    app: &crate::kernel::state::App,
    launch_id: uuid::Uuid,
    session_id: &str,
    prompt_id: &str,
) -> anyhow::Result<Option<uuid::Uuid>> {
    // The log is the launch's repository's tenant's (ADR 0087).
    let Some(target) = crate::launch::launch_target(&app.db, launch_id).await? else {
        return Ok(None);
    };
    let t = app.repo_tenant(&target.issue.repo)?;
    store::prompt_message(&app.db, &t, launch_id, session_id, prompt_id).await
}

/// The newest reply's text of each of `launch_ids`, in one query (ADR 0072's `fleet_list`).
pub async fn last_replies(
    db: &sqlx::SqlitePool,
    launch_ids: &[uuid::Uuid],
) -> anyhow::Result<std::collections::HashMap<uuid::Uuid, String>> {
    store::last_replies(db, launch_ids).await
}
