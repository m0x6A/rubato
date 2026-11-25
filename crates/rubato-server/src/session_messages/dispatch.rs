//! Dispatch (ADR 0051, plan 122 design §D1). The server keeps no idle state: it pushes a
//! launch's next message on enqueue, on an idle hook, on the companion's Hello and after a
//! `not_idle`; the companion decides whether the session waits for input and answers once.
//! Nothing that was pushed and went unanswered is ever pushed to a live companion again.

use super::store::{self, MessageRow};
use crate::kernel::state::App;
use crate::launch;
use chrono::{DateTime, Duration, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{
    HookEvent, HookEventKind, ServerMessage, SessionMessage, SessionMessageOutcome,
    SessionMessageResult, SessionMessageState, SessionState, SessionStatus, StateSource,
    WebResource, capability,
};
use tracing::Instrument;
use uuid::Uuid;

/// A pushed message unanswered this long while its companion stayed live is *delivery unknown*.
pub const ANSWER_WITHIN_SECS: i64 = 60;

pub(super) fn wire(row: &MessageRow) -> ServerMessage {
    ServerMessage::SessionMessage(SessionMessage {
        id: row.id,
        user: row.owner.clone(),
        launch_id: row.launch_id,
        session_id: row.session_id.clone(),
        issue: row.issue.clone(),
        kind: row.kind.clone(),
        created_at: row.created_at,
    })
}

fn settled(app: &App, row: &MessageRow, outcome: &str) {
    app.metrics.session_messages.add(
        1,
        &[
            KeyValue::new(attr::OUTCOME, outcome.to_owned()),
            KeyValue::new(attr::SESSION_MESSAGE_KIND, row.kind.name()),
        ],
    );
}

/// Push the launch's next queued message, if its companion is live and nothing is in flight.
pub async fn dispatch(app: &App, launch_id: Uuid) {
    let span = tracing::info_span!(span::SESSION_MESSAGE_DISPATCH, rubato.launch_id = %launch_id);
    async move {
        let now = app.clock.now();
        let target = match launch::launch_target(&app.db, launch_id).await {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "dispatch: reading the launch failed");
                return;
            }
        };
        if !app.companion_live(target.companion_id, capability::SESSION_INPUT, now) {
            return;
        }
        let handoff = app.announced(target.companion_id, capability::PLAN_HANDOFF);
        let turns = app.announced(target.companion_id, capability::ROOM_TURNS);
        let row = match store::take_next(&app.db, launch_id, now, handoff, turns).await {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "dispatch: taking the next message failed");
                return;
            }
        };
        if app.send_to_companion(target.companion_id, wire(&row)) {
            tracing::info!(
                rubato.session_message.id = %row.id,
                rubato.session_message.kind = row.kind.name(),
                rubato.issue = %row.issue,
                "session message dispatched"
            );
        } else if let Err(e) = store::unsend(&app.db, row.id).await {
            tracing::error!(error = %format!("{e:#}"), "dispatch: un-sending failed");
        }
        app.notify_resource(WebResource::LaunchMessages { launch_id });
    }
    .instrument(span)
    .await
}

/// A hook that says the session waits for input: try its launch's queue. A hook from a session
/// started before the launch header existed is matched by its session id.
pub async fn on_hook(app: &App, companion_id: Uuid, hook: &HookEvent) {
    let idle = match hook.hook_event_name {
        HookEventKind::Stop => true,
        HookEventKind::Notification => hook.notification_type.as_deref() == Some("idle_prompt"),
        _ => false,
    };
    if !idle {
        return;
    }
    let launches = match hook.launch_id {
        Some(l) => vec![l],
        None => store::launches_for_session(&app.db, companion_id, &hook.session_id)
            .await
            .unwrap_or_default(),
    };
    for l in launches {
        dispatch(app, l).await;
    }
}

/// The companion said Hello: push its unanswered rows again with the same id (its
/// `deliveries.json` decides whether to type), then try every launch with something queued.
pub async fn on_hello(app: &App, companion_id: Uuid) {
    if !app.announced(companion_id, capability::SESSION_INPUT) {
        return;
    }
    let sent = store::sent_for_companion(&app.db, companion_id)
        .await
        .unwrap_or_default();
    for row in &sent {
        if app.send_to_companion(companion_id, wire(row)) {
            tracing::info!(rubato.session_message.id = %row.id, "session message pushed again after hello");
        }
    }
    for l in store::queued_launches(&app.db, companion_id)
        .await
        .unwrap_or_default()
    {
        dispatch(app, l).await;
    }
}

/// A snapshot names each launched session's current id: unsettled rows follow a `/clear`.
pub async fn on_sessions(app: &App, sessions: &[SessionState]) {
    for s in sessions {
        if let (Some(l), Some(sid)) = (s.launch_id, s.session_id.as_deref())
            && let Err(e) = store::move_session(&app.db, l, sid).await
        {
            tracing::error!(error = %format!("{e:#}"), "moving messages to a new session id failed");
        }
    }
}

