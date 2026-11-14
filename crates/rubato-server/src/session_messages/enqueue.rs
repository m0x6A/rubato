//! Messages the server itself queues for a launched session (ADR 0055): `plan_approved` at
//! priority 0, so it goes before any chat queued behind it. The same queue, states and dispatch
//! as a person's message; the server is never the author of a chat.

use super::dispatch;
use super::store::{self, MessageRow};
use crate::kernel::state::App;
use crate::launch;
use anyhow::Result;
use rubato_protocol::{SessionMessageKind, SessionMessageState, WebResource};
use uuid::Uuid;

/// The priority of `plan_approved`: before every chat (priority 1).
pub const PLAN_APPROVED_PRIORITY: i64 = 0;

/// Queue `kind` for the launch, as `author`, and try to push it. `None` when the launch is
/// unknown. The message goes to the launch's own companion and its paired user only.
pub async fn enqueue(
    app: &App,
    launch_id: Uuid,
    kind: SessionMessageKind,
    author: &str,
    priority: i64,
) -> Result<Option<Uuid>> {
    let Some(target) = launch::launch_target(&app.db, launch_id).await? else {
        return Ok(None);
    };
    let row = MessageRow {
        id: Uuid::new_v4(),
        launch_id,
        session_id: target.session_id.clone(),
        companion_id: target.companion_id,
        owner: target.user.clone(),
        issue: target.issue.clone(),
        kind,
        author: author.to_owned(),
        priority,
        state: SessionMessageState::Queued,
        created_at: app.clock.now(),
        pushed_at: None,
        outcome_at: None,
        detail: None,
    };
    store::insert(&app.db, &row).await?;
    tracing::info!(
        rubato.session_message.id = %row.id,
        rubato.session_message.kind = row.kind.name(),
        rubato.issue = %row.issue,
        rubato.user = %author,
        priority,
        "session message queued"
    );
    dispatch::dispatch(app, launch_id).await;
    app.notify_resource(WebResource::LaunchMessages { launch_id });
    Ok(Some(row.id))
}
