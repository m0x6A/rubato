//! A hook event updates the registry: status, source, last seen, transcript, and the last hook
//! the idle check reads (plan 122 §D1). A hook from a new session id that names a launch the
//! registry knows (`X-Rubato-Launch`, after a `/clear`) moves that launch to the new id.

use crate::sessions::{Entry, Registry, identify};
use rubato_protocol::{HookEvent, HookEventKind, IssueRef, SessionStatus, StateSource};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Which pasted message a prompt carries: the newest whose text the prompt ends with, and
/// whether other text comes before it. A paste is appended to whatever a person had half-typed
/// (plan 122 §Spike), so that prompt is `mixed`; one that is the message alone is not.
pub fn carried(pasted: &[(Uuid, String)], prompt: &str) -> Option<(usize, bool)> {
    let prompt = prompt.trim_end();
    pasted.iter().enumerate().rev().find_map(|(i, (_, text))| {
        let text = text.trim_end();
        if text.is_empty() {
            return None;
        }
        let before = prompt.strip_suffix(text)?;
        Some((i, !before.trim().is_empty()))
    })
}

/// The hook says the turn ended and the prompt waits: `Stop`, or `Notification(idle_prompt)`.
pub fn waits_for_input(ev: &HookEvent) -> bool {
    match ev.hook_event_name {
        HookEventKind::Stop => true,
        HookEventKind::Notification => ev.notification_type.as_deref() == Some("idle_prompt"),
        _ => false,
    }
}

impl Registry {
    /// The session message a launched session's prompt carried, and whether it was mixed.
    /// Taken off the entry, so it tags one prompt only.
    pub fn take_carried(&mut self, session_id: &str, prompt: &str) -> Option<(Uuid, bool)> {
        let e = self.find_by_session(session_id)?;
        let (i, mixed) = carried(&e.pasted, prompt)?;
        let (id, _) = e.pasted.remove(i);
        Some((id, mixed))
    }

    /// A hook event. Returns the entry's issue so the caller can report per issue.
    pub fn apply_hook(&mut self, ev: &HookEvent) -> Option<IssueRef> {
        let implied = ev.implied_status();
        let now = ev.received_at;
        if let Some(launch) = ev.launch_id
            && self.by_session(&ev.session_id).is_none()
            && let Some(e) = self.find_by_launch(launch).filter(|e| !e.ended)
        {
            if ev.hook_event_name == HookEventKind::SessionEnd {
                // The end of the session a `/clear` replaced, arriving after its successor:
                // the launch has moved on, and nothing is recorded for the old id.
                return None;
            }
            let old = e.session_id.replace(ev.session_id.clone());
            // The new session writes its own transcript; the next hook names it.
            e.transcript_path = None;
            self.launches_changed = true;
            tracing::info!(launch_id = %launch, old_session_id = ?old, session_id = %ev.session_id, "launch moved to a new session id");
        }
        if let Some(e) = self.find_by_session(&ev.session_id) {
            if let Some(s) = implied {
                e.status = s;
                e.blocked_by = ev.implied_block_cause();
                e.last_hook = Some(ev.hook_event_name);
                e.last_hook_idle = waits_for_input(ev);
                e.last_hook_at = Some(now);
            }
            // Its hooks evidently arrive.
            e.hooks_lost = false;
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
            last_hook: implied.map(|_| ev.hook_event_name),
            last_hook_idle: implied.is_some() && waits_for_input(ev),
            last_hook_at: implied.map(|_| now),
            input_sent_at: None,
            hooks_lost: false,
            pasted: Vec::new(),
        };
        self.entries.push(entry);
        id.issue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::LaunchRecord;
    use chrono::{DateTime, TimeZone, Utc};
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

    fn launched(r: &mut Registry, launch: Uuid, session: &str) {
        r.restore_launches(
            vec![LaunchRecord {
                launch_id: launch,
                session_id: session.into(),
                worktree: PathBuf::from("/w"),
                pane_id: Some("%7".into()),
                tmux_session: Some("pantry@issue-7-restock".into()),
                issue: IssueRef::new("acme/pantry", 7),
                branch: Some("issue/7-restock".into()),
                spend_cap_usd: Some(20.0),
            }],
            None,
            false,
            now(),
        );
    }

    /// Plan 122 PR 4: the prompt that carried a pasted message names it once; text typed before
    /// it makes it mixed; a prompt that does not end with any pasted text names none.
    #[test]
    fn a_prompt_ending_with_a_pasted_message_carries_it_once() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let pasted = vec![
            (a, "line one\nline two".to_owned()),
            (b, "second".to_owned()),
        ];
        assert_eq!(carried(&pasted, "line one\nline two"), Some((0, false)));
        assert_eq!(carried(&pasted, "line one\nline two\n"), Some((0, false)));
        assert_eq!(
            carried(&pasted, "half-typed line one\nline two"),
            Some((0, true))
        );
        assert_eq!(
            carried(&pasted, "  second"),
            Some((1, false)),
            "only blanks before"
        );
        assert_eq!(carried(&pasted, "something else"), None);
        assert_eq!(
            carried(&pasted, "line one"),
            None,
            "a prefix is not the message"
        );

        let launch = Uuid::from_u128(7);
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        launched(&mut r, launch, "s7");
        r.mark_input_sent(launch, now(), a, "line one\nline two");
        assert_eq!(
            r.take_carried("s7", "half-typed line one\nline two"),
            Some((a, true))
        );
        assert_eq!(
            r.take_carried("s7", "line one\nline two"),
            None,
            "tagged once"
        );
        assert_eq!(r.take_carried("unknown", "x"), None);
    }