/// The companion's answer. Applied once, idempotently by id, including a late one and one whose
/// envelope was a replay.
pub async fn on_result(app: &App, companion_id: Uuid, r: &SessionMessageResult) {
    let now = app.clock.now();
    let row = match store::get(&app.db, r.message_id).await {
        Ok(Some(row)) if row.companion_id == companion_id => row,
        Ok(Some(_)) => {
            tracing::warn!(rubato.session_message.id = %r.message_id, %companion_id, "session message result from another companion ignored");
            return;
        }
        Ok(None) => {
            tracing::warn!(rubato.session_message.id = %r.message_id, "result for an unknown session message");
            return;
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading a session message failed");
            return;
        }
    };
    let (to, detail): (Option<SessionMessageState>, Option<String>) = match &r.outcome {
        SessionMessageOutcome::Delivered => (Some(SessionMessageState::Delivered), None),
        SessionMessageOutcome::NotIdle => (None, None),
        SessionMessageOutcome::Refused { reason } => (
            Some(SessionMessageState::Refused),
            Some(reason.name().to_owned()),
        ),
        SessionMessageOutcome::Interrupted => (
            Some(SessionMessageState::DeliveryUnknown),
            Some("interrupted".to_owned()),
        ),
        SessionMessageOutcome::Failed { error } => {
            (Some(SessionMessageState::Failed), Some(error.clone()))
        }
    };
    let moved = match to {
        Some(state) => {
            store::settle(&app.db, row.id, companion_id, state, detail.as_deref(), now).await
        }
        None => store::requeue(&app.db, row.id, companion_id).await,
    };
    match moved {
        Ok(true) => {
            settled(app, &row, r.outcome.name());
            tracing::info!(rubato.session_message.id = %row.id, rubato.outcome = r.outcome.name(), "session message outcome");
        }
        Ok(false) => {
            tracing::debug!(rubato.session_message.id = %row.id, "session message result already applied")
        }
        Err(e) => tracing::error!(error = %format!("{e:#}"), "settling a session message failed"),
    }
    app.notify_resource(WebResource::LaunchMessages {
        launch_id: row.launch_id,
    });
}

/// `sent` rows unanswered for `ANSWER_WITHIN_SECS` while their companion stayed live become
/// *delivery unknown*. A companion that went away keeps its rows `sent`; its Hello pushes them
/// again. Returns how many moved.
pub async fn sweep(app: &App) -> usize {
    let now = app.clock.now();
    let before = now - Duration::seconds(ANSWER_WITHIN_SECS);
    let rows = store::sent_before(&app.db, before)
        .await
        .unwrap_or_default();
    let mut moved = 0;
    for row in rows {
        if !app.companion_live(row.companion_id, capability::SESSION_INPUT, now) {
            continue;
        }
        if store::mark_unknown(&app.db, row.id, "no answer", now)
            .await
            .unwrap_or(false)
        {
            moved += 1;
            settled(app, &row, SessionMessageState::DeliveryUnknown.name());
            tracing::warn!(rubato.session_message.id = %row.id, rubato.issue = %row.issue, "session message delivery unknown");
            app.notify_resource(WebResource::LaunchMessages {
                launch_id: row.launch_id,
            });
        }
    }
    moved
}

/// The sweep, every 10 s until shutdown.
pub async fn run_sweep(app: App, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {}
            _ = shutdown.changed() => return,
        }
        sweep(&app).await;
    }
}

/// The session waits for input, by a trusted hook (`Stop` → idle, `idle_prompt` → blocked).
fn waits_for_input(s: &SessionState) -> bool {
    s.source == StateSource::Hook
        && (s.status == SessionStatus::Idle
            || (s.status == SessionStatus::Blocked
                && s.blocked_by == Some(rubato_protocol::BlockCause::IdlePrompt)))
}

/// Every capability a companion may announce in Hello, each a series on
/// `rubato.companions.by_capability` (0 when no connected companion has it). The cutover waits
/// for `session_input`, `draft_mirror` and `plan_handoff` on every companion (QUICKSTART §3); the
/// list once stopped at the first (#274). `skill:` and `claude_auth:` carry values and stay off.
const COMPANION_CAPABILITIES: [&str; 10] = [
    capability::HOOK_ALLOWLIST,
    capability::SESSION_INPUT,
    capability::DRAFT_MIRROR,
    capability::PLAN_HANDOFF,
    capability::DRAFT_RESTORE,
    capability::SESSION_STOP,
    capability::ROOM_TURNS,
    capability::AGENT_MCP,
    capability::HEADLESS,
    capability::FLEET,
];

/// Gauges, each poll tick: the oldest message queued for a launch whose session waits for input
/// (alert `SessionMessageStuck`), and connected companions by capability.
pub async fn record_gauges(app: &App, sessions: &[SessionState]) {
    let now: DateTime<Utc> = app.clock.now();
    let waiting: std::collections::HashSet<Uuid> = sessions
        .iter()
        .filter(|s| waits_for_input(s))
        .filter_map(|s| s.launch_id)
        .collect();
    let oldest = store::queued(&app.db)
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| waiting.contains(&r.launch_id))
        .map(|r| now.signed_duration_since(r.created_at).num_seconds().max(0) as u64)
        .max()
        .unwrap_or(0);
    app.metrics
        .session_messages_oldest_idle_queued
        .record(oldest, &[]);
    for (cap, n) in app.capability_counts(&COMPANION_CAPABILITIES) {
        app.metrics
            .companions_by_capability
            .record(n, &[KeyValue::new(attr::CAPABILITY, cap)]);
    }
}
