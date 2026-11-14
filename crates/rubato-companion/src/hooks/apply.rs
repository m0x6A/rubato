//! A hook event updates the registry: status, source, last seen, transcript.

use crate::sessions::{Entry, Registry, identify};
use rubato_protocol::{HookEvent, IssueRef, SessionStatus, StateSource};
use std::path::{Path, PathBuf};

impl Registry {
    /// A hook event. Returns the entry's issue so the caller can report per issue.
    pub fn apply_hook(&mut self, ev: &HookEvent) -> Option<IssueRef> {
        let implied = ev.implied_status();
        let now = ev.received_at;
        if let Some(e) = self.find_by_session(&ev.session_id) {
            if let Some(s) = implied {
                e.status = s;
                e.blocked_by = ev.implied_block_cause();
            }
            e.source = StateSource::Hook;
            e.last_seen = now;
            e.stale_reported = false;
            if let Some(t) = &ev.transcript_path {
                e.transcript_path = Some(PathBuf::from(t));
            }
            if !ev.cwd.is_empty() {
                e.cwd = PathBuf::from(&ev.cwd);
            }
            return e.issue.clone();
        }
        // A session we did not launch: identity from its cwd.
        let id = identify(Path::new(&ev.cwd));
        let entry = Entry {
            session_id: Some(ev.session_id.clone()),
            pane_id: None,
            tmux_session: None,
            window: None,
            cwd: PathBuf::from(&ev.cwd),
            worktree: id.worktree,
            branch: id.branch,
            repo: id.repo,
            issue: id.issue.clone(),
            status: implied.unwrap_or(SessionStatus::Unknown),
            source: StateSource::Hook,
            last_seen: now,
            spend_usd: None,
            transcript_path: ev.transcript_path.as_ref().map(PathBuf::from),
            launch_id: None,
            spend_cap_usd: None,
            stale_reported: false,
            ended: false,
            head: None,
            head_mtime: None,
            branch_drift: None,
            blocked_by: ev.implied_block_cause(),
        };
        self.entries.push(entry);
        id.issue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use rubato_protocol::HookEventKind;
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
    fn hook_sets_status_and_source() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.apply_hook(&hook("s1", "UserPromptSubmit", "/nowhere", now()));
        let e = r.by_session("s1").unwrap();
        assert_eq!(
            (e.status, e.source),
            (SessionStatus::Working, StateSource::Hook)
        );
        assert_eq!(e.issue, None, "no git here, so no identity");
        r.apply_hook(&hook("s1", "Notification", "/nowhere", now()));
        // Notification without a type implies nothing; status stays.
        assert_eq!(r.by_session("s1").unwrap().status, SessionStatus::Working);
        let mut ev = hook("s1", "Notification", "/nowhere", now());
        ev.notification_type = Some("permission_prompt".into());
        r.apply_hook(&ev);
        assert_eq!(r.by_session("s1").unwrap().status, SessionStatus::Blocked);
        assert_eq!(ev.hook_event_name, HookEventKind::Notification);
    }

    /// Every status-implying hook sets or clears the cause; a typeless `Notification` leaves
    /// status and cause alone (ADR 0043).
    #[test]
    fn hook_sets_and_clears_the_block_cause() {
        use rubato_protocol::BlockCause;
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.apply_hook(&hook("s1", "StopFailure", "/nowhere", now()));
        let e = r.by_session("s1").unwrap();
        assert_eq!(
            (e.status, e.blocked_by),
            (SessionStatus::Blocked, Some(BlockCause::StopFailure))
        );
        r.apply_hook(&hook("s1", "Notification", "/nowhere", now()));
        let e = r.by_session("s1").unwrap();
        assert_eq!(
            (e.status, e.blocked_by),
            (SessionStatus::Blocked, Some(BlockCause::StopFailure)),
            "a typeless Notification implies nothing"
        );
        let mut idle = hook("s1", "Notification", "/nowhere", now());
        idle.notification_type = Some("idle_prompt".into());
        r.apply_hook(&idle);
        assert_eq!(
            r.by_session("s1").unwrap().blocked_by,
            Some(BlockCause::IdlePrompt)
        );
        assert_eq!(r.snapshot()[0].blocked_by, Some(BlockCause::IdlePrompt));
        r.apply_hook(&hook("s1", "UserPromptSubmit", "/nowhere", now()));
        let e = r.by_session("s1").unwrap();
        assert_eq!((e.status, e.blocked_by), (SessionStatus::Working, None));
        assert_eq!(r.snapshot()[0].blocked_by, None);
    }
}
