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

    /// A launch is about to run without the lock: note its issue so discovery leaves the new pane
    /// to `apply_launch`, and say whether a session is already live for it.
    pub fn begin_launch(&mut self, issue: &IssueRef) -> Option<LiveSession> {
        self.launching = Some(issue.clone());
        self.live_for_issue(issue)
    }

    /// The launch returned, whatever its result: discovery sees the issue's panes again.
    pub fn end_launch(&mut self) {
        self.launching = None;
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
        // Plan 40 S6: the launched session's hook got here first (hooks do not wait for the
        // launch). Keep what the hook said, add what only the launch knows.
        let sid = cmd.session_id.to_string();
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| !e.ended && e.session_id.as_deref() == Some(sid.as_str()))
        {
            e.pane_id = Some(pane_id.to_owned());
            e.tmux_session = Some(tmux_session.to_owned());
            e.window = Some(0);
            e.worktree = Some(worktree.to_path_buf());
            e.branch = Some(cmd.branch.clone());
            e.repo = Some(cmd.issue.repo.clone());
            e.issue = Some(cmd.issue.clone());
            e.launch_id = Some(cmd.id);
            e.spend_cap_usd = Some(cmd.spend_cap_usd);
            // A pane reconcile found meanwhile is the same session: drop the duplicate.
            self.entries.retain(|o| {
                !(o.session_id.is_none() && o.pane_id.as_deref() == Some(pane_id) && !o.ended)
            });
            return;
        }
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
            head: None,
            head_mtime: None,
            branch_drift: None,
            blocked_by: None,
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

    /// Plan 40 S6: hooks no longer wait for the launch, so the launched session's first hook can
    /// be applied before `apply_launch`. One entry for the session id, keeping the hook's state.
    #[test]
    fn a_hook_from_the_launched_session_before_the_launch_is_recorded_is_upgraded() {
        let now = Utc.with_ymd_and_hms(2026, 11, 12, 12, 0, 0).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path();
        let cmd = command();
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        let ev = rubato_protocol::HookEvent::from_payload(
            serde_json::json!({
                "session_id": cmd.session_id.to_string(),
                "hook_event_name": "UserPromptSubmit",
                "cwd": worktree.to_string_lossy(),
                "transcript_path": "/t.jsonl",
            }),
            now,
        )
        .unwrap();
        r.begin_launch(&cmd.issue);
        r.apply_hook(&ev);
        // Reconcile runs before the launch is recorded: it matches the hook-only entry to the new
        // pane by cwd and gives it the pane id.
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
        r.end_launch();
        r.apply_launch(&cmd, "%7", "rubato-27", worktree, now);

        assert!(
            r.entries().iter().all(|e| !e.ended),
            "nothing retired: {:?}",
            r.entries()
        );
        assert_eq!(r.entries().len(), 1, "{:?}", r.entries());
        let sid = cmd.session_id.to_string();
        let same: Vec<&Entry> = r
            .entries()
            .iter()
            .filter(|e| e.session_id.as_deref() == Some(sid.as_str()))
            .collect();
        assert_eq!(same.len(), 1, "one entry for the session id, not two");
        let e = same[0];
        assert_eq!(e.launch_id, Some(cmd.id));
        assert_eq!(e.pane_id.as_deref(), Some("%7"));
        assert_eq!(e.tmux_session.as_deref(), Some("rubato-27"));
        assert_eq!(e.issue, Some(cmd.issue.clone()));
        assert_eq!(e.spend_cap_usd, Some(cmd.spend_cap_usd));
        assert_eq!(
            e.status,
            SessionStatus::Working,
            "the hook's status is kept"
        );
        assert_eq!(e.source, StateSource::Hook);
        assert_eq!(
            e.transcript_path.as_deref(),
            Some(Path::new("/t.jsonl")),
            "the hook's transcript is kept"
        );
    }

    /// A real repository on an issue branch, so `identify` reads the issue from it.
    fn repo_on_issue_branch(dir: &Path, branch: &str) {
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q"]);
        git(&[
            "remote",
            "add",
            "origin",
            "https://github.com/m0x6A/rubato.git",
        ]);
        git(&["checkout", "-q", "-b", branch]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ]);
    }

    #[test]
    fn discovery_leaves_the_pane_of_a_launch_in_flight_alone() {
        let now = Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cmd = command();
        repo_on_issue_branch(dir.path(), &cmd.branch);
        let panes = [Pane {
            session: "rubato-27".into(),
            window: 0,
            pane_id: "%7".into(),
            cwd: dir.path().to_path_buf(),
            current_command: "claude".into(),
            pid: 1,
        }];
        let mut r = Registry::new(Uuid::nil(), "m0x6A");

        assert_eq!(r.begin_launch(&cmd.issue), None);
        r.apply_panes(&panes, now);
        assert!(
            r.snapshot().is_empty(),
            "no discovered session for the issue while its launch runs: {:?}",
            r.snapshot()
        );

        r.end_launch();
        r.apply_launch(&cmd, "%7", "rubato-27", dir.path(), now);
        let snap = r.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].source, StateSource::Launch);
        assert_eq!(snap[0].issue, Some(cmd.issue.clone()));

        // A launch that failed leaves nothing behind: discovery sees the pane again.
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.begin_launch(&cmd.issue);
        r.end_launch();
        r.apply_panes(&panes, now);
        assert_eq!(r.snapshot().len(), 1);
        assert_eq!(r.snapshot()[0].issue, Some(cmd.issue));
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
