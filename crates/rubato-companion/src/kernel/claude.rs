//! Headless Claude Code turns (ADR 0059, plan 122 §D14). Each turn is one `claude -p` child that
//! leads its own process group; the companion holds it, reads its stream-json stdout, and is the
//! authority for the turn's end, its reply and its liveness. Hooks, if they fire, are cross-checked
//! only. Bounded twice: a total time and a time without any stream event (A19).
//!
//! At most one turn per session runs. The caller records `pgid` and `started_at`, so that after a
//! restart [`same_group_alive`] can tell a live turn of ours from a reused pid.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

/// Why a turn was killed (`rubato.headless.killed{rubato.reason}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillReason {
    Stop,
    Cap,
    Revoke,
    Timeout,
    Quiet,
    CompanionRestarted,
}

impl KillReason {
    pub fn name(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Cap => "cap",
            Self::Revoke => "revoke",
            Self::Timeout => "timeout",
            Self::Quiet => "quiet",
            Self::CompanionRestarted => "companion_restarted",
        }
    }
}

/// One turn to run.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub max_total: Duration,
    pub max_quiet: Duration,
}

/// The arguments of the first turn of a session.
pub fn first_turn_args(session_id: &str, prompt: &str, permission_mode: &str) -> Vec<String> {
    let mut a = vec![
        "-p".to_owned(),
        prompt.to_owned(),
        "--session-id".to_owned(),
        session_id.to_owned(),
    ];
    a.extend(common(permission_mode));
    a
}

/// The arguments of a later turn: one message typed into the session.
pub fn resume_args(session_id: &str, text: &str, permission_mode: &str) -> Vec<String> {
    let mut a = vec![
        "-p".to_owned(),
        "--resume".to_owned(),
        session_id.to_owned(),
        text.to_owned(),
    ];
    a.extend(common(permission_mode));
    a
}

fn common(permission_mode: &str) -> Vec<String> {
    vec![
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--permission-mode".into(),
        permission_mode.into(),
    ]
}

/// What a turn sends while it runs.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnProgress {
    /// Any stream event: the turn is alive.
    Activity(DateTime<Utc>),
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnEnd {
    /// The process exit code; None when it was killed by a signal.
    pub exit_code: Option<i32>,
    /// The stream's `result` event text, the reply.
    pub result_text: Option<String>,
    /// The `result` event's `total_cost_usd`, cross-checked against the transcript sum.
    pub cost_usd: Option<f64>,
    /// The `result` event said `is_error`.
    pub is_error: bool,
    /// Set when the companion killed it.
    pub killed: Option<KillReason>,
    pub ended_at: DateTime<Utc>,
}

/// A running turn: its process group and a way to kill it with a reason.
#[derive(Debug)]
pub struct RunningTurn {
    pub pgid: u32,
    pub started_at: DateTime<Utc>,
    kill: Option<oneshot::Sender<KillReason>>,
}

impl RunningTurn {
    /// Kill the turn's group; its [`TurnEnd`] carries the reason. Idempotent.
    pub fn kill(&mut self, reason: KillReason) {
        if let Some(tx) = self.kill.take() {
            let _ = tx.send(reason);
        }
    }
}

