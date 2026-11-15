//! From hooks to log entries (ADR 0052). A `UserPromptSubmit` gives the turn's prompt and a
//! `Stop` its last reply, both with Claude Code's `prompt_id`. Only from a companion that
//! announced `hook_allowlist` (an older one forwards full payloads, which never enter the log)
//! and only for a launched session (Q3).

use super::store;
use crate::kernel::state::App;
use crate::launch;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{
    HookEvent, HookEventKind, LogKind, LogRole, SessionLogEntry, WebResource, capability,
};
use uuid::Uuid;

fn dropped(app: &App, reason: &'static str) {
    app.metrics
        .session_log_dropped
        .add(1, &[KeyValue::new(attr::DROP_REASON, reason)]);
}

pub async fn on_hook(app: &App, companion_id: Uuid, hook: &HookEvent) {
    let (role, field) = match hook.hook_event_name {
        HookEventKind::UserPromptSubmit => (LogRole::User, "prompt"),
        HookEventKind::Stop => (LogRole::Assistant, "last_assistant_message"),
        _ => return,
    };
    let Some(launch_id) = hook.launch_id else {
        return;
    };
    if !app.announced(companion_id, capability::HOOK_ALLOWLIST) {
        return;
    }
    let payload = hook.payload.as_ref();
    let Some(prompt_id) = payload
        .and_then(|p| p.get("prompt_id"))
        .and_then(|v| v.as_str())
    else {
        dropped(app, "no_prompt_id");
        return;
    };
    let Some(text) = payload
        .and_then(|p| p.get(field))
        .and_then(|v| v.as_str())
        .filter(|t| !t.trim().is_empty())
    else {
        dropped(app, "no_text");
        return;
    };
    let target = match launch::launch_target(&app.db, launch_id).await {
        Ok(Some(t)) if t.companion_id == companion_id => t,
        Ok(_) => {
            dropped(app, "unknown_launch");
            return;
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "session log: reading the launch failed");
            return;
        }
    };
    let kind = match role {
        LogRole::User => LogKind::Prompt,
        LogRole::Assistant => LogKind::of_reply(text),
    };
    let entry = SessionLogEntry {
        session_id: hook.session_id.clone(),
        prompt_id: prompt_id.to_owned(),
        role,
        kind,
        text: text.to_owned(),
        truncated: hook.truncated,
        message_id: hook.message_id,
        mixed: hook.mixed,
        at: hook.received_at,
    };
    match store::insert(&app.db, launch_id, &target.issue, &entry).await {
        Ok(true) => {
            app.metrics.session_log_entries.add(
                1,
                &[
                    KeyValue::new(attr::LOG_ROLE, role.name()),
                    KeyValue::new(attr::LOG_KIND, kind.name()),
                ],
            );
            tracing::info!(rubato.issue = %target.issue, rubato.session_id = %hook.session_id, kind = kind.name(), "session log entry");
            app.notify_resource(WebResource::LaunchLog { launch_id });
        }
        Ok(false) => tracing::debug!("session log entry already stored"),
        Err(e) => tracing::error!(error = %format!("{e:#}"), "storing a session log entry failed"),
    }
}
