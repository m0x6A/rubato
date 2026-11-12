//! The hook arm of the runtime and the dropped-request counter. Order (design §9.1): count →
//! under the lock apply, read transcript and cap, snapshot → send `Hook` → send `Sessions`. The
//! runtime then hands the result to `spend::after_hook`. The arm runs inside a
//! `rubato.hook.forward` span.

use crate::kernel::telemetry::Metrics;
use crate::sessions::Registry;
use anyhow::Result;
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{CompanionMessage, HookEvent, IssueRef};
use std::path::PathBuf;
use tokio::sync::{Mutex, mpsc};
use tracing::Instrument;
use uuid::Uuid;

/// What the spend step needs from the hook: the session's issue, transcript and spend cap.
pub type HookOutcome = (Option<IssueRef>, Option<PathBuf>, Option<f64>);

/// `rubato.hook.forwarded` exists at 0 for this companion from startup. After a restart the
/// sessions launched earlier post to a dead port: without this series `SessionWorkingNoHooks`
/// would have no hook side to compare with and stay silent in exactly that case.
pub fn seed_forwarded(metrics: &Metrics, companion_id: Uuid) {
    metrics.hook_forwarded.add(
        0,
        &[KeyValue::new(attr::COMPANION_ID, companion_id.to_string())],
    );
}

pub async fn on_event(
    ev: &HookEvent,
    registry: &Mutex<Registry>,
    out: &mpsc::UnboundedSender<CompanionMessage>,
    metrics: &Metrics,
    companion_id: Uuid,
) -> HookOutcome {
    let span = tracing::info_span!(
        span::HOOK_FORWARD,
        rubato.session_id = %ev.session_id,
        rubato.hook_event = ?ev.hook_event_name,
    );
    forward(ev, registry, out, metrics, companion_id)
        .instrument(span)
        .await
}

async fn forward(
    ev: &HookEvent,
    registry: &Mutex<Registry>,
    out: &mpsc::UnboundedSender<CompanionMessage>,
    metrics: &Metrics,
    companion_id: Uuid,
) -> HookOutcome {
    metrics.hook_forwarded.add(
        1,
        &[
            // No session id: one series per session would grow for the life of the process
            // (#43). The `rubato.hook.forward` span around this carries it.
            KeyValue::new(attr::HOOK_EVENT, format!("{:?}", ev.hook_event_name)),
            KeyValue::new(attr::COMPANION_ID, companion_id.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::metric_reader;
    use rubato_protocol::otel::metric;

    #[tokio::test]
    async fn hook_forwarded_carries_the_companion_id() {
        let id = Uuid::new_v4();
        let (reader, metrics) = metric_reader::start();

        // The startup seed: one series at 0 with only the companion id.
        seed_forwarded(&metrics, id);
        let points = reader.points(metric::HOOK_FORWARDED);
        assert_eq!(points.len(), 1, "{points:?}");
        assert_eq!(points[0].value, 0);
        assert_eq!(
            points[0].attrs,
            [(attr::COMPANION_ID.to_owned(), id.to_string())].into()
        );

        // A real hook: counted with the companion id next to the session and the event.
        let registry = Mutex::new(Registry::new(id, "m0x6A"));
        let (out, mut sent) = mpsc::unbounded_channel();
        let ev = HookEvent::from_payload(
            serde_json::json!({"session_id": "s1", "hook_event_name": "PreToolUse", "cwd": "/x"}),
            chrono::Utc::now(),
        )
        .unwrap();
        on_event(&ev, &registry, &out, &metrics, id).await;
        let points = reader.points(metric::HOOK_FORWARDED);
        assert_eq!(points.len(), 2, "{points:?}");
        for p in &points {
            assert_eq!(
                p.attrs.get(attr::COMPANION_ID),
                Some(&id.to_string()),
                "{p:?}"
            );
        }
        let hook = points
            .iter()
            .find(|p| p.attrs.contains_key(attr::HOOK_EVENT))
            .unwrap();
        assert_eq!(hook.value, 1);
        assert_eq!(hook.attrs[attr::HOOK_EVENT], "PreToolUse");
        // One series per companion and event, not per session (#43): the session id is on the
        // `rubato.hook.forward` span only.
        assert!(!hook.attrs.contains_key(attr::SESSION_ID), "{hook:?}");

        // A second session adds no series.
        let ev2 = HookEvent::from_payload(
            serde_json::json!({"session_id": "s2", "hook_event_name": "PreToolUse", "cwd": "/y"}),
            chrono::Utc::now(),
        )
        .unwrap();
        on_event(&ev2, &registry, &out, &metrics, id).await;
        let points = reader.points(metric::HOOK_FORWARDED);
        assert_eq!(points.len(), 2, "{points:?}");
        let hook = points
            .iter()
            .find(|p| p.attrs.contains_key(attr::HOOK_EVENT))
            .unwrap();
        assert_eq!(hook.value, 2);

        // The arm's order is unchanged: `Hook`, then `Sessions`.
        assert!(matches!(sent.recv().await, Some(CompanionMessage::Hook(_))));
        assert!(matches!(
            sent.recv().await,
            Some(CompanionMessage::Sessions { .. })
        ));
    }
}
