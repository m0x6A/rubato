//! `launches.json`: the sessions this companion launched, kept across its own restarts (plan
//! 122 §D1). Written atomically with mode 0600 when a launch is recorded or moves to a new
//! session id; read once at start. A restored launch keeps its launch id, so session messages
//! still reach it, and says `hooks_lost` when the hook port could not be taken back.

use super::{Entry, Registry};
use crate::kernel::config::write_atomic_private;
use crate::kernel::tmux::Pane;
use chrono::{DateTime, Utc};
use rubato_protocol::{IssueRef, SessionStatus, StateSource};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// One launched session as `launches.json` keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchRecord {
    pub launch_id: Uuid,
    pub session_id: String,
    pub worktree: PathBuf,
    #[serde(default)]
    pub pane_id: Option<String>,
    #[serde(default)]
    pub tmux_session: Option<String>,
    pub issue: IssueRef,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub spend_cap_usd: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LaunchesFile {
    launches: Vec<LaunchRecord>,
}

pub fn launches_path(dir: &Path) -> PathBuf {
    dir.join("launches.json")
}

/// The launches the last run kept. Empty when there is no file; a file that does not parse is
/// logged and read as empty, so a bad file never stops the companion.
pub fn load_launches(dir: &Path) -> Vec<LaunchRecord> {
    let p = launches_path(dir);
    let Ok(bytes) = std::fs::read(&p) else {
        return Vec::new();
    };
    match serde_json::from_slice::<LaunchesFile>(&bytes) {
        Ok(f) => f.launches,
        Err(e) => {
            tracing::warn!(path = %p.display(), error = %e, "launches file does not parse; none restored");
            Vec::new()
        }
    }
}

/// Write `launches.json` when a launch changed since the last write. Called with the registry
/// locked, so two writers never race; launches and moves are rare.
pub fn save_launches_if_changed(reg: &mut Registry, dir: &Path) {
    if !reg.launches_changed {
        return;
    }
    let file = LaunchesFile {
        launches: reg.launch_records(),
    };
    let written = serde_json::to_vec_pretty(&file)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| write_atomic_private(&launches_path(dir), &bytes));
    match written {
        Ok(()) => reg.launches_changed = false,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), launches = file.launches.len(), "launches file not written")
        }
    }
}

impl Registry {
    /// The launched sessions worth keeping: launched here, not ended, with a session id.
    pub fn launch_records(&self) -> Vec<LaunchRecord> {
        self.entries
            .iter()
            .filter(|e| !e.ended)
            .filter_map(|e| {
                Some(LaunchRecord {
                    launch_id: e.launch_id?,
                    session_id: e.session_id.clone()?,
                    worktree: e.worktree.clone().unwrap_or_else(|| e.cwd.clone()),
                    pane_id: e.pane_id.clone(),
                    tmux_session: e.tmux_session.clone(),
                    issue: e.issue.clone()?,
                    branch: e.branch.clone(),
                    spend_cap_usd: e.spend_cap_usd,
                })
            })
            .collect()
    }

