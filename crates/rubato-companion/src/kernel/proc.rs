//! Running an external command with a deadline (plan 40 S2). The command runs as the leader of
//! its own process group; when the deadline passes the whole group gets TERM, then KILL after a
//! short grace, so a `sh` wrapper and the children it started go too, and the caller gets
//! [`TimedOut`]. Synchronous on purpose:
//! the adapters are called from blocking code (the launcher runs in `spawn_blocking`).

use anyhow::{Context, Result};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Local git and tmux calls: quick, no network (plan 40 A1).
pub const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);

/// The command did not finish (or did not close its output) before the deadline.
#[derive(Debug, thiserror::Error)]
#[error("{command} in {cwd} timed out after {}", human(*timeout))]
pub struct TimedOut {
    pub command: String,
    pub cwd: String,
    pub timeout: Duration,
}

/// `cmd.output()` with a deadline. `label` names the command in errors and logs.
pub fn output(mut cmd: Command, label: &str, timeout: Duration) -> Result<Output> {
    let deadline = Instant::now() + timeout;
    let cwd = cmd
        .get_current_dir()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = cmd.spawn().with_context(|| format!("running {label}"))?;
    let pid = child.id();
    let stdout = read_all(child.stdout.take());
    let stderr = read_all(child.stderr.take());
    let (status_tx, status_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = status_tx.send(child.wait());
    });
    let left = |d: Instant| d.saturating_duration_since(Instant::now());
    let finished = status_rx.recv_timeout(left(deadline)).ok().and_then(|s| {
        // The pipes close when the last process holding them exits; a grandchild may still.
        let out = stdout.recv_timeout(left(deadline)).ok()?;
        let err = stderr.recv_timeout(left(deadline)).ok()?;
        Some((s, out, err))
    });
    match finished {
        Some((status, stdout, stderr)) => Ok(Output {
            status: status.with_context(|| format!("waiting for {label}"))?,
            stdout,
            stderr,
        }),
        None => {
            // TERM first: git removes its `*.lock` files on a catchable signal; KILL after a grace.
            signal_group("-TERM", pid);
            if status_rx.recv_timeout(KILL_GRACE).is_err() {
                signal_group("-KILL", pid);
                // Reap the leader; the wait thread ends once it is gone.
                let _ = status_rx.recv_timeout(Duration::from_secs(5));
            }
            tracing::warn!(
                command = label,
                cwd = %cwd,
                timeout_secs = timeout.as_secs_f64(),
                "external command timed out"
            );
            Err(TimedOut {
                command: label.to_owned(),
                cwd,
                timeout,
            }
            .into())
        }
    }
}

/// `2s`, or `300ms` under a second.
pub fn human(d: Duration) -> String {
    if d >= Duration::from_secs(1) {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}

/// True when `e` (or its cause) is a [`TimedOut`].
pub fn is_timeout(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<TimedOut>().is_some())
}

fn read_all(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// Between TERM and KILL on a timeout.
const KILL_GRACE: Duration = Duration::from_secs(2);

/// `kill -<sig> -- -<pgid>`: the group the child leads. No `libc` dependency.
fn signal_group(sig: &str, pgid: u32) {
    let _ = Command::new("kill")
        .args([sig, "--", &format!("-{pgid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;

    #[test]
    fn output_and_status_are_kept() {
        let mut c = Command::new("sh");
        c.args(["-c", "echo out; echo err >&2; exit 3"]);
        let o = output(c, "sh", Duration::from_secs(5)).unwrap();
        assert_eq!(o.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&o.stdout), "out\n");
        assert_eq!(String::from_utf8_lossy(&o.stderr), "err\n");
    }

    #[test]
    fn a_command_that_cannot_start_is_an_error_not_a_timeout() {
        let c = Command::new("/nonexistent/rubato-no-such-command");
        let e = output(c, "nothing", Duration::from_secs(5)).unwrap_err();
        assert!(!is_timeout(&e), "{e:#}");
    }

    #[test]
    fn a_hung_command_is_killed_with_its_children() {
        let (events, _g) = capture::start();
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg("sleep 30 & echo $! > \"$1\"; wait")
            .arg("sh")
            .arg(&pid_file)
            .current_dir(dir.path());
        let started = Instant::now();
        let e = output(c, "sh sleeper", Duration::from_millis(300)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(3), "returned late");
        assert!(is_timeout(&e), "{e:#}");
        assert!(
            format!("{e:#}").contains("timed out after 300ms"),
            "names the timeout: {e:#}"
        );
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        // The orphaned sleep was in the group: it is gone (kill -0 fails), allowing the kernel
        // a moment to deliver the signal.
        let gone = (0..50).any(|_| {
            let alive = Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            if alive {
                std::thread::sleep(Duration::from_millis(20));
            }
            !alive
        });
        assert!(gone, "the child's own child still runs");
        let ev = events.lock().unwrap();
        let line = ev
            .iter()
            .find(|e| e.message == "external command timed out")
            .expect("a timeout line");
        assert_eq!(
            line.fields.get("timeout_secs").map(String::as_str),
            Some("0.3")
        );
        assert_eq!(
            line.fields.get("command").map(String::as_str),
            Some("sh sleeper")
        );
    }
}
