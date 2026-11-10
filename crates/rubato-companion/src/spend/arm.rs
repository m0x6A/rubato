//! After a hook: add up the transcript's spend, record it, and stop the session when it passes
//! the cap (`rubato.cap.hit` and a `CapHit` to the server; an event, not a log line).

use crate::kernel::config::ModelPrice;
use crate::kernel::tmux::Tmux;
use crate::sessions::Registry;
use chrono::Utc;
use rubato_protocol::{CapHit, CapKind, CompanionMessage, HookEvent, IssueRef};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::sync::{Mutex, mpsc};

pub async fn after_hook(
    ev: &HookEvent,
    (issue, transcript, cap): (Option<IssueRef>, Option<PathBuf>, Option<f64>),
    registry: &Mutex<Registry>,
    out: &mpsc::UnboundedSender<CompanionMessage>,
    pricing: &BTreeMap<String, ModelPrice>,
    tmux: &Tmux,
) {
    let Some(path) = transcript else { return };
    if pricing.is_empty() {
        return;
    }
    let pricing = pricing.clone();
    if let Ok(Ok(s)) =
        tokio::task::spawn_blocking(move || super::sum::sum_file(&path, &pricing)).await
    {
        let mut reg = registry.lock().await;
        reg.set_spend(&ev.session_id, s.usd);
        if let (Some(limit), Some(issue)) = (cap, issue)
            && s.usd > limit
        {
            let entry = reg.by_session(&ev.session_id).cloned();
            drop(reg);
            tracing::warn!(issue = %issue, spend = s.usd, limit, "{}", rubato_protocol::otel::event::CAP_HIT);
            let _ = out.send(CompanionMessage::CapHit(CapHit {
                issue,
                session_id: Some(ev.session_id.clone()),
                kind: CapKind::Spend,
                value: s.usd,
                limit,
                at: Utc::now(),
            }));
            if let Some(name) = entry.and_then(|e| e.tmux_session) {
                let t = tmux.clone();
                let _ = tokio::task::spawn_blocking(move || t.kill_session(&name)).await;
            }
        }
    }
}