/// Start a turn. Returns the running turn, its progress, and its end.
pub fn spawn(
    req: TurnRequest,
) -> Result<(
    RunningTurn,
    mpsc::UnboundedReceiver<TurnProgress>,
    oneshot::Receiver<TurnEnd>,
)> {
    let mut cmd = Command::new(&req.program);
    cmd.args(&req.args)
        .current_dir(&req.cwd)
        .envs(req.env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        // A panic or an aborted task drops the child: its group goes with it.
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting {}", req.program))?;
    let pgid = child.id().context("turn has no pid")?;
    let started_at = Utc::now();
    let stdout = child.stdout.take().context("turn stdout")?;
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (end_tx, end_rx) = oneshot::channel();
    let (kill_tx, mut kill_rx) = oneshot::channel::<KillReason>();

    tokio::spawn(async move {
        let _guard = GroupGuard(pgid);
        let mut lines = BufReader::new(stdout).lines();
        let deadline = tokio::time::Instant::now() + req.max_total;
        let mut result_text = None;
        let mut cost_usd = None;
        let mut is_error = false;
        let mut killed = None;
        loop {
            tokio::select! {
                reason = &mut kill_rx => {
                    if let Ok(r) = reason {
                        killed = Some(r);
                    }
                    break;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    killed = Some(KillReason::Timeout);
                    break;
                }
                line = tokio::time::timeout(req.max_quiet, lines.next_line()) => match line {
                    Err(_) => {
                        killed = Some(KillReason::Quiet);
                        break;
                    }
                    Ok(Ok(Some(l))) => {
                        let _ = progress_tx.send(TurnProgress::Activity(Utc::now()));
                        if let Some(r) = parse_result(&l) {
                            result_text = r.text;
                            cost_usd = r.cost_usd;
                            is_error = r.is_error;
                        }
                    }
                    // End of output: the turn is ending.
                    Ok(_) => break,
                }
            }
        }
        if killed.is_some() {
            kill_group(pgid);
        }
        let status = child.wait().await.ok();
        let _ = end_tx.send(TurnEnd {
            exit_code: status.and_then(|s| s.code()),
            result_text,
            cost_usd,
            is_error,
            killed,
            ended_at: Utc::now(),
        });
    });
    Ok((
        RunningTurn {
            pgid,
            started_at,
            kill: Some(kill_tx),
        },
        progress_rx,
        end_rx,
    ))
}

/// Kills the group when the watcher ends for any reason, a normal exit included: a turn's
/// helpers never outlive it.
struct GroupGuard(u32);

impl Drop for GroupGuard {
    fn drop(&mut self) {
        kill_group(self.0);
    }
}

/// TERM, then KILL, to a turn's whole group.
pub fn kill_group(pgid: u32) {
    super::proc::signal_group("-TERM", pgid);
    super::proc::signal_group("-KILL", pgid);
}

struct ResultEvent {
    text: Option<String>,
    cost_usd: Option<f64>,
    is_error: bool,
}

/// The stream-json `result` event: `{"type":"result","result":"…","total_cost_usd":…}`.
fn parse_result(line: &str) -> Option<ResultEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "result" {
        return None;
    }
    Some(ResultEvent {
        text: v.get("result").and_then(|r| r.as_str()).map(str::to_owned),
        cost_usd: v.get("total_cost_usd").and_then(|c| c.as_f64()),
        is_error: v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false),
    })
}

/// Whether the group `pgid`, started at `started_at`, still runs. A pid reused by another process
/// started at a different time does not count (plan 122 §D14, "one turn at a time").
pub fn same_group_alive(pgid: u32, started_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-o", "etime=", "-p", &pgid.to_string()])
        .output();
    let Ok(out) = out else { return false };
    if !out.status.success() {
        return false;
    }
    let Some(elapsed) = parse_etime(String::from_utf8_lossy(&out.stdout).trim()) else {
        return false;
    };
    let started = now - chrono::Duration::seconds(elapsed as i64);
    (started - started_at).num_seconds().abs() <= 5
}

