//! Merge what tmux reports into the registry: pane metadata only, never state (ADR 0070).
//! Vanished panes end; ended entries are swept after a cycle.

use super::{Entry, Registry, identify};
use crate::kernel::tmux::Pane;
use chrono::{DateTime, Utc};
use rubato_protocol::{SessionStatus, StateSource};

/// Commands a pane may run when Claude Code is in it. Node is included because `claude` is a Node program.
pub fn looks_like_agent(command: &str) -> bool {
    matches!(command, "claude" | "node" | "claude-code")
}

impl Registry {
    /// Pane metadata from tmux. New agent panes appear as `Scraped`/`Unknown`; vanished panes end.
    pub fn apply_panes(&mut self, panes: &[Pane], now: DateTime<Utc>) {
        self.apply_panes_with(panes, &Default::default(), now)
    }

    /// As [`Self::apply_panes`], with the panes whose process tree runs Claude Code under a
    /// wrapper (`kernel::ps`) counted as agent panes too.
    pub fn apply_panes_with(
        &mut self,
        panes: &[Pane],
        wrapped: &std::collections::HashSet<String>,
        now: DateTime<Utc>,
    ) {
        let agent_panes: Vec<&Pane> = panes
            .iter()
            .filter(|p| looks_like_agent(&p.current_command) || wrapped.contains(&p.pane_id))
            .collect();
        for p in &agent_panes {
            if let Some(e) = self.find_by_pane(&p.pane_id) {
                e.cwd = p.cwd.clone();
                e.tmux_session = Some(p.session.clone());
                e.window = Some(p.window);
                continue;
            }
            // Match a hook-only entry by cwd.
            if let Some(e) = self
                .entries
                .iter_mut()
                .find(|e| e.pane_id.is_none() && e.cwd == p.cwd)
            {
                e.pane_id = Some(p.pane_id.clone());
                e.tmux_session = Some(p.session.clone());
                e.window = Some(p.window);
                continue;
            }
            let id = identify(&p.cwd);
            if id.issue.is_some() && id.issue == self.launching {
                continue;
            }
            self.entries.push(Entry {
                session_id: None,
                pane_id: Some(p.pane_id.clone()),
                tmux_session: Some(p.session.clone()),
                window: Some(p.window),
                cwd: p.cwd.clone(),
                worktree: id.worktree,
                branch: id.branch,
                repo: id.repo,
                issue: id.issue,
                status: SessionStatus::Unknown,
                source: StateSource::Scraped,
                last_seen: now,
                spend_usd: None,
                transcript_path: None,
                launch_id: None,
                spend_cap_usd: None,
                stale_reported: false,
                ended: false,
                head: None,
                head_mtime: None,
                branch_drift: None,
                blocked_by: None,
                last_hook: None,
                last_hook_idle: false,
                last_hook_at: None,
                input_sent_at: None,
                hooks_lost: false,
                pasted: Vec::new(),
                plan_path: None,
                restored_hash: None,
            });
        }
        let live: std::collections::HashSet<&str> =
            panes.iter().map(|p| p.pane_id.as_str()).collect();
        for e in &mut self.entries {
            if let Some(pid) = &e.pane_id
                && !live.contains(pid.as_str())
                && !e.ended
            {
                e.status = SessionStatus::Ended;
                e.ended = true;
                e.last_seen = now;
            }
        }
    }

    /// Drop entries that ended more than a cycle ago.
    pub fn sweep(&mut self, now: DateTime<Utc>, keep_ended_secs: i64) {
        self.entries.retain(|e| {
            !(e.ended && now.signed_duration_since(e.last_seen).num_seconds() > keep_ended_secs)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rubato_protocol::HookEvent;
    use serde_json::json;
    use std::path::PathBuf;
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

    fn pane(id: &str, cwd: &str, command: &str) -> Pane {
        Pane {
            session: "t".into(),
            window: 1,
            pane_id: id.into(),
            cwd: PathBuf::from(cwd),
            current_command: command.into(),
            pid: 1,
        }
    }

    /// A new agent pane is `Scraped` and *unknown*: panes give metadata, never state (ADR 0070).
    #[test]
    fn a_new_agent_pane_is_scraped_and_unknown() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.apply_panes(
            &[pane("%5", "/w", "claude"), pane("%6", "/v", "bash")],
            now(),
        );
        assert_eq!(r.entries().len(), 1, "only the agent pane");
        let e = &r.entries()[0];
        assert_eq!(
            (e.status, e.source),
            (SessionStatus::Unknown, StateSource::Scraped)
        );
        let counts = r.counts_by_source();
        assert_eq!(counts[&StateSource::Scraped], 1);
        assert_eq!(counts.values().sum::<usize>(), r.entries().len());
    }

    /// Panes never change what a hook said.
    #[test]
    fn panes_never_change_a_hook_source() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.apply_hook(&hook("s1", "PreToolUse", "/x", now()));
        r.apply_panes(&[pane("%9", "/x", "claude")], now());
        assert_eq!(r.by_session("s1").unwrap().pane_id.as_deref(), Some("%9"));
        r.apply_panes(&[pane("%9", "/x", "claude")], now());
        let e = r.by_session("s1").unwrap();
        assert_eq!(
            (e.status, e.source),
            (SessionStatus::Working, StateSource::Hook)
        );
    }

    #[test]
    fn vanished_pane_ends_the_session() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        let pane = Pane {
            session: "t".into(),
            window: 1,
            pane_id: "%9".into(),
            cwd: PathBuf::from("/x"),
            current_command: "claude".into(),
            pid: 1,
        };
        r.apply_panes(&[pane], now());
        assert_eq!(r.entries().len(), 1);
        r.apply_panes(&[], now());
        assert_eq!(r.entries()[0].status, SessionStatus::Ended);
        r.sweep(now() + chrono::Duration::seconds(100), 60);
        assert!(r.entries().is_empty());
    }
}
