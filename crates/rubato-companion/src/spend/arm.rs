//! After a hook: add up the transcript's spend, record it, and stop the session when it passes
//! the cap (`rubato.cap.hit` and a `CapHit` to the server; an event, not a log line). A
//! transcript the guard refuses (ADR 0063) is logged as `rubato.transcript.refused`, counted as
//! `rubato.events.dropped` with `rubato.reason = transcript`, and decides nothing.

use super::transcript::{Refusal, claude_projects_dir};
use crate::kernel::config::ModelPrice;
use crate::kernel::telemetry::Metrics;
use crate::kernel::tmux::Tmux;
use crate::sessions::Registry;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, event};
use rubato_protocol::{CapHit, CapKind, CompanionMessage, HookEvent, IssueRef};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::sync::{Mutex, mpsc};

/// The `rubato.reason` a refused transcript is counted under in `rubato.events.dropped`.
pub const DROP_REASON_TRANSCRIPT: &str = "transcript";

#[allow(clippy::too_many_arguments)]
pub async fn after_hook(
    ev: &HookEvent,
    (issue, transcript, cap): (Option<IssueRef>, Option<PathBuf>, Option<f64>),
    registry: &Mutex<Registry>,
    out: &mpsc::UnboundedSender<CompanionMessage>,
    pricing: &BTreeMap<String, ModelPrice>,
    tmux: &Tmux,
    metrics: &Metrics,
    headless: Option<&crate::sessions::Headless>,
) {
    let Some(path) = transcript else { return };
    if pricing.is_empty() {
        return;
    }
    let root = claude_projects_dir().unwrap_or_default();
    let pricing = pricing.clone();
    let shown = path.clone();
    let summed =
        tokio::task::spawn_blocking(move || super::sum::sum_file(&path, &root, &pricing)).await;
    let s = match summed {
        Ok(Ok(s)) => s,
        Ok(Err(refusal)) => {
            refused(ev, &shown, &refusal, metrics);
            return;
        }
        Err(_) => return,
    };
    let mut reg = registry.lock().await;
    reg.set_spend(&ev.session_id, s.usd);
    if let (Some(limit), Some(issue)) = (cap, issue)
        && s.usd > limit
    {
        let entry = reg.by_session(&ev.session_id).cloned();
        drop(reg);
        tracing::warn!(issue = %issue, spend = s.usd, limit, "{}", event::CAP_HIT);
        let _ = out.send(CompanionMessage::CapHit(CapHit {
            issue,
            session_id: Some(ev.session_id.clone()),
            kind: CapKind::Spend,
            value: s.usd,
            limit,
            at: Utc::now(),
        }));
        // A headless turn is a process group the companion holds (ADR 0062).
        let killed_turn = match (headless, entry.as_ref().and_then(|e| e.launch_id)) {
            (Some(h), Some(launch)) => h.kill(launch, crate::kernel::claude::KillReason::Cap),
            _ => false,
        };
        if !killed_turn && let Some(name) = entry.and_then(|e| e.tmux_session) {
            let t = tmux.clone();
            let _ = tokio::task::spawn_blocking(move || t.kill_session(&name)).await;
        }
    }
}

fn refused(ev: &HookEvent, path: &std::path::Path, refusal: &Refusal, metrics: &Metrics) {
    tracing::warn!(
        rubato.session_id = %ev.session_id,
        rubato.reason = refusal.reason(),
        path = %path.display(),
        detail = %refusal,
        "{}",
        event::TRANSCRIPT_REFUSED
    );
    metrics.events_dropped.add(
        1,
        &[KeyValue::new(attr::DROP_REASON, DROP_REASON_TRANSCRIPT)],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::metric_reader;
    use rubato_protocol::otel::metric;
    use uuid::Uuid;

    /// A forged hook naming `/dev/zero`: refused, counted, no spend recorded, no cap hit.
    #[tokio::test]
    async fn a_refused_transcript_is_counted_and_decides_nothing() {
        let (reader, metrics) = metric_reader::start();
        let registry = Mutex::new(Registry::new(Uuid::nil(), "m0x6A"));
        let ev = HookEvent::from_payload(
            serde_json::json!({"session_id": "s1", "hook_event_name": "Stop", "cwd": "/x",
                "transcript_path": "/dev/zero"}),
            Utc::now(),
        )
        .unwrap();
        registry.lock().await.apply_hook(&ev);
        let (out, mut sent) = mpsc::unbounded_channel();
        let mut pricing = BTreeMap::new();
        pricing.insert(
            "claude-".into(),
            ModelPrice {
                input: 1.0,
                output: 1.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
        );
        let tmux = Tmux::new("tmux", None);
        after_hook(
            &ev,
            (
                Some(IssueRef::new("acme/pantry", 7)),
                Some(PathBuf::from("/dev/zero")),
                Some(0.0),
            ),
            &registry,
            &out,
            &pricing,
            &tmux,
            &metrics,
            None,
        )
        .await;
        let points = reader.points(metric::EVENTS_DROPPED);
        assert_eq!(points.len(), 1, "{points:?}");
        assert_eq!(points[0].value, 1);
        assert_eq!(points[0].attrs[attr::DROP_REASON], DROP_REASON_TRANSCRIPT);
        assert_eq!(
            registry.lock().await.by_session("s1").unwrap().spend_usd,
            None
        );
        assert!(sent.try_recv().is_err(), "no CapHit");
    }
}
