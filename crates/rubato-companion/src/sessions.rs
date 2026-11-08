//! The session registry: everything the companion knows about sessions on this machine, from
//! launches, hooks, pane metadata and tmux-ide, merged with a fixed precedence
//! (Hook > TmuxIde > Launch > Scraped). Never rendered as *working* without a trusted signal.

use crate::gitx;
use crate::tmux::Pane;
use crate::tmux_ide::Fleet;
use chrono::{DateTime, TimeZone, Utc};
use rubato_protocol::{
    HookEvent, IssueRef, LaunchCommand, SessionState, SessionStatus, StaleSession, StateSource,
    TmuxLocation,
};
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

#[derive(Debug, Default)]
pub struct Registry {
    pub companion_id: Uuid,
    pub user: String,
    entries: Vec<Entry>,
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

    fn find_by_session(&mut self, session_id: &str) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|e| e.session_id.as_deref() == Some(session_id))
    }

    fn find_by_pane(&mut self, pane_id: &str) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|e| e.pane_id.as_deref() == Some(pane_id))
    }

    pub fn live_for_issue(&self, issue: &IssueRef) -> Option<&Entry> {
        self.entries.iter().find(|e| {
            !e.ended && e.status != SessionStatus::Ended && e.issue.as_ref() == Some(issue)
        })
    }

    /// A launch we performed: identity is known before the first hook.
    pub fn apply_launch(
        &mut self,
        cmd: &LaunchCommand,
        pane_id: &str,
        tmux_session: &str,
        worktree: &Path,
        now: DateTime<Utc>,
    ) {
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

    /// A hook event. Returns the entry's issue so the caller can report per issue.
    pub fn apply_hook(&mut self, ev: &HookEvent) -> Option<IssueRef> {
        let implied = ev.implied_status();
        let now = ev.received_at;
        if let Some(e) = self.find_by_session(&ev.session_id) {
            if let Some(s) = implied {
                e.status = s;
            }
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
        };
        self.entries.push(entry);
        id.issue
    }

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

    pub fn set_spend(&mut self, session_id: &str, usd: f64) {
        if let Some(e) = self.find_by_session(session_id) {
            e.spend_usd = Some(usd);
        }
    }

    /// *Working* sessions with a trusted source and no signal for longer than `stale_secs`. Reported once each.
    pub fn stale(&mut self, now: DateTime<Utc>, stale_secs: u64) -> Vec<StaleSession> {
        let mut out = Vec::new();
        for e in &mut self.entries {
            if e.ended
                || e.stale_reported
                || e.status != SessionStatus::Working
                || !e.source.is_trusted()
            {
                continue;
            }
            let silent = now.signed_duration_since(e.last_seen).num_seconds();
            if silent > stale_secs as i64 {
                e.stale_reported = true;
                out.push(StaleSession {
                    companion_id: self.companion_id,
                    session_id: e.session_id.clone(),
                    issue: e.issue.clone(),
                    last_seen: e.last_seen,
                    silent_for_secs: silent as u64,
                    at: now,
                });
            }
        }
        out
    }

    /// Drop entries that ended more than a cycle ago.
    pub fn sweep(&mut self, now: DateTime<Utc>, keep_ended_secs: i64) {
        self.entries.retain(|e| {
            !(e.ended && now.signed_duration_since(e.last_seen).num_seconds() > keep_ended_secs)
        });
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

    /// Count by status, for the `rubato.sessions.by_state` gauge.
    pub fn counts(&self) -> HashMap<SessionStatus, usize> {
        let mut m = HashMap::new();
        for e in &self.entries {
            *m.entry(e.status).or_insert(0) += 1;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rubato_protocol::HookEventKind;
    use serde_json::json;

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

    #[test]
    fn stale_reports_once_and_only_trusted_working() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        r.apply_hook(&hook("s1", "PreToolUse", "/x", now()));
        assert!(
            r.stale(now() + chrono::Duration::seconds(10), 120)
                .is_empty()
        );
        let late = now() + chrono::Duration::seconds(200);
        let s = r.stale(late, 120);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].silent_for_secs, 200);
        assert!(r.stale(late, 120).is_empty(), "reported once");
        r.apply_hook(&hook("s1", "PostToolUse", "/x", late));
        assert!(
            r.stale(late + chrono::Duration::seconds(300), 120).len() == 1,
            "reset after a new hook"
        );
    }

    #[test]
    fn fleet_without_authority_is_scraped_and_unknown() {
        let mut r = Registry::new(Uuid::nil(), "m0x6A");
        let f = Fleet::parse(include_str!("../features/fixtures/team.json")).unwrap();
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
