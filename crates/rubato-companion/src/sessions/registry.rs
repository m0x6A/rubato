//! The session registry: everything the companion knows about sessions on this machine, from
//! launches, hooks, pane metadata and tmux-ide, merged with a fixed precedence
//! (Hook > TmuxIde > Launch > Scraped). Never rendered as *working* without a trusted signal.
//! The methods that change it live with the slice that drives the change (design D7).

use crate::kernel::gitx;
use chrono::{DateTime, Utc};
use rubato_protocol::{IssueRef, SessionState, SessionStatus, StateSource, TmuxLocation};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub session_id: Option<String>,
    pub pane_id: Option<String>,
    pub tmux_session: Option<String>,
    pub window: Option<u32>,
    pub cwd: PathBuf,
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub repo: Option<String>,
    pub issue: Option<IssueRef>,
    pub status: SessionStatus,
    pub source: StateSource,
    pub last_seen: DateTime<Utc>,
    pub spend_usd: Option<f64>,
    pub transcript_path: Option<PathBuf>,
    pub launch_id: Option<Uuid>,
    pub spend_cap_usd: Option<f64>,
    pub stale_reported: bool,
    /// Set when the pane vanished; kept one cycle so the server sees `Ended`.
    pub ended: bool,
}

/// Identity from a path: worktree → branch → issue (ADR 0006).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Identity {
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub repo: Option<String>,
    pub issue: Option<IssueRef>,
}

pub fn identify(path: &Path) -> Identity {
    let Some(worktree) = gitx::worktree_root(path) else {
        return Identity::default();
    };
    let branch = gitx::current_branch(&worktree);
    let repo = gitx::origin_repo(&worktree);
    let issue = match (&branch, &repo) {
        (Some(b), Some(r)) => IssueRef::number_from_branch(b).map(|n| IssueRef::new(r.clone(), n)),
        _ => None,
    };
    Identity {
        worktree: Some(worktree),
        branch,
        repo,
        issue,
    }
}

#[derive(Debug, Default)]
pub struct Registry {
    pub companion_id: Uuid,
    pub user: String,
    /// Every slice that adds an `impl Registry` block reads this vector (design D7).
    pub(crate) entries: Vec<Entry>,
}

impl Registry {
    pub fn new(companion_id: Uuid, user: impl Into<String>) -> Self {
        Self {
            companion_id,
            user: user.into(),
            entries: Vec::new(),
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub(crate) fn find_by_session(&mut self, session_id: &str) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|e| e.session_id.as_deref() == Some(session_id))
    }

    pub(crate) fn find_by_pane(&mut self, pane_id: &str) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|e| e.pane_id.as_deref() == Some(pane_id))
    }

    /// The snapshot sent to the server.
    pub fn snapshot(&self) -> Vec<SessionState> {
        self.entries
            .iter()
            .map(|e| SessionState {
                session_id: e.session_id.clone(),
                companion_id: self.companion_id,
                user: self.user.clone(),
                tmux: e.tmux_session.as_ref().map(|s| TmuxLocation {
                    socket: None,
                    session: s.clone(),
                    window: e.window,
                    pane: e.pane_id.clone(),
                }),
                cwd: e.cwd.to_string_lossy().into_owned(),
                worktree: e
                    .worktree
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                branch: e.branch.clone(),
                repo: e.repo.clone(),
                issue: e.issue.clone(),
                status: e.status,
                source: e.source,
                last_seen: e.last_seen,
                spend_usd: e.spend_usd,
            })
            .collect()
    }

    pub fn by_session(&self, session_id: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.session_id.as_deref() == Some(session_id))
    }

    /// Count by status, for the `rubato.sessions.by_state` gauge. Every status is present, zero
    /// included: a gauge keeps its last value, so a status that empties must be recorded as 0.
    pub fn counts(&self) -> HashMap<SessionStatus, usize> {
        let mut m: HashMap<_, _> = STATUSES.iter().map(|s| (*s, 0)).collect();
        for e in &self.entries {
            *m.entry(e.status).or_insert(0) += 1;
        }
        m
    }

    /// Count by source, for the `rubato.sessions.discovered` gauge. Every source is present,
    /// zero included, for the same reason.
    pub fn counts_by_source(&self) -> HashMap<StateSource, usize> {
        let mut m: HashMap<_, _> = SOURCES.iter().map(|s| (*s, 0)).collect();
        for e in &self.entries {
            *m.entry(e.source).or_insert(0) += 1;
        }
        m
    }
}

/// Every session status and every state source, for the gauges that also report zero.
const STATUSES: [SessionStatus; 5] = [
    SessionStatus::Working,
    SessionStatus::Idle,
    SessionStatus::Blocked,
    SessionStatus::Ended,
    SessionStatus::Unknown,
];
const SOURCES: [StateSource; 4] = [
    StateSource::Hook,
    StateSource::TmuxIde,
    StateSource::Launch,
    StateSource::Scraped,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_include_every_status() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        let counts = r.counts();
        assert_eq!(counts.len(), 5, "{counts:?}");
        assert!(counts.values().all(|n| *n == 0), "{counts:?}");
        r.entries.push(Entry {
            session_id: Some("s1".into()),
            pane_id: None,
            tmux_session: None,
            window: None,
            cwd: PathBuf::from("/x"),
            worktree: None,
            branch: None,
            repo: None,
            issue: None,
            status: SessionStatus::Working,
            source: StateSource::Hook,
            last_seen: Utc::now(),
            spend_usd: None,
            transcript_path: None,
            launch_id: None,
            spend_cap_usd: None,
            stale_reported: false,
            ended: false,
        });
        let counts = r.counts();
        assert_eq!(counts.len(), 5, "{counts:?}");
        assert_eq!(counts[&SessionStatus::Working], 1);
        for status in [
            SessionStatus::Idle,
            SessionStatus::Blocked,
            SessionStatus::Ended,
            SessionStatus::Unknown,
        ] {
            assert_eq!(counts.get(&status), Some(&0), "{status:?}");
        }
    }
}
