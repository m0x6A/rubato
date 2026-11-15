//! The session registry: everything the companion knows about sessions on this machine, from
//! launches, hooks, headless turns and pane metadata, merged with a fixed precedence
//! (Hook = Headless > Launch > Scraped; ADR 0070 retired tmux-ide's state). Never rendered as
//! *working* without a trusted signal.
//! The methods that change it live with the slice that drives the change (design D7).

use crate::kernel::gitx;
use chrono::{DateTime, Utc};
use rubato_protocol::{
    BlockCause, HookEventKind, IssueRef, SessionState, SessionStatus, StateSource, TmuxLocation,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// How many pasted texts an entry keeps for matching its next prompts.
pub const PASTED_KEPT: usize = 5;

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
    /// The worktree's `HEAD` file and its modification time when identity was last read: a
    /// checkout changes it, so identity is read again only then (design 80 §5).
    pub head: Option<PathBuf>,
    pub head_mtime: Option<std::time::SystemTime>,
    /// A launched session now on another branch than its launch branch.
    pub branch_drift: Option<String>,
    /// Which hook blocked the session (ADR 0043): set by the hook that implied *blocked*,
    /// cleared by the next status a hook sets. Sent only with *blocked*.
    pub blocked_by: Option<BlockCause>,
    /// The last hook that implied a status, and when it arrived (plan 122 §D1). Set by hooks
    /// alone: a status discovery sets never proves a turn ended.
    pub last_hook: Option<HookEventKind>,
    /// That hook says the session waits for input: `Stop`, or `Notification(idle_prompt)`.
    pub last_hook_idle: bool,
    pub last_hook_at: Option<DateTime<Utc>>,
    /// When the companion last typed into the session (`session_input`). A message waits for
    /// an idle hook newer than this, so a second one never lands in the turn the first began.
    pub input_sent_at: Option<DateTime<Utc>>,
    /// A launch restored after a restart whose hook port could not be taken back: its hooks
    /// go nowhere, so chat needs a relaunch (`SessionState.hooks_lost`).
    pub hooks_lost: bool,
    /// Session messages typed into the session that no prompt has carried yet, oldest first, at
    /// most [`PASTED_KEPT`]. The next `UserPromptSubmit` ending with one is tagged with its id
    /// (plan 122 PR 4). In memory only; it follows the launch across a `/clear`.
    pub pasted: Vec<(Uuid, String)>,
    /// Where the launch's plan lives in the worktree (`LaunchCommand.plan_path`): what
    /// `plan_mirror` reads at settle points (ADR 0053). None from an older server.
    pub plan_path: Option<String>,
    /// The bundle hash of a draft this launch restored (plan 122 §D5): the plan mirror's
    /// first settle does not send it back as new. In memory only.
    pub restored_hash: Option<String>,
    /// What the launch was for, for a launched session (plan 220 §2).
    pub assignment: Option<super::Assignment>,
}

impl Entry {
    /// The session waits for input, as only a hook can say: its last hook was idle and arrived
    /// after the companion last typed into it.
    pub fn waits_for_input(&self) -> bool {
        match (self.last_hook_idle, self.last_hook_at, self.input_sent_at) {
            (true, Some(hook), Some(typed)) => hook > typed,
            (true, Some(_), None) => true,
            _ => false,
        }
    }

    /// Whether the entry has a session to type into: its pane has not vanished and its session
    /// id is known. A `SessionEnd` hook alone does not end it: a `/clear` sends one and the
    /// same pane goes on under a new session id.
    pub fn is_live(&self) -> bool {
        !self.ended && self.session_id.is_some()
    }
}

/// Identity from a path: worktree → branch → issue (ADR 0006).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Identity {
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub repo: Option<String>,
    pub issue: Option<IssueRef>,
    /// The worktree's `HEAD` file.
    pub head: Option<PathBuf>,
}

