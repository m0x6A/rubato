//! tmux-ide as a CLI (ADR 0003): `team --json` for reconciliation, `worktree create`,
//! `adopt`, `events --follow --json` as a change trigger, and the version pin.

use crate::tmux::Tmux;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct TmuxIde {
    pub command: String,
    tmux: Tmux,
}

/// `tmux-ide team --json`, the parts we read. Unknown fields are ignored on purpose.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct Fleet {
    #[serde(default)]
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct Project {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default)]
    pub sessions: Vec<FleetSession>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct FleetSession {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub agents: Vec<Agent>,
}

/// One detected agent pane.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Agent {
    #[serde(rename = "paneId")]
    pub pane_id: String,
    #[serde(rename = "windowIndex", default)]
    pub window_index: Option<u32>,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub kind: String,
    /// `working`, `blocked`, `done`, `idle`, `unknown`.
    #[serde(default)]
    pub state: String,
    /// Detection confidence. Hook authority sets `since`; screen matching leaves it null.
    #[serde(default)]
    pub confidence: Option<String>,
    /// Unix epoch seconds of the hook stamp, or null when the state was not hook-derived.
    #[serde(default)]
    pub since: Option<f64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub dir: Option<String>,
}

impl Agent {
    /// Hook authority: tmux-ide's own Claude hooks stamped this pane (A10).
    pub fn has_hook_authority(&self) -> bool {
        self.since.is_some()
    }
}

impl Fleet {
    pub fn parse(text: &str) -> Result<Self> {
        serde_json::from_str(text).context("tmux-ide team --json does not parse")
    }

    pub fn agents(&self) -> impl Iterator<Item = &Agent> {
        self.projects
            .iter()
            .flat_map(|p| p.sessions.iter())
            .flat_map(|s| s.agents.iter())
    }
}

impl TmuxIde {
    pub fn new(command: impl Into<String>, tmux: Tmux) -> Self {
        Self {
            command: command.into(),
            tmux,
        }
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(&self.command);
        self.tmux.apply_env(&mut c);
        c
    }

    fn run(&self, args: &[&str], cwd: Option<&Path>) -> Result<String> {
        let mut c = self.cmd();
        c.args(args);
        if let Some(d) = cwd {
            c.current_dir(d);
        }
        let out = c
            .output()
            .with_context(|| format!("running {} {}", self.command, args.join(" ")))?;
        if !out.status.success() {
            bail!(
                "{} {} failed: {}",
                self.command,
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// `tmux-ide --version` → `2.9.18`.
    pub fn version(&self) -> Result<String> {
        let out = self.run(&["--version"], None)?;
        let v = out.split_whitespace().find(|w| {
            w.starts_with('v') && w[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
                || w.chars().next().is_some_and(|c| c.is_ascii_digit())
        });
        v.map(|s| s.trim_start_matches('v').to_owned())
            .context("no version in tmux-ide --version output")
    }

    pub fn team(&self) -> Result<Fleet> {
        Fleet::parse(&self.run(&["team", "--json"], None)?)
    }

    /// `tmux-ide worktree create <branch> --from <base> --no-session` in `repo`; returns the worktree path.
    pub fn worktree_create(&self, repo: &Path, branch: &str, base_ref: &str) -> Result<PathBuf> {
        let out = self.run(
            &[
                "worktree",
                "create",
                branch,
                "--from",
                base_ref,
                "--no-session",
            ],
            Some(repo),
        )?;
        if let Some(p) = out
            .lines()
            .find_map(|l| l.trim().strip_prefix("Worktree ready:"))
        {
            return Ok(PathBuf::from(p.trim()));
        }
        Ok(default_worktree_path(repo, branch))
    }

    pub fn adopt(&self, session: &str) -> Result<()> {
        self.run(&["adopt", session], None).map(|_| ())
    }

    /// Spawn `tmux-ide events --follow --json`; each stdout line is a change trigger.
    pub fn events_follow(&self) -> Result<std::process::Child> {
        let mut c = self.cmd();
        c.args(["events", "--follow", "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        c.spawn().context("spawning tmux-ide events --follow")
    }
}

/// tmux-ide's default: a sibling directory `<repo>-worktrees/<branch>` (slashes kept as directories).
pub fn default_worktree_path(repo: &Path, branch: &str) -> PathBuf {
    let name = repo
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let parent = repo.parent().unwrap_or(repo);
    parent.join(format!("{name}-worktrees")).join(branch)
}

/// Compare dotted versions.
pub fn version_at_least(have: &str, min: &str) -> bool {
    let parse = |s: &str| {
        s.split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect::<Vec<_>>()
    };
    parse(have) >= parse(min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_recorded_fleet() {
        let text = include_str!("../features/fixtures/team.json");
        let f = Fleet::parse(text).unwrap();
        let agents: Vec<&Agent> = f.agents().collect();
        assert!(agents.len() >= 2);
        assert!(agents.iter().any(|a| !a.has_hook_authority()));
        assert!(agents.iter().any(|a| a.has_hook_authority()));
    }

    #[test]
    fn versions() {
        assert!(version_at_least("2.9.18", "2.9.0"));
        assert!(!version_at_least("2.8.30", "2.9.0"));
        assert!(version_at_least("3.0.0", "2.9.0"));
    }

    #[test]
    fn worktree_path_is_sibling() {
        assert_eq!(
            default_worktree_path(Path::new("/w/pantry"), "issue/7-restock"),
            PathBuf::from("/w/pantry-worktrees/issue/7-restock")
        );
    }
}