/// `ps -o etime`: `[[dd-]hh:]mm:ss`, on Linux and macOS alike.
fn parse_etime(s: &str) -> Option<u64> {
    let (days, rest) = match s.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, s),
    };
    let parts: Vec<u64> = rest
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let (h, m, sec) = match parts.as_slice() {
        [m, s] => (0, *m, *s),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    Some(days * 86_400 + h * 3_600 + m * 60 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn standin(dir: &std::path::Path, body: &str) -> String {
        let p = dir.join("claude");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.display().to_string()
    }

    fn req(program: String, dir: &std::path::Path) -> TurnRequest {
        TurnRequest {
            program,
            args: resume_args("s1", "hello", "acceptEdits"),
            cwd: dir.to_owned(),
            env: vec![],
            max_total: Duration::from_secs(10),
            max_quiet: Duration::from_secs(10),
        }
    }

    #[test]
    fn args_name_the_session_and_stream_json() {
        let a = first_turn_args("s1", "plan it", "plan");
        assert_eq!(&a[..4], ["-p", "plan it", "--session-id", "s1"]);
        assert!(
            a.windows(2)
                .any(|w| w == ["--output-format", "stream-json"])
        );
        let r = resume_args("s1", "next", "plan");
        assert_eq!(&r[..4], ["-p", "--resume", "s1", "next"]);
    }

    #[test]
    fn etime_parses_every_form() {
        assert_eq!(parse_etime("00:07"), Some(7));
        assert_eq!(parse_etime("01:02:03"), Some(3723));
        assert_eq!(parse_etime("2-00:00:01"), Some(172_801));
        assert_eq!(parse_etime("bad"), None);
    }

    #[tokio::test]
    async fn the_result_event_is_the_reply_and_its_cost() {
        let dir = tempfile::tempdir().unwrap();
        let p = standin(
            dir.path(),
            r#"echo '{"type":"system","subtype":"init"}'
echo '{"type":"result","result":"PONG","total_cost_usd":0.25,"is_error":false}'"#,
        );
        let (_turn, mut progress, end) = spawn(req(p, dir.path())).unwrap();
        let end = end.await.unwrap();
        assert_eq!(end.result_text.as_deref(), Some("PONG"));
        assert_eq!(end.cost_usd, Some(0.25));
        assert_eq!(end.exit_code, Some(0));
        assert_eq!(end.killed, None);
        let mut events = 0;
        while progress.try_recv().is_ok() {
            events += 1;
        }
        assert_eq!(events, 2, "one activity per stream line");
    }

    #[tokio::test]
    async fn a_turn_without_a_result_ends_with_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = standin(dir.path(), "echo '{\"type\":\"system\"}'; exit 3");
        let (_turn, _p, end) = spawn(req(p, dir.path())).unwrap();
        let end = end.await.unwrap();
        assert_eq!(end.result_text, None);
        assert_eq!(end.exit_code, Some(3));
    }

    #[tokio::test]
    async fn kill_stops_the_whole_group_with_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("child.pid");
        let p = standin(
            dir.path(),
            &format!(
                "sleep 30 & echo $! > {}; echo '{{\"type\":\"system\"}}'; wait",
                marker.display()
            ),
        );
        let (mut turn, mut progress, end) = spawn(req(p, dir.path())).unwrap();
        progress.recv().await.unwrap();
        turn.kill(KillReason::Stop);
        let end = end.await.unwrap();
        assert_eq!(end.killed, Some(KillReason::Stop));
        let child: u32 = std::fs::read_to_string(&marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let alive = std::process::Command::new("kill")
            .args(["-0", &child.to_string()])
            .status()
            .unwrap()
            .success();
        assert!(!alive, "the turn's child outlived the kill");
    }

    #[tokio::test]
    async fn a_quiet_turn_is_killed_as_quiet_and_a_long_one_as_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let p = standin(dir.path(), "sleep 30");
        let mut r = req(p.clone(), dir.path());
        r.max_quiet = Duration::from_millis(200);
        let (_t, _p, end) = spawn(r).unwrap();
        assert_eq!(end.await.unwrap().killed, Some(KillReason::Quiet));

        let chatty = standin(
            dir.path(),
            "while true; do echo '{\"type\":\"x\"}'; sleep 0.05; done",
        );
        let mut r = req(chatty, dir.path());
        r.max_total = Duration::from_millis(300);
        let (_t, _p, end) = spawn(r).unwrap();
        assert_eq!(end.await.unwrap().killed, Some(KillReason::Timeout));
    }

    #[tokio::test]
    async fn a_live_group_is_ours_only_with_its_start_time() {
        let dir = tempfile::tempdir().unwrap();
        let p = standin(dir.path(), "echo '{\"type\":\"x\"}'; sleep 30");
        let (mut turn, mut progress, end) = spawn(req(p, dir.path())).unwrap();
        progress.recv().await.unwrap();
        let now = Utc::now();
        assert!(same_group_alive(turn.pgid, turn.started_at, now));
        assert!(
            !same_group_alive(turn.pgid, turn.started_at - chrono::Duration::hours(1), now),
            "a pid started at another time is not our turn"
        );
        turn.kill(KillReason::CompanionRestarted);
        let pgid = turn.pgid;
        end.await.unwrap();
        assert!(!same_group_alive(pgid, turn.started_at, Utc::now()));
    }
}
