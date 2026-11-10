//! *Working* sessions that went quiet, reported once each, swept every second.

use crate::sessions::Registry;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{CompanionMessage, Limits, SessionStatus, StaleSession};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, watch};

impl Registry {
    /// *Working* sessions with a trusted source and no signal for longer than `stale_secs`. Reported once each.
    pub fn stale(&mut self, now: DateTime<Utc>, stale_secs: u64) -> Vec<StaleSession> {
        let mut out = Vec::new();
        for e in &mut self.entries {
            if e.ended
                || e.stale_reported
                || e.status != SessionStatus::Working
                || !e.source.is_trusted()
            {
                continue;
            }
            let silent = now.signed_duration_since(e.last_seen).num_seconds();
            if silent > stale_secs as i64 {
                e.stale_reported = true;
                out.push(StaleSession {
                    companion_id: self.companion_id,
                    session_id: e.session_id.clone(),
                    issue: e.issue.clone(),
                    last_seen: e.last_seen,
                    silent_for_secs: silent as u64,
                    at: now,
                });
            }
        }
        out
    }
}

/// Every second: report sessions that went quiet.
pub async fn sweep(
    out: mpsc::UnboundedSender<CompanionMessage>,
    registry: Arc<Mutex<Registry>>,
    limits: Arc<std::sync::Mutex<Limits>>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = shutdown.changed() => return Ok(()),
        }
        let stale_secs = limits.lock().unwrap().session_stale_secs;
        let mut reg = registry.lock().await;
        let stale = reg.stale(Utc::now(), stale_secs);
        for s in stale {
            tracing::warn!(session = ?s.session_id, silent_for_secs = s.silent_for_secs, "{}", rubato_protocol::otel::event::SESSION_STALE);
            let _ = out.send(CompanionMessage::Stale(s));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rubato_protocol::HookEvent;
    use serde_json::json;
    use uuid::Uuid;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap()
    }

    fn hook(session: &str, name: &str, cwd: &str, at: DateTime<Utc>) -> HookEvent {
        HookEvent::from_payload(
            json!({"session_id": session, "hook_event_name": name, "cwd": cwd}),
            at,
        )
        .unwrap()
    }

    #[test]
    fn stale_reports_once_and_only_trusted_working() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.apply_hook(&hook("s1", "PreToolUse", "/x", now()));
        assert!(
            r.stale(now() + chrono::Duration::seconds(10), 120)
                .is_empty()
        );
        let late = now() + chrono::Duration::seconds(200);
        let s = r.stale(late, 120);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].silent_for_secs, 200);
        assert!(r.stale(late, 120).is_empty(), "reported once");
        r.apply_hook(&hook("s1", "PostToolUse", "/x", late));
        assert!(
            r.stale(late + chrono::Duration::seconds(300), 120).len() == 1,
            "reset after a new hook"
        );
    }
}
