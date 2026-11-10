//! Merge what tmux and tmux-ide report into the registry. Vanished panes end; ended entries are
//! swept after a cycle.

use super::{Entry, Registry, identify};
use crate::kernel::tmux::Pane;
use crate::kernel::tmux_ide::Fleet;
use chrono::{DateTime, TimeZone, Utc};
use rubato_protocol::{SessionStatus, StateSource};
use std::path::PathBuf;

fn rank(s: StateSource) -> u8 {
    match s {
        StateSource::Hook => 3,
        StateSource::TmuxIde => 2,
        StateSource::Launch => 1,
        StateSource::Scraped => 0,
    }
}

fn tmux_ide_status(state: &str) -> SessionStatus {
    match state {
        "working" => SessionStatus::Working,
        "blocked" => SessionStatus::Blocked,
        "done" | "idle" => SessionStatus::Idle,
        _ => SessionStatus::Unknown,
    }
}

/// Commands a pane may run when Claude Code is in it. Node is included because `claude` is a Node program.
fn looks_like_agent(command: &str) -> bool {
    matches!(command, "claude" | "node" | "claude-code")
}

impl Registry {
    /// Pane metadata from tmux. New agent panes appear as `Scraped`/`Unknown`; vanished panes end.
    pub fn apply_panes(&mut self, panes: &[Pane], now: DateTime<Utc>) {
        let agent_panes: Vec<&Pane> = panes
            .iter()
            .filter(|p| looks_like_agent(&p.current_command))
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

    /// tmux-ide's view. Hook-authority states are trusted (`TmuxIde`); screen-matched ones are `Scraped`.
    pub fn apply_fleet(&mut self, fleet: &Fleet, now: DateTime<Utc>) {
        for a in fleet.agents() {
            let trusted = a.has_hook_authority();
            let status = tmux_ide_status(&a.state);
            let seen = a
                .since
                .and_then(|s| Utc.timestamp_opt(s as i64, 0).single())
                .unwrap_or(now);
            let src = if trusted {
                StateSource::TmuxIde
            } else {
                StateSource::Scraped
            };
            if let Some(e) = self.find_by_pane(&a.pane_id) {
                if rank(src) >= rank(e.source)
                    || e.source == StateSource::Hook && e.last_seen < seen
                {
                    if trusted {
                        e.status = status;
                        e.last_seen = seen.max(e.last_seen);
                        e.source = if e.source == StateSource::Hook {
                            StateSource::Hook
                        } else {
                            StateSource::TmuxIde
                        };
                    } else if e.source == StateSource::Scraped {
                        e.status = SessionStatus::Unknown;
                    }
                }
                continue;
            }
            let cwd = a.dir.as_deref().map(PathBuf::from).unwrap_or_default();
            let id = identify(&cwd);
            self.entries.push(Entry {
                session_id: None,
                pane_id: Some(a.pane_id.clone()),
                tmux_session: Some(a.session.clone()),
                window: a.window_index,
                cwd,
                worktree: id.worktree,
                branch: id.branch,
                repo: id.repo,
                issue: id.issue,
                status: if trusted {
                    status
                } else {
                    SessionStatus::Unknown
                },
                source: src,
                last_seen: seen,
                spend_usd: None,
                transcript_path: None,
                launch_id: None,
                spend_cap_usd: None,
                stale_reported: false,
                ended: false,
            });
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
    fn fleet_without_authority_is_scraped_and_unknown() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        let f = Fleet::parse(include_str!("../../features/fixtures/team.json")).unwrap();
        r.apply_fleet(&f, now());
        let scraped: Vec<&Entry> = r
            .entries()
            .iter()
            .filter(|e| e.source == StateSource::Scraped)
            .collect();
        assert!(!scraped.is_empty());
        assert!(scraped.iter().all(|e| e.status == SessionStatus::Unknown));
        let trusted: Vec<&Entry> = r
            .entries()
            .iter()
            .filter(|e| e.source == StateSource::TmuxIde)
            .collect();
        assert_eq!(trusted.len(), 1);
        assert_eq!(trusted[0].status, SessionStatus::Working);
    }

    #[test]
    fn fleet_never_downgrades_a_hook_source() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.apply_hook(&hook("s1", "PreToolUse", "/x", now()));
        let pane = Pane {
            session: "t".into(),
            window: 1,
            pane_id: "%9".into(),
            cwd: PathBuf::from("/x"),
            current_command: "claude".into(),
            pid: 1,
        };
        r.apply_panes(&[pane], now());
        assert_eq!(r.by_session("s1").unwrap().pane_id.as_deref(), Some("%9"));
        let f = Fleet::parse(r#"{"projects":[{"sessions":[{"agents":[{"paneId":"%9","state":"blocked","since":null}]}]}]}"#).unwrap();
        r.apply_fleet(&f, now());
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
