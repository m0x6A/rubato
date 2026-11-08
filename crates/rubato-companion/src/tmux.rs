//! tmux as a CLI, honouring a private socket directory. Pane *metadata* only (cwd, command,
//! ids); pane content is never read. State comes from hooks and tmux-ide.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Tmux {
    pub command: String,
    pub tmpdir: Option<PathBuf>,
}

/// One pane's metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub session: String,
    pub window: u32,
    pub pane_id: String,
    pub cwd: PathBuf,
    pub current_command: String,
    pub pid: u32,
}

impl Tmux {
    pub fn new(command: impl Into<String>, tmpdir: Option<PathBuf>) -> Self {
        Self {
            command: command.into(),
            tmpdir,
        }
    }

    /// A `Command` with the socket env applied. Also used for tmux-ide and the launcher so they see the same server.
    pub fn apply_env(&self, cmd: &mut Command) {
        cmd.env_remove("TMUX");
        if let Some(d) = &self.tmpdir {
            cmd.env("TMUX_TMPDIR", d);
        }
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(&self.command);
        self.apply_env(&mut c);
        c
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let out = self
            .cmd()
            .args(args)
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

    pub fn server_running(&self) -> bool {
        self.cmd()
            .args(["list-sessions"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Every pane on the server. Empty when no server is running.
    pub fn panes(&self) -> Result<Vec<Pane>> {
        if !self.server_running() {
            return Ok(vec![]);
        }
        let out = self.run(&["list-panes", "-a", "-F", "#{session_name}\t#{window_index}\t#{pane_id}\t#{pane_current_path}\t#{pane_current_command}\t#{pane_pid}"])?;
        let mut panes = Vec::new();
        for line in out.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 6 {
                continue;
            }
            panes.push(Pane {
                session: f[0].to_owned(),
                window: f[1].parse().unwrap_or(0),
                pane_id: f[2].to_owned(),
                cwd: PathBuf::from(f[3]),
                current_command: f[4].to_owned(),
                pid: f[5].parse().unwrap_or(0),
            });
        }
        Ok(panes)
    }

    pub fn has_session(&self, name: &str) -> bool {
        self.cmd()
            .args(["has-session", "-t", &format!("={name}")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Start a detached session running `command` in `cwd`; returns the pane id.
    pub fn new_session(&self, name: &str, cwd: &Path, command: &str) -> Result<String> {
        let out = self.run(&[
            "new-session",
            "-d",
            "-s",
            name,
            "-c",
            &cwd.to_string_lossy(),
            "-P",
            "-F",
            "#{pane_id}",
            command,
        ])?;
        Ok(out.trim().to_owned())
    }

    pub fn kill_session(&self, name: &str) -> Result<()> {
        self.run(&["kill-session", "-t", &format!("={name}")])
            .map(|_| ())
    }

    pub fn kill_server(&self) {
        let _ = self.cmd().args(["kill-server"]).output();
    }

    /// A pane option such as `@agent_session_id`, if set.
    pub fn pane_option(&self, pane_id: &str, name: &str) -> Option<String> {
        let out = self
            .cmd()
            .args(["show-options", "-p", "-v", "-t", pane_id, name])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let v = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if v.is_empty() { None } else { Some(v) }
    }
}

/// tmux session names may not contain `:` or `.`; `/` would also confuse targets.
pub fn session_name(repo: &str, branch: &str) -> String {
    let repo_name = repo.rsplit('/').next().unwrap_or(repo);
    let clean = |s: &str| {
        s.chars()
            .map(|c| {
                if matches!(c, ':' | '.' | '/' | ' ') {
                    '-'
                } else {
                    c
                }
            })
            .collect::<String>()
    };
    format!("{}@{}", clean(repo_name), clean(branch))
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_names_are_tmux_safe() {
        assert_eq!(
            super::session_name("acme/pantry", "issue/7-restock"),
            "pantry@issue-7-restock"
        );
        assert_eq!(super::session_name("acme/my.repo", "a:b"), "my-repo@a-b");
    }
}
