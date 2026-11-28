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
/// The priority of a chat, and of a room turn (ADR 0057).
pub const CHAT_PRIORITY: i64 = 1;

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
        tenant: target.tenant.clone(),
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
        via_launch: None,
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

/// Why `enqueue_chat_via` queued nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum ChatRefused {
    UnknownLaunch,
    Empty,
    TooLong,
}

/// A chat for `launch_id`, sent for its owner by the owner's lead agent `via` (ADR 0072), with the
/// same checks as the web route: not empty, and at most `Limits.session_message_max_chars`
/// counting `text` as it will be typed. The caller has checked whose launch it is.
pub async fn enqueue_chat_via(
    app: &App,
    launch_id: Uuid,
    text: &str,
    author: &str,
    via: Uuid,
) -> Result<std::result::Result<Uuid, ChatRefused>> {
    if text.trim().is_empty() {
        return Ok(Err(ChatRefused::Empty));
    }
    if text.chars().count() as u64 > app.config.limits.session_message_max_chars {
        return Ok(Err(ChatRefused::TooLong));
    }
    let Some(target) = launch::launch_target(&app.db, launch_id).await? else {
        return Ok(Err(ChatRefused::UnknownLaunch));
    };
    let row = MessageRow {
        tenant: target.tenant.clone(),
        id: Uuid::new_v4(),
        launch_id,
        session_id: target.session_id.clone(),
        companion_id: target.companion_id,
        owner: target.user.clone(),
        issue: target.issue.clone(),
        kind: SessionMessageKind::Chat {
            text: text.to_owned(),
            author: author.to_owned(),
        },
        author: author.to_owned(),
        priority: CHAT_PRIORITY,
        state: SessionMessageState::Queued,
        created_at: app.clock.now(),
        pushed_at: None,
        outcome_at: None,
        detail: None,
        via_launch: Some(via),
    };
    store::insert(&app.db, &row).await?;
    tracing::info!(
        rubato.session_message.id = %row.id,
        rubato.issue = %row.issue,
        rubato.user = %author,
        via_launch = %via,
        "session message queued for a lead"
    );
    dispatch::dispatch(app, launch_id).await;
    app.notify_resource(WebResource::LaunchMessages { launch_id });
    Ok(Ok(row.id))
}

/// How many messages wait (queued or in flight) for each of `launch_ids`.
pub async fn waiting_counts(
    app: &App,
    launch_ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, u32>> {
    store::waiting_counts(&app.db, launch_ids).await
}