pub fn identify(path: &Path) -> Identity {
    let Some(worktree) = gitx::worktree_root(path) else {
        return Identity::default();
    };
    let branch = gitx::current_branch(&worktree);
    let repo = gitx::origin_repo(&worktree).or_else(|| gitx::remote_repo(&worktree, "upstream"));
    let head = gitx::head_file(&worktree);
    let issue = match (&branch, &repo) {
        (Some(b), Some(r)) => IssueRef::number_from_branch(b).map(|n| IssueRef::new(r.clone(), n)),
        _ => None,
    };
    Identity {
        worktree: Some(worktree),
        branch,
        repo,
        issue,
        head,
    }
}

#[derive(Debug, Default)]
pub struct Registry {
    pub companion_id: Uuid,
    pub user: String,
    /// Every slice that adds an `impl Registry` block reads this vector (design D7).
    pub(crate) entries: Vec<Entry>,
    /// The issue of the launch in flight, if any. Discovery adds no entry for it: the launch
    /// arm records the session itself, so the server never sees it as a discovered one first.
    pub(crate) launching: Option<IssueRef>,
    /// The launch in flight, for `assignment_of` before the launcher returns (plan 220 §2).
    pub(crate) pending: Option<super::assignment::PendingLaunch>,
    /// A launch was recorded or moved to a new session id since `launches.json` was written
    /// (`sessions::save_launches_if_changed`).
    pub(crate) launches_changed: bool,
}

impl Registry {
    pub fn new(companion_id: Uuid, user: impl Into<String>) -> Self {
        Self {
            companion_id,
            user: user.into(),
            entries: Vec::new(),
            launching: None,
            pending: None,
            launches_changed: false,
        }
    }

    /// The live entry of a launch, by launch id (`session_input`, the `/clear` move).
    pub fn by_launch(&self, launch_id: Uuid) -> Option<&Entry> {
        self.entries
            .iter()
            .filter(|e| e.launch_id == Some(launch_id))
            .min_by_key(|e| !e.is_live())
    }

    pub(crate) fn find_by_launch(&mut self, launch_id: Uuid) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .filter(|e| e.launch_id == Some(launch_id))
            .min_by_key(|e| !e.is_live())
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
                branch_drift: e.branch_drift.clone(),
                // The cause only travels with the status it explains.
                blocked_by: e.blocked_by.filter(|_| e.status == SessionStatus::Blocked),
                launch_id: e.launch_id,
                hooks_lost: e.hooks_lost,
            })
            .collect()
    }

    pub fn by_session(&self, session_id: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.session_id.as_deref() == Some(session_id))
    }

    /// Count by source, for the `rubato.sessions.discovered` gauge. Every source is present, zero
    /// included: a gauge keeps its last value, so a source that empties must be recorded as 0.
    /// Every source this companion can produce, including its zeros. `TmuxIde` is not one since
    /// ADR 0070: the protocol keeps it for stored data, the gauge does not (plan 147 A6).
    pub fn counts_by_source(&self) -> HashMap<StateSource, usize> {
        let mut m: HashMap<_, _> = StateSource::ALL
            .iter()
            .filter(|s| **s != StateSource::TmuxIde)
            .map(|s| (*s, 0))
            .collect();
        for e in &self.entries {
            *m.entry(e.source).or_insert(0) += 1;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_include_every_source() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        let counts = r.counts_by_source();
        assert_eq!(counts.len(), StateSource::ALL.len() - 1, "{counts:?}");
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
            assignment: None,
        });
        let counts = r.counts_by_source();
        assert_eq!(counts.len(), StateSource::ALL.len() - 1, "{counts:?}");
        assert_eq!(counts[&StateSource::Hook], 1);
        for source in [
            StateSource::Headless,
            StateSource::Scraped,
            StateSource::Launch,
        ] {
            assert_eq!(counts.get(&source), Some(&0), "{source:?}");
        }
        assert_eq!(
            counts.get(&StateSource::TmuxIde),
            None,
            "never produced since ADR 0070"
        );
    }
}