    /// Only a status-implying hook moves the idle check; `Stop` and `idle_prompt` wait for
    /// input, a permission prompt does not (typing there would answer it).
    #[test]
    fn the_last_hook_says_whether_the_session_waits_for_input() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        let t = |s| now() + chrono::Duration::seconds(s);
        r.apply_hook(&hook("s1", "UserPromptSubmit", "/nowhere", t(1)));
        let e = r.by_session("s1").unwrap();
        assert_eq!(e.last_hook, Some(HookEventKind::UserPromptSubmit));
        assert!(!e.waits_for_input());
        r.apply_hook(&hook("s1", "Stop", "/nowhere", t(2)));
        let e = r.by_session("s1").unwrap();
        assert_eq!(
            (e.last_hook, e.last_hook_at),
            (Some(HookEventKind::Stop), Some(t(2)))
        );
        assert!(e.waits_for_input());
        // A typeless Notification implies no status, so the idle check stays.
        r.apply_hook(&hook("s1", "Notification", "/nowhere", t(3)));
        assert!(r.by_session("s1").unwrap().waits_for_input());
        let mut ev = hook("s1", "Notification", "/nowhere", t(4));
        ev.notification_type = Some("permission_prompt".into());
        r.apply_hook(&ev);
        assert!(!r.by_session("s1").unwrap().waits_for_input());
        ev.notification_type = Some("idle_prompt".into());
        ev.received_at = t(5);
        r.apply_hook(&ev);
        assert!(r.by_session("s1").unwrap().waits_for_input());
        // A new session's first hook counts too.
        r.apply_hook(&hook("s2", "Stop", "/nowhere", t(6)));
        assert!(r.by_session("s2").unwrap().waits_for_input());
    }

    /// A `/clear` gives the launched session a new id; its hooks carry the launch header, so the
    /// launch follows instead of a second, unlaunched entry appearing (plan 122 §D1).
    #[test]
    fn a_new_session_id_with_a_known_launch_keeps_the_launch() {
        let launch = Uuid::from_u128(7);
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        launched(&mut r, launch, "old");
        r.apply_hook(&hook("old", "SessionEnd", "/w", now()));
        let mut start = hook("new", "SessionStart", "/w", now());
        start.launch_id = Some(launch);
        r.apply_hook(&start);
        assert_eq!(r.entries().len(), 1, "{:?}", r.entries());
        let e = r.by_launch(launch).unwrap();
        assert_eq!(e.session_id.as_deref(), Some("new"));
        assert_eq!(e.status, SessionStatus::Working);
        assert_eq!(e.issue, Some(IssueRef::new("acme/pantry", 7)));
        assert!(r.launches_changed);
        assert_eq!(r.snapshot()[0].launch_id, Some(launch));
        assert_eq!(r.launch_records()[0].session_id, "new");

        // The old session's end arriving late changes nothing.
        let mut tail = hook("old", "SessionEnd", "/w", now());
        tail.launch_id = Some(launch);
        assert_eq!(r.apply_hook(&tail), None);
        assert_eq!(r.entries().len(), 1);
        assert_eq!(r.by_launch(launch).unwrap().status, SessionStatus::Working);

        // Without the header, or with a launch nobody knows, a new id is a new session.
        r.apply_hook(&hook("other", "SessionStart", "/elsewhere", now()));
        let mut stranger = hook("third", "SessionStart", "/elsewhere", now());
        stranger.launch_id = Some(Uuid::from_u128(99));
        r.apply_hook(&stranger);
        assert_eq!(r.entries().len(), 3);
        assert_eq!(r.by_session("other").unwrap().launch_id, None);
        assert_eq!(r.by_session("third").unwrap().launch_id, None);
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
