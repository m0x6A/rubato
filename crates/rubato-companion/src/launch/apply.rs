//! What a launch knows about the registry: one live session per issue, and the entry a started
//! launch creates.

use crate::sessions::{Entry, Registry};
use chrono::{DateTime, Utc};
use rubato_protocol::{IssueRef, LaunchCommand, SessionStatus, StateSource};
use std::path::Path;

/// The session already running for an issue, as an owned description: the launcher reads it
/// after the registry lock is released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSession {
    pub session_id: Option<String>,
    pub tmux_session: Option<String>,
}

impl Registry {
    pub fn live_for_issue(&self, issue: &IssueRef) -> Option<LiveSession> {
        self.entries
            .iter()
            .find(|e| {
                !e.ended && e.status != SessionStatus::Ended && e.issue.as_ref() == Some(issue)
            })
            .map(|e| LiveSession {
                session_id: e.session_id.clone(),
                tmux_session: e.tmux_session.clone(),
            })
    }

    /// A launch we performed: identity is known before the first hook. The registry is not
    /// locked while the launch runs, so reconcile may already have added the new pane: an entry
    /// on the pane with no session id, in the launch's worktree, is upgraded in place. A tmux
    /// server that restarted reuses pane ids, so any other entry on the same pane is retired
    /// first.
    pub fn apply_launch(
        &mut self,
        cmd: &LaunchCommand,
        pane_id: &str,
        tmux_session: &str,
        worktree: &Path,
        now: DateTime<Utc>,
    ) {
        let mut discovered = None;
        for (i, e) in self
            .entries
            .iter_mut()
            .enumerate()
            .filter(|(_, e)| e.pane_id.as_deref() == Some(pane_id) && !e.ended)
        {
            if discovered.is_none() && e.session_id.is_none() && e.cwd == worktree {
                discovered = Some(i);
                continue;
            }
            e.status = SessionStatus::Ended;
            e.ended = true;
            e.last_seen = now;
        }
        let launched = Entry {
            session_id: Some(cmd.session_id.to_string()),
            pane_id: Some(pane_id.to_owned()),
            tmux_session: Some(tmux_session.to_owned()),
            window: Some(0),
            cwd: worktree.to_path_buf(),
            worktree: Some(worktree.to_path_buf()),
            branch: Some(cmd.branch.clone()),
            repo: Some(cmd.issue.repo.clone()),
            issue: Some(cmd.issue.clone()),
            status: SessionStatus::Idle,
            source: StateSource::Launch,
            last_seen: now,
            spend_usd: None,
            transcript_path: None,
            launch_id: Some(cmd.id),
            spend_cap_usd: Some(cmd.spend_cap_usd),
            stale_reported: false,
            ended: false,
        };
        match discovered {
            Some(i) => self.entries[i] = launched,
            None => self.entries.push(launched),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::tmux::Pane;
    use chrono::TimeZone;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn command() -> LaunchCommand {
        LaunchCommand {
            id: Uuid::from_u128(27),
            user: "m0x6A".into(),
            issue: IssueRef::new("m0x6A/rubato", 27),
            repo_url: "https://github.com/m0x6A/rubato.git".into(),
            branch: "issue/27-launch-without-lock".into(),
            base_ref: "origin/main".into(),
            node_ids: Vec::new(),
            spend_cap_usd: 20.0,
            attempt_limit: 3,
            attempt: 1,
            session_id: Uuid::from_u128(1),
            prompt: String::new(),
            otel_resource: BTreeMap::new(),
            map_version: None,
        }
    }

    #[test]
    fn a_pane_discovered_before_the_launch_is_recorded_is_upgraded() {
        let now = Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path();
        let cmd = command();
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        // Reconcile sees the new pane while the launch is still running.
        r.apply_panes(
            &[Pane {
                session: "rubato-27".into(),
                window: 0,
                pane_id: "%7".into(),
                cwd: worktree.to_path_buf(),
                current_command: "claude".into(),
                pid: 1,
            }],
            now,
        );
        assert_eq!(r.entries().len(), 1);
        assert_eq!(r.entries()[0].session_id, None);

        r.apply_launch(&cmd, "%7", "rubato-27", worktree, now);

        let on_pane: Vec<&Entry> = r
            .entries()
            .iter()
            .filter(|e| e.pane_id.as_deref() == Some("%7"))
            .collect();
        assert_eq!(on_pane.len(), 1, "one entry for the pane, not two");
        let e = on_pane[0];
        assert!(!e.ended);
        assert_ne!(e.status, SessionStatus::Ended);
        assert_eq!(e.session_id, Some(cmd.session_id.to_string()));
        assert_eq!(e.source, StateSource::Launch);
        assert_eq!(e.issue, Some(cmd.issue.clone()));
        assert_eq!(e.launch_id, Some(cmd.id));
        assert!(
            r.entries()
                .iter()
                .all(|e| !e.ended && e.status != SessionStatus::Ended),
            "no Ended entry"
        );
    }

    #[test]
    fn an_older_session_on_the_same_pane_is_still_retired() {
        let now = Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path();
        let mut first = command();
        first.session_id = Uuid::from_u128(2);
        let cmd = command();
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.apply_launch(&first, "%7", "rubato-27", worktree, now);

        r.apply_launch(&cmd, "%7", "rubato-27", worktree, now);

        assert_eq!(r.entries().len(), 2);
        assert!(r.entries()[0].ended);
        assert_eq!(r.entries()[0].status, SessionStatus::Ended);
        assert!(!r.entries()[1].ended);
        assert_eq!(r.entries()[1].session_id, Some(cmd.session_id.to_string()));
    }
}
