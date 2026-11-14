//! `plan_approved` for the owner's session (ADR 0054, ADR 0055, plan 122 §D6): a recorded
//! approval is queued at priority 0 for the owner's live launched session of that draft, so it
//! goes before any chat. With no live launch it waits unbound: the approval row is the record of
//! it, and nothing is queued. Binding it when a launch of the draft starts is PR 8; that launch
//! calls [`deliver_to_launch`].

use super::records::RubatoApproval;
use crate::kernel::state::App;
use crate::session_messages;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{SessionMessageKind, SessionState, SessionStatus};
use uuid::Uuid;

/// Rubato's page for an approval, written into accepted ADRs and the link comment's record:
/// `<public url>/approvals/<id>`. Served by the web app like any page path; stable across
/// restarts and workspaces, since approvals are not scoped to a map.
pub fn approval_url(app: &App, id: Uuid) -> String {
    format!(
        "{}/approvals/{id}",
        app.config.public_url.trim_end_matches('/')
    )
}

/// The owner's launched session of the draft that is still there (any status but ended), the
/// most recently seen first: its launch.
pub(super) fn live_launch(
    sessions: &[SessionState],
    a: &RubatoApproval,
    owner: &str,
    now: DateTime<Utc>,
    stale_secs: u64,
) -> Option<Uuid> {
    sessions
        .iter()
        .filter(|s| {
            s.launch_id.is_some()
                && s.user.eq_ignore_ascii_case(owner)
                && s.branch.as_deref() == Some(a.branch.as_str())
                && s.repo
                    .as_deref()
                    .is_none_or(|r| r.eq_ignore_ascii_case(&a.issue.repo))
                && s.issue.as_ref().is_none_or(|i| *i == a.issue)
                && s.effective_status(now, stale_secs) != SessionStatus::Ended
        })
        .max_by_key(|s| s.last_seen)
        .and_then(|s| s.launch_id)
}

/// The message for `a`: every field the companion types into `plan.ts handoff`.
pub fn plan_approved(app: &App, a: &RubatoApproval, plan_path: &str) -> SessionMessageKind {
    SessionMessageKind::PlanApproved {
        approval_id: a.id,
        approved_by: a.approved_by.clone(),
        approved_at: a.approved_at,
        approved_sha: a.plan_sha.clone(),
        bundle_hash: a.bundle_hash.clone(),
        branch: a.branch.clone(),
        plan_path: plan_path.to_owned(),
        self_approved: a.self_approved,
        url: approval_url(app, a.id),
    }
}

/// Queue `plan_approved` for `launch_id` at priority 0, as the approver. The hook for PR 8.
pub async fn deliver_to_launch(
    app: &App,
    a: &RubatoApproval,
    plan_path: &str,
    launch_id: Uuid,
) -> Result<Option<Uuid>> {
    session_messages::enqueue(
        app,
        launch_id,
        plan_approved(app, a, plan_path),
        &a.approved_by,
        session_messages::PLAN_APPROVED_PRIORITY,
    )
    .await
}

/// A new approval: to the owner's live launched session, or waiting for one.
pub(super) async fn on_recorded(app: &App, a: &RubatoApproval, owner: &str, plan_path: &str) {
    let now = app.clock.now();
    let launch = live_launch(
        &app.latest_sessions(),
        a,
        owner,
        now,
        app.config.limits.session_stale_secs,
    );
    let Some(launch_id) = launch else {
        tracing::info!(
            rubato.issue = %a.issue,
            rubato.branch = %a.branch,
            rubato.approval_id = %a.id,
            "plan approval waits for a launch of the draft"
        );
        return;
    };
    match deliver_to_launch(app, a, plan_path, launch_id).await {
        Ok(Some(id)) => tracing::info!(
            rubato.issue = %a.issue,
            rubato.approval_id = %a.id,
            rubato.launch_id = %launch_id,
            rubato.session_message.id = %id,
            "plan approval queued for the owner's session"
        ),
        Ok(None) => tracing::warn!(
            rubato.approval_id = %a.id,
            rubato.launch_id = %launch_id,
            "plan approval: the session's launch is not known"
        ),
        Err(e) => tracing::error!(
            rubato.approval_id = %a.id,
            error = %format!("{e:#}"),
            "queueing the plan approval failed"
        ),
    }
}
