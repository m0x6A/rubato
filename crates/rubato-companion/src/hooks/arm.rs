//! The hook arm of the runtime and the dropped-request counter. Order (design §9.1): count →
//! under the lock apply, read transcript and cap, snapshot → send `Hook` → send `Sessions`. The
//! runtime then hands the result to `spend::after_hook`.

use crate::kernel::telemetry::Metrics;
use crate::sessions::Registry;
use anyhow::Result;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{CompanionMessage, HookEvent, IssueRef};
use std::path::PathBuf;
use tokio::sync::{Mutex, mpsc};

/// What the spend step needs from the hook: the session's issue, transcript and spend cap.
pub type HookOutcome = (Option<IssueRef>, Option<PathBuf>, Option<f64>);

pub async fn on_event(
    ev: &HookEvent,
    registry: &Mutex<Registry>,
    out: &mpsc::UnboundedSender<CompanionMessage>,
    metrics: &Metrics,
) -> HookOutcome {
    metrics.hook_forwarded.add(
        1,
        &[
            KeyValue::new(attr::HOOK_EVENT, format!("{:?}", ev.hook_event_name)),
            KeyValue::new(attr::SESSION_ID, ev.session_id.clone()),
        ],
    );
    let (issue, transcript, cap, snapshot) = {
        let mut reg = registry.lock().await;
        let issue = reg.apply_hook(ev);
        let e = reg.by_session(&ev.session_id).cloned();
        (
            issue,
            e.as_ref().and_then(|e| e.transcript_path.clone()),
            e.as_ref().and_then(|e| e.spend_cap_usd),
            reg.snapshot(),
        )
    };
    let _ = out.send(CompanionMessage::Hook(ev.clone()));
    let _ = out.send(CompanionMessage::Sessions { sessions: snapshot });
    (issue, transcript, cap)
}

/// Dropped hook requests → `rubato.events.dropped`. Ends only when the hook server is gone.
pub async fn count_dropped(
    mut dropped_rx: mpsc::UnboundedReceiver<&'static str>,
    metrics: std::sync::Arc<Metrics>,
) -> Result<()> {
    while let Some(reason) = dropped_rx.recv().await {
        metrics
            .events_dropped
            .add(1, &[KeyValue::new(attr::DROP_REASON, reason)]);
    }
    Ok(())
}
