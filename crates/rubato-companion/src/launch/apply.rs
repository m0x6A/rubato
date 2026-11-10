//! What a launch knows about the registry: one live session per issue, and the entry a started
//! launch creates.

use crate::sessions::{Entry, Registry};
use chrono::{DateTime, Utc};
use rubato_protocol::{IssueRef, LaunchCommand, SessionStatus, StateSource};
use std::path::Path;

impl Registry {
    pub fn live_for_issue(&self, issue: &IssueRef) -> Option<&Entry> {
        self.entries.iter().find(|e| {
            !e.ended && e.status != SessionStatus::Ended && e.issue.as_ref() == Some(issue)
        })
    }

    /// A launch we performed: identity is known before the first hook. A tmux server that
    /// restarted reuses pane ids, so an older entry on the same pane is retired first.
    pub fn apply_launch(
        &mut self,
        cmd: &LaunchCommand,
        pane_id: &str,
        tmux_session: &str,
        worktree: &Path,
        now: DateTime<Utc>,
    ) {
        for e in self
            .entries
            .iter_mut()
            .filter(|e| e.pane_id.as_deref() == Some(pane_id) && !e.ended)
        {
            e.status = SessionStatus::Ended;
            e.ended = true;
            e.last_seen = now;
        }
        self.entries.push(Entry {
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
        });
    }
}
