//! tmux as a CLI, honouring a private socket directory. Pane *metadata* only (cwd, command,
//! ids); pane content is never read. State comes from hooks and tmux-ide.

use crate::kernel::proc;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Tmux {
    pub command: String,
    pub tmpdir: Option<PathBuf>,
    /// Every call; [`proc::LOCAL_TIMEOUT`] unless a test shortens it.
    pub timeout: std::time::Duration,
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
            timeout: proc::LOCAL_TIMEOUT,
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
        let mut c = self.cmd();
        c.args(args);
        let out = proc::output(
            c,
            &format!("{} {}", self.command, args.join(" ")),
            self.timeout,
        )?;
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

    /// `Err` only on a timeout: a wedged server is not "no server" (plan 40 N7), or every
    /// session would be reported as ended.
    pub fn server_running(&self) -> Result<bool> {
        self.succeeds(&["list-sessions"])
    }

    /// Exit status only. A command that cannot start counts as failed; a timeout is `Err`.
    fn succeeds(&self, args: &[&str]) -> Result<bool> {
        let mut c = self.cmd();
        c.args(args);
        match proc::output(
            c,
            &format!("{} {}", self.command, args.join(" ")),
            self.timeout,
        ) {
            Ok(o) => Ok(o.status.success()),
            Err(e) if proc::is_timeout(&e) => Err(e),
            Err(_) => Ok(false),
        }
    }

    /// Every pane on the server. Empty when no server is running; `Err` when tmux hangs.
    pub fn panes(&self) -> Result<Vec<Pane>> {
        if !self.server_running()? {
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

    pub fn has_session(&self, name: &str) -> Result<bool> {
        self.succeeds(&["has-session", "-t", &format!("={name}")])
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
        let _ = self.succeeds(&["kill-server"]);
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
    use super::*;
    use std::time::Duration;

    /// A stand-in `tmux` running `body`, written by a child `sh` (see `launch::arm::tests`).
    fn standin(dir: &Path, body: &str) -> Tmux {
        let script = dir.join("tmux");
        let ok = Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg(format!("#!/bin/sh\n{body}\n"))
            .arg(&script)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        Tmux {
            timeout: Duration::from_millis(300),
            ..Tmux::new(script.to_string_lossy(), None)
        }
    }

    /// Plan 40 N7: a hung tmux is an error, so reconcile keeps its entries; no server is empty.
    #[test]
    fn a_hung_tmux_is_an_error_not_an_empty_server() {
        let dir = tempfile::tempdir().unwrap();
        let hung = standin(dir.path(), "sleep 5");
        let e = hung.panes().unwrap_err();
        assert!(proc::is_timeout(&e), "{e:#}");
        assert!(hung.has_session("x").is_err());

        let dir = tempfile::tempdir().unwrap();
        let none = standin(dir.path(), "exit 1");
        assert_eq!(none.panes().unwrap(), vec![]);
        assert!(!none.has_session("x").unwrap());
    }

    #[test]
    fn session_names_are_tmux_safe() {
        assert_eq!(
            super::session_name("acme/pantry", "issue/7-restock"),
            "pantry@issue-7-restock"
        );
        assert_eq!(super::session_name("acme/my.repo", "a:b"), "my-repo@a-b");
    }
}
