//! Claude Code as a headless CLI, for read-only fork turns (ADR 0057, plan 122 §D8). The first
//! turn of a room agent forks the live session: `claude -p --resume <session> --fork-session
//! --permission-mode plan --output-format stream-json --verbose`; later turns `--resume` the fork
//! without `--fork-session`. The spike showed a fork leaves the live session's transcript alone
//! and fires no hooks of its own beyond the ones it is given.
//!
//! No Rubato hooks: the launch's `hooks.json` reaches a session only through `--settings` in
//! `launch.sh`, which a fork never gets. The fork's settings say `disableAllHooks`, so hooks a
//! developer installed by hand do not fire either, and the `RUBATO_*` variables `launch.sh`
//! exports are removed. The turn's text goes on stdin, so its length meets no argument limit.
//! Blocking: call from `spawn_blocking`.

use crate::kernel::proc;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// The fork's own settings: no hooks at all.
pub const NO_HOOKS: &str = r#"{"disableAllHooks":true}"#;

/// What `launch.sh` exports for a launched session; a fork carries none of it.
pub const LAUNCH_ENV: &[&str] = &[
    "RUBATO_LAUNCH_ID",
    "RUBATO_HOOK_URL",
    "RUBATO_HOOK_TOKEN",
    "RUBATO_PLANNING",
];

#[derive(Debug, Clone)]
pub struct Claude {
    /// `companion.claude_command`, run through `sh` as `launch.sh` runs it.
    pub command: String,
    pub timeout: Duration,
}

/// What one headless turn gave back.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// The final `result` text; empty when the turn failed.
    pub text: String,
    /// The session the turn ran in: the fork's, for the next `--resume`.
    pub session_id: Option<String>,
    pub cost_usd: Option<f64>,
    pub exit_code: Option<i32>,
    /// Why the turn has no answer, when it has none.
    pub error: Option<String>,
}

/// The arguments of one turn: `fork` makes a new fork of `resume`; otherwise `resume` is the
/// fork itself.
pub fn turn_args(resume: &str, fork: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["-p".into(), "--resume".into(), resume.into()];
    if fork {
        a.push("--fork-session".into());
    }
    a.extend(
        [
            "--permission-mode",
            "plan",
            "--output-format",
            "stream-json",
            "--verbose",
            "--settings",
            NO_HOOKS,
        ]
        .map(String::from),
    );
    a
}

/// The last `result` event of a stream-json run: its text, session and cost. A session id from
/// the `init` event stands in when the result has none.
pub fn parse_stream(stdout: &str, exit_code: Option<i32>, stderr: &str) -> Turn {
    let mut session_id = None;
    let mut result = None;
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        let sid = v
            .get("session_id")
            .and_then(|s| s.as_str())
            .map(str::to_owned);
        match v.get("type").and_then(|t| t.as_str()) {
            Some("system") if session_id.is_none() => session_id = sid,
            Some("result") => {
                if sid.is_some() {
                    session_id = sid;
                }
                result = Some(v);
            }
            _ => {}
        }
    }
    let Some(r) = result else {
        let err = stderr.trim();
        let start = err.char_indices().rev().nth(299).map_or(0, |(i, _)| i);
        let tail = &err[start..];
        return Turn {
            text: String::new(),
            session_id,
            cost_usd: None,
            exit_code,
            error: Some(format!(
                "no result from claude (exit {}): {tail}",
                exit_code.map_or("signal".to_owned(), |c| c.to_string())
            )),
        };
    };
    let text = r
        .get("result")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .to_owned();
    let cost_usd = r.get("total_cost_usd").and_then(|c| c.as_f64());
    let is_error = r.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
    let error = is_error.then(|| {
        let subtype = r.get("subtype").and_then(|s| s.as_str()).unwrap_or("error");
        if text.is_empty() {
            subtype.to_owned()
        } else {
            format!("{subtype}: {text}")
        }
    });
    Turn {
        text: if is_error { String::new() } else { text },
        session_id,
        cost_usd,
        exit_code,
        error,
    }
}

