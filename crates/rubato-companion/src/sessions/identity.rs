//! Identity read again when a session's checkout changes (design 80 §5). The checks are taken
//! under the registry lock, run off it (git calls, `spawn_blocking`), and applied under it
//! again, so the hook path never waits on git.

use super::{Identity, Registry, identify};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Which entry, by pane or by session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Pane(String),
    Session(String),
}

#[derive(Debug, Clone)]
pub struct Check {
    pub key: Key,
    pub cwd: PathBuf,
    pub worktree: Option<PathBuf>,
    pub head: Option<PathBuf>,
    pub head_mtime: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct Rechecked {
    pub key: Key,
    pub id: Identity,
    pub head_mtime: Option<SystemTime>,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Off the lock: identity again for every entry whose cwd left its worktree or whose `HEAD`
/// changed since it was read. An entry never read (no `head`) is read once. A stat per entry;
/// git only for those that changed.
pub fn recheck(checks: Vec<Check>) -> Vec<Rechecked> {
    checks
        .into_iter()
        .filter_map(|c| {
            let left = c.worktree.as_ref().is_none_or(|w| !c.cwd.starts_with(w));
            let now = c.head.as_deref().and_then(mtime);
            let moved = c.head.is_none() || now != c.head_mtime;
            if !(left || moved) {
                return None;
            }
            let id = identify(&c.cwd);
            let head_mtime = id.head.as_deref().and_then(mtime);
            Some(Rechecked {
                key: c.key,
                id,
                head_mtime,
            })
        })
        .collect()
}

impl Registry {
    /// What [`recheck`] needs, for every live entry.
    pub fn identity_checks(&self) -> Vec<Check> {
        self.entries
            .iter()
            .filter(|e| !e.ended)
            .filter_map(|e| {
                let key = match (&e.pane_id, &e.session_id) {
                    (Some(p), _) => Key::Pane(p.clone()),
                    (None, Some(s)) => Key::Session(s.clone()),
                    (None, None) => return None,
                };
                Some(Check {
                    key,
                    cwd: e.cwd.clone(),
                    worktree: e.worktree.clone(),
                    head: e.head.clone(),
                    head_mtime: e.head_mtime,
                })
            })
            .collect()
    }

    /// Apply what [`recheck`] found. A launched session keeps its launch issue (its spend cap,
    /// its `CapHit` and its telemetry attributes are that issue's) and reports the branch it
    /// moved to as drift; any other session follows its branch.
    pub fn apply_identities(&mut self, found: Vec<Rechecked>) {
        for r in found {
            let Some(e) = self.entries.iter_mut().find(|e| match &r.key {
                Key::Pane(p) => e.pane_id.as_deref() == Some(p),
                Key::Session(s) => e.pane_id.is_none() && e.session_id.as_deref() == Some(s),
            }) else {
                continue;
            };
            e.head = r.id.head.clone();
            e.head_mtime = r.head_mtime;
            if e.launch_id.is_some() && e.issue.is_some() {
                let drift = r.id.branch.clone().filter(|b| Some(b) != e.branch.as_ref());
                if drift != e.branch_drift {
                    tracing::info!(
                        rubato.issue = ?e.issue.as_ref().map(ToString::to_string),
                        launch_branch = ?e.branch,
                        now_on = ?drift,
                        "branch drift"
                    );
                    e.branch_drift = drift;
                }
                continue;
            }
            if (e.branch.as_ref(), e.issue.as_ref(), e.repo.as_ref())
                != (
                    r.id.branch.as_ref(),
                    r.id.issue.as_ref(),
                    r.id.repo.as_ref(),
                )
            {
                tracing::info!(
                    old_branch = ?e.branch,
                    new_branch = ?r.id.branch,
                    rubato.issue = ?r.id.issue.as_ref().map(ToString::to_string),
                    "identity changed"
                );
                e.worktree = r.id.worktree;
                e.branch = r.id.branch;
                e.repo = r.id.repo;
                e.issue = r.id.issue;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::Entry;
    use chrono::Utc;
    use rubato_protocol::{IssueRef, SessionStatus, StateSource};
    use std::process::Command;
    use uuid::Uuid;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        git(d.path(), &["init", "-q", "-b", "main"]);
        git(
            d.path(),
            &["remote", "add", "origin", "git@github.com:acme/pantry.git"],
        );
        git(d.path(), &["commit", "-q", "--allow-empty", "-m", "init"]);
        d
    }

    fn entry(cwd: &Path, launch: bool) -> Entry {
        let id = identify(cwd);
        Entry {
            session_id: None,
            pane_id: Some("%1".into()),
            tmux_session: None,
            window: None,
            cwd: cwd.to_owned(),
            worktree: id.worktree,
            branch: id.branch,
            repo: id.repo,
            issue: id.issue,
            status: SessionStatus::Unknown,
            source: StateSource::Scraped,
            last_seen: Utc::now(),
            spend_usd: None,
            transcript_path: None,
            launch_id: launch.then(Uuid::nil),
            spend_cap_usd: None,
            stale_reported: false,
            ended: false,
            head: None,
            head_mtime: None,
            branch_drift: None,
            blocked_by: None,
        }
    }

    fn cycle(r: &mut Registry) -> usize {
        let found = recheck(r.identity_checks());
        let n = found.len();
        r.apply_identities(found);
        n
    }

    #[test]
    fn a_checkout_moves_a_session_onto_its_issue_and_an_unchanged_head_costs_no_git() {
        let d = repo();
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.entries.push(entry(d.path(), false));
        assert_eq!(r.entries[0].issue, None);
        assert_eq!(cycle(&mut r), 1, "first read records the HEAD file");
        assert_eq!(cycle(&mut r), 0, "unchanged HEAD: nothing read again");
        // mtime granularity: make sure the checkout lands on a later time.
        std::thread::sleep(std::time::Duration::from_millis(20));
        git(d.path(), &["checkout", "-q", "-b", "issue/42-x"]);
        assert_eq!(cycle(&mut r), 1);
        assert_eq!(r.entries[0].issue, Some(IssueRef::new("acme/pantry", 42)));
        assert_eq!(r.entries[0].branch.as_deref(), Some("issue/42-x"));
    }

    #[test]
    fn a_launched_session_keeps_its_issue_and_reports_drift() {
        let d = repo();
        git(d.path(), &["checkout", "-q", "-b", "issue/7-y"]);
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.entries.push(entry(d.path(), true));
        cycle(&mut r);
        std::thread::sleep(std::time::Duration::from_millis(20));
        git(d.path(), &["checkout", "-q", "main"]);
        cycle(&mut r);
        let e = &r.entries[0];
        assert_eq!(e.issue, Some(IssueRef::new("acme/pantry", 7)));
        assert_eq!(e.branch_drift.as_deref(), Some("main"));
        assert_eq!(r.snapshot()[0].branch_drift.as_deref(), Some("main"));
    }

    #[test]
    fn upstream_names_the_repo_when_there_is_no_origin() {
        let d = tempfile::tempdir().unwrap();
        git(d.path(), &["init", "-q", "-b", "issue/3-z"]);
        git(
            d.path(),
            &[
                "remote",
                "add",
                "upstream",
                "https://github.com/acme/pantry.git",
            ],
        );
        git(d.path(), &["commit", "-q", "--allow-empty", "-m", "init"]);
        assert_eq!(
            identify(d.path()).issue,
            Some(IssueRef::new("acme/pantry", 3))
        );
    }
}