    /// Put back the launches a previous run kept. With `panes` (tmux answered), a launch comes
    /// back only when its pane still runs in its tmux session: a tmux server that restarted
    /// reuses pane ids, and a message must never reach someone else's pane. `hooks_lost` marks
    /// them when the hook port changed. Returns how many came back; a launch whose pane is gone
    /// marks the file for rewriting.
    pub fn restore_launches(
        &mut self,
        records: Vec<LaunchRecord>,
        panes: Option<&[Pane]>,
        hooks_lost: bool,
        now: DateTime<Utc>,
    ) -> usize {
        let mut restored = 0;
        for r in records {
            if self
                .entries
                .iter()
                .any(|e| e.launch_id == Some(r.launch_id))
            {
                continue;
            }
            if let Some(panes) = panes {
                let alive = panes.iter().any(|p| {
                    r.pane_id.as_deref().is_none_or(|id| id == p.pane_id)
                        && r.tmux_session.as_deref().is_none_or(|s| s == p.session)
                });
                if !alive {
                    tracing::info!(launch_id = %r.launch_id, pane = ?r.pane_id, "launch not restored: its pane is gone");
                    // Rewritten at the next save, so it does not come back at every start.
                    self.launches_changed = true;
                    continue;
                }
            }
            self.entries.push(Entry {
                session_id: Some(r.session_id),
                pane_id: r.pane_id,
                tmux_session: r.tmux_session,
                window: Some(0),
                cwd: r.worktree.clone(),
                worktree: Some(r.worktree),
                branch: r.branch,
                repo: Some(r.issue.repo.clone()),
                issue: Some(r.issue),
                // Nothing proves what it does until a hook or tmux-ide says.
                status: SessionStatus::Unknown,
                source: StateSource::Launch,
                last_seen: now,
                spend_usd: None,
                transcript_path: None,
                launch_id: Some(r.launch_id),
                spend_cap_usd: r.spend_cap_usd,
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
                hooks_lost,
            });
            restored += 1;
        }
        restored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap()
    }

    fn record(n: u128, pane: &str) -> LaunchRecord {
        LaunchRecord {
            launch_id: Uuid::from_u128(n),
            session_id: format!("s{n}"),
            worktree: PathBuf::from(format!("/w/{n}")),
            pane_id: Some(pane.into()),
            tmux_session: Some(format!("pantry@issue-{n}")),
            issue: IssueRef::new("acme/pantry", n as u64),
            branch: Some(format!("issue/{n}-x")),
            spend_cap_usd: Some(20.0),
        }
    }

    fn pane(id: &str, session: &str) -> Pane {
        Pane {
            session: session.into(),
            window: 0,
            pane_id: id.into(),
            cwd: PathBuf::from("/w"),
            current_command: "claude".into(),
            pid: 1,
        }
    }

    #[test]
    fn launches_round_trip_through_the_file_with_private_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.restore_launches(vec![record(7, "%7"), record(8, "%8")], None, false, now());
        // Nothing changed since the start: nothing written.
        save_launches_if_changed(&mut r, dir.path());
        assert!(!launches_path(dir.path()).exists());
        r.launches_changed = true;
        save_launches_if_changed(&mut r, dir.path());
        assert!(!r.launches_changed);
        let mode = std::fs::metadata(launches_path(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(
            load_launches(dir.path()),
            vec![record(7, "%7"), record(8, "%8")]
        );
    }

    #[test]
    fn only_launches_whose_pane_still_runs_in_their_session_come_back() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        let panes = [
            pane("%7", "pantry@issue-7"),
            // Pane id 8 was reused by another session after a tmux restart.
            pane("%8", "someone@else"),
        ];
        let n = r.restore_launches(
            vec![record(7, "%7"), record(8, "%8"), record(9, "%9")],
            Some(&panes),
            true,
            now(),
        );
        assert_eq!(n, 1);
        assert!(
            r.launches_changed,
            "the gone ones are dropped from the file at the next save"
        );
        let e = r.by_launch(Uuid::from_u128(7)).unwrap();
        assert!(e.hooks_lost);
        assert_eq!(e.source, StateSource::Launch);
        assert!(!e.waits_for_input(), "no hook yet, so not idle");
        assert!(r.by_launch(Uuid::from_u128(8)).is_none());
        let snap = r.snapshot();
        assert_eq!(snap[0].launch_id, Some(Uuid::from_u128(7)));
        assert!(snap[0].hooks_lost);
        // A second restore of the same launch adds nothing.
        assert_eq!(
            r.restore_launches(vec![record(7, "%7")], None, false, now()),
            0
        );
    }

    #[test]
    fn a_bad_file_restores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_launches(dir.path()).is_empty());
        std::fs::write(launches_path(dir.path()), "{\"launches\": 7}").unwrap();
        assert!(load_launches(dir.path()).is_empty());
    }
}