impl Claude {
    /// One headless turn in `worktree`, with `prompt` on stdin. A turn past the deadline is
    /// killed with its process group (`proc`) and is an error.
    pub fn turn(&self, worktree: &Path, resume: &str, fork: bool, prompt: &str) -> Result<Turn> {
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg(format!("{} \"$@\"", self.command))
            .arg("sh")
            .args(turn_args(resume, fork))
            .current_dir(worktree)
            .env_remove("TMUX");
        for v in LAUNCH_ENV {
            c.env_remove(v);
        }
        let out = proc::output_with_stdin(c, "claude -p", self.timeout, prompt.as_bytes().to_vec())
            .context("running the fork turn")?;
        Ok(parse_stream(
            &String::from_utf8_lossy(&out.stdout),
            out.status.code(),
            &String::from_utf8_lossy(&out.stderr),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_turn_forks_and_a_later_one_resumes_the_fork() {
        let first = turn_args("s1", true);
        assert_eq!(
            first[..7],
            [
                "-p",
                "--resume",
                "s1",
                "--fork-session",
                "--permission-mode",
                "plan",
                "--output-format"
            ]
        );
        assert!(first.contains(&"--verbose".to_owned()));
        let later = turn_args("fork-1", false);
        assert!(!later.contains(&"--fork-session".to_owned()));
        assert_eq!(later[..3], ["-p", "--resume", "fork-1"]);
    }

    #[test]
    fn the_result_event_gives_text_session_and_cost() {
        let out = r#"{"type":"system","subtype":"init","session_id":"fork-1"}
{"type":"assistant","message":{"content":[{"type":"text","text":"thinking"}]}}
not json at all
{"type":"result","subtype":"success","is_error":false,"result":"Because b keeps the seam.","session_id":"fork-1","total_cost_usd":0.1}"#;
        let t = parse_stream(out, Some(0), "");
        assert_eq!(t.text, "Because b keeps the seam.");
        assert_eq!(t.session_id.as_deref(), Some("fork-1"));
        assert_eq!(t.cost_usd, Some(0.1));
        assert_eq!(t.error, None);

        let failed = r#"{"type":"system","subtype":"init","session_id":"fork-2"}
{"type":"result","subtype":"error_max_turns","is_error":true,"total_cost_usd":0.05}"#;
        let t = parse_stream(failed, Some(1), "");
        assert_eq!(t.error.as_deref(), Some("error_max_turns"));
        assert_eq!(
            (t.text.as_str(), t.cost_usd),
            ("", Some(0.05)),
            "a failed turn still cost"
        );
        assert_eq!(t.session_id.as_deref(), Some("fork-2"));

        let t = parse_stream("", Some(2), "Error: session s9 not found\n");
        assert!(t.error.unwrap().contains("session s9 not found"));
    }

    /// The real adapter on a stand-in: the prompt arrives on stdin, the launch's variables do
    /// not, and the turn runs in the worktree.
    #[test]
    fn a_turn_reads_its_prompt_from_stdin_without_the_launch_env() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("claude");
        let body = format!(
            "#!/bin/sh\n{{ echo \"ARGS: $*\"; env | grep '^RUBATO_'; echo \"PWD: $PWD\"; cat; }} > '{}'\necho '{{\"type\":\"result\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"f\",\"total_cost_usd\":0.01}}'\n",
            dir.path().join("log").display()
        );
        let ok = Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg(&body)
            .arg(&script)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let claude = Claude {
            command: script.to_string_lossy().into_owned(),
            timeout: Duration::from_secs(10),
        };
        let prompt = "Messages from the plan's room follow.\n\nalice: why b?";
        let t = claude.turn(dir.path(), "s1", true, prompt).unwrap();
        assert_eq!((t.text.as_str(), t.cost_usd), ("ok", Some(0.01)));
        let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
        assert!(log.contains("ARGS: -p --resume s1 --fork-session"), "{log}");
        assert!(!log.lines().any(|l| l.starts_with("RUBATO_")), "{log}");
        assert!(log.ends_with(prompt), "{log}");
        let real = std::fs::canonicalize(dir.path()).unwrap();
        assert!(
            log.contains(&format!("PWD: {}", real.display()))
                || log.contains(&format!("PWD: {}", dir.path().display())),
            "{log}"
        );
    }
}
