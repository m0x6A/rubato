//! tmux as a CLI, honouring a private socket directory. Pane *metadata* only (cwd, command,
//! ids); pane content is never read. State comes from hooks and tmux-ide. The one write into a
//! pane is [`SessionInput::paste`] (ADR 0051, amending ADR 0003 for typed input).

use crate::kernel::proc;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The port that types text into a session (plan 122 §D1). Its first adapter is [`Tmux`]; a
/// second (a headless turn, plan 122 PR 11) can come later. Blocking: call from
/// `spawn_blocking`.
pub trait SessionInput: Send + Sync {
    /// Type `text` into `target` (a tmux pane id or session name) and press Enter, exactly
    /// once. `key` names the transient buffer, so two pastes never share one.
    fn paste(&self, target: &str, key: &str, text: &str) -> Result<()>;
}

impl SessionInput for Tmux {
    /// `load-buffer` from stdin (the text never touches argv or a file), `paste-buffer -p -d`
    /// (bracketed paste when the program asked for it; the buffer is deleted after), then
    /// `send-keys Enter`. The spike in plan 122 showed the prompt arrives byte for byte.
    fn paste(&self, target: &str, key: &str, text: &str) -> Result<()> {
        let buffer = format!("rubato-{key}");
        self.run_input(
            &["load-buffer", "-b", &buffer, "-"],
            text.as_bytes().to_vec(),
        )
        .context("tmux load-buffer")?;
        if let Err(e) = self.run(&["paste-buffer", "-p", "-d", "-b", &buffer, "-t", target]) {
            // Nothing was typed; leave no buffer behind.
            let _ = self.run(&["delete-buffer", "-b", &buffer]);
            return Err(e.context("tmux paste-buffer"));
        }
        self.run(&["send-keys", "-t", target, "Enter"])
            .context("tmux send-keys Enter")?;
        Ok(())
    }
}

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
        Self::stdout_of(out, &self.command, args)
    }

    /// As `run`, with `input` on stdin.
    fn run_input(&self, args: &[&str], input: Vec<u8>) -> Result<String> {
        let mut c = self.cmd();
        c.args(args);
        let out = proc::output_with_stdin(
            c,
            &format!("{} {}", self.command, args.join(" ")),
            self.timeout,
            input,
        )?;
        Self::stdout_of(out, &self.command, args)
    }

    fn stdout_of(out: std::process::Output, command: &str, args: &[&str]) -> Result<String> {
        if !out.status.success() {
            bail!(
                "{} {} failed: {}",
                command,
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

    /// The paste on a real, private tmux server: the pane's program reads exactly the text, its
    /// lines intact, then the Enter. No buffer is left behind.
    #[test]
    fn paste_types_the_text_once_into_a_real_pane() {
        // Unix socket paths are short-limited: keep the socket dir under /tmp.
        let dir = tempfile::Builder::new()
            .prefix("rubato-paste-")
            .tempdir_in("/tmp")
            .unwrap();
        let tmux = Tmux::new("tmux", Some(dir.path().to_path_buf()));
        let typed = dir.path().join("typed");
        let pane = tmux
            .new_session("paste", dir.path(), &format!("cat > '{}'", typed.display()))
            .unwrap();
        let text = "line one\nline two\nline three, with 'quotes' and \"more\"";
        let pasted = tmux.paste(&pane, "k1", text);
        let mut got = String::new();
        for _ in 0..100 {
            got = std::fs::read_to_string(&typed).unwrap_or_default();
            if got.lines().count() >= 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let buffers = tmux.run(&["list-buffers", "-F", "#{buffer_name}"]);
        tmux.kill_server();
        pasted.unwrap();
        assert_eq!(got, format!("{text}\n"));
        assert!(
            !buffers.unwrap_or_default().contains("rubato-k1"),
            "the buffer is deleted after the paste"
        );
    }

    /// A paste that fails names its step, types nothing more, and deletes its buffer. The
    /// stand-in records each call and keeps what `load-buffer` read from stdin.
    #[test]
    fn a_failed_paste_says_which_step_and_deletes_its_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let calls = dir.path().join("calls");
        let stdin = dir.path().join("stdin");
        let tmux = standin(
            dir.path(),
            &format!(
                "echo \"$*\" >> '{}'\ncase \"$1\" in\n  load-buffer) cat > '{}' ;;\n  paste-buffer) echo 'no such pane' >&2; exit 1 ;;\nesac",
                calls.display(),
                stdin.display()
            ),
        );
        let e = tmux.paste("%9", "k2", "hello\nworld").unwrap_err();
        assert!(format!("{e:#}").contains("tmux paste-buffer"), "{e:#}");
        assert!(format!("{e:#}").contains("no such pane"), "{e:#}");
        assert_eq!(std::fs::read_to_string(&stdin).unwrap(), "hello\nworld");
        let calls = std::fs::read_to_string(&calls).unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            [
                "load-buffer -b rubato-k2 -",
                "paste-buffer -p -d -b rubato-k2 -t %9",
                "delete-buffer -b rubato-k2",
            ],
            "no Enter after a failed paste"
        );
    }

    /// Every subcommand that reads what a pane shows. Spelled in parts so this file does not
    /// match its own scan.
    fn pane_reads() -> Vec<String> {
        [["capture", "pane"], ["pipe", "pane"]]
            .iter()
            .flat_map(|[a, b]| {
                // The long names, and the aliases tmux also takes: the first word and one letter.
                [format!("{a}-{b}"), format!("{a}{}", &b[..1])]
            })
            .collect()
    }

    /// The (file, needle) pairs in `files` that read a pane.
    fn scan(files: &[(String, String)]) -> Vec<(String, String)> {
        let needles = pane_reads();
        files
            .iter()
            .flat_map(|(path, text)| {
                needles
                    .iter()
                    .filter(|n| text.contains(n.as_str()))
                    .map(|n| (path.clone(), n.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn rust_files(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((
                    path.display().to_string(),
                    std::fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }

    /// AGENTS.md and ADR 0003: the companion writes into a pane and never reads one; replies
    /// come from hooks (plan 122, "What it must not break").
    #[test]
    fn the_companion_never_reads_a_pane() {
        let mut files = Vec::new();
        rust_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        assert!(files.len() > 30, "scanned only {} files", files.len());
        let hits = scan(&files);
        assert!(hits.is_empty(), "pane reads in the companion: {hits:?}");

        // Must fail: the same scan over fixtures that read a pane reports each one.
        let mut fixtures = Vec::new();
        rust_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pane_reads"),
            &mut fixtures,
        );
        let caught = scan(&fixtures);
        assert_eq!(fixtures.len(), 2, "{fixtures:?}");
        for (path, _) in &fixtures {
            assert!(
                caught.iter().any(|(p, _)| p == path),
                "{path} reads a pane and was not caught"
            );
        }
        println!(
            "scanned {} files for {} pane reads: none; {} must-fail fixtures caught",
            files.len(),
            pane_reads().len(),
            fixtures.len()
        );
    }
}
