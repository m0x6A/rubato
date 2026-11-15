//! Claude Code as a headless CLI. Every turn is one `claude -p` child that leads its own process
//! group; the companion holds it, reads its stream-json stdout, and is the authority for the
//! turn's end, its reply and its liveness. Bounded twice: a total time and a time without any
//! stream event (A19). A Drop guard kills the group on every exit path, a panic included.
//!
//! Two kinds of turn run on the same [`spawn`]:
//! - headless sessions (ADR 0062, plan 122 §D14): the first turn ([`first_turn_args`]) and resume
//!   turns ([`resume_args`]). At most one per session; the caller records `pgid` and
//!   `started_at`, so that after a restart [`same_group_alive`] can tell a live turn of ours from
//!   a reused pid;
//! - read-only fork turns of a room agent (ADR 0057, plan 122 §D8, [`Claude`]): `claude -p --resume
//!   <session> --fork-session --permission-mode plan`, later turns `--resume` the fork. The
//!   spike showed a fork leaves the live session's transcript alone. No Rubato hooks: the launch's
//!   `hooks.json` reaches a session only through `--settings` in `launch.sh`, which a fork never
//!   gets; the fork's settings say `disableAllHooks`, so hooks a developer installed by hand do
//!   not fire either; and the `RUBATO_*` variables `launch.sh` exports are removed. The turn's
//!   text goes on stdin, so its length meets no argument limit.
//!
//! Both read the stream with [`parse_event`].

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
    /// Variables the child must not inherit.
    pub env_remove: Vec<String>,
    /// Written to the child's stdin, then closed; None gives it no stdin.
    pub stdin: Option<String>,
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
    /// The `result` event's `subtype` (`success`, `error_max_turns`, …).
    pub subtype: Option<String>,
    /// The session the turn ran in, from the `result` event or else the `init` one: a fork's
    /// own session, for the next `--resume`.
    pub session_id: Option<String>,
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
        .stdin(if req.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        // A panic or an aborted task drops the child: its group goes with it.
        .kill_on_drop(true);
    for v in &req.env_remove {
        cmd.env_remove(v);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting {}", req.program))?;
    let pgid = child.id().context("turn has no pid")?;
    let started_at = Utc::now();
    let stdout = child.stdout.take().context("turn stdout")?;
    if let (Some(text), Some(mut stdin)) = (req.stdin.clone(), child.stdin.take()) {
        // Dropped once written: the child sees the end of its input.
        tokio::spawn(async move {
            let _ = stdin.write_all(text.as_bytes()).await;
        });
    }
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (end_tx, end_rx) = oneshot::channel();
    let (kill_tx, mut kill_rx) = oneshot::channel::<KillReason>();

    tokio::spawn(async move {
        let _guard = GroupGuard(pgid);
        let mut lines = BufReader::new(stdout).lines();
        let deadline = tokio::time::Instant::now() + req.max_total;
        let mut stream = Stream::default();
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
                        stream.read(&l);
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
        let _ = end_tx.send(stream.end(status.and_then(|s| s.code()), killed));
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

/// One stream-json event the companion reads.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// `{"type":"system","subtype":"init","session_id":…}`.
    Init { session_id: Option<String> },
    /// `{"type":"result","result":"…","total_cost_usd":…,"is_error":…,"session_id":…}`.
    Result {
        text: Option<String>,
        cost_usd: Option<f64>,
        is_error: bool,
        subtype: Option<String>,
        session_id: Option<String>,
    },
}

/// One line of stream-json, when it is an event the companion reads.
pub fn parse_event(line: &str) -> Option<StreamEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned);
    match v.get("type")?.as_str()? {
        "system" => Some(StreamEvent::Init {
            session_id: s("session_id"),
        }),
        "result" => Some(StreamEvent::Result {
            text: s("result"),
            cost_usd: v.get("total_cost_usd").and_then(|c| c.as_f64()),
            is_error: v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false),
            subtype: s("subtype"),
            session_id: s("session_id"),
        }),
        _ => None,
    }
}

/// What a stream said so far: the last result, and the session.
#[derive(Debug, Default)]
struct Stream {
    result_text: Option<String>,
    cost_usd: Option<f64>,
    is_error: bool,
    subtype: Option<String>,
    session_id: Option<String>,
}

impl Stream {
    fn read(&mut self, line: &str) {
        match parse_event(line) {
            Some(StreamEvent::Init { session_id }) if self.session_id.is_none() => {
                self.session_id = session_id;
            }
            Some(StreamEvent::Result {
                text,
                cost_usd,
                is_error,
                subtype,
                session_id,
            }) => {
                self.result_text = text;
                self.cost_usd = cost_usd;
                self.is_error = is_error;
                self.subtype = subtype;
                if session_id.is_some() {
                    self.session_id = session_id;
                }
            }
            _ => {}
        }
    }

    fn end(self, exit_code: Option<i32>, killed: Option<KillReason>) -> TurnEnd {
        TurnEnd {
            exit_code,
            result_text: self.result_text,
            cost_usd: self.cost_usd,
            is_error: self.is_error,
            subtype: self.subtype,
            session_id: self.session_id,
            killed,
            ended_at: Utc::now(),
        }
    }
}

/// A whole stream-json output read at once, as [`spawn`] reads it line by line.
pub fn parse_stream(stdout: &str, exit_code: Option<i32>) -> TurnEnd {
    let mut s = Stream::default();
    for line in stdout.lines() {
        s.read(line);
    }
    s.end(exit_code, None)
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

// ----- read-only fork turns (ADR 0057) ------------------------------------------------------

/// The fork's own settings: no hooks at all.
pub const NO_HOOKS: &str = r#"{"disableAllHooks":true}"#;

/// What `launch.sh` exports for a launched session; a fork carries none of it.
pub const LAUNCH_ENV: &[&str] = &[
    "RUBATO_LAUNCH_ID",
    "RUBATO_HOOK_URL",
    "RUBATO_HOOK_TOKEN",
    "RUBATO_PLANNING",
    "TMUX",
];

/// How the companion runs fork turns.
#[derive(Debug, Clone)]
pub struct Claude {
    /// `companion.claude_command`, run through `sh` as `launch.sh` runs it.
    pub command: String,
    /// A fork turn's deadline, both in total and without a stream event.
    pub timeout: Duration,
}

/// What one fork turn gave back.
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

impl Turn {
    /// A fork turn's answer from how its child ended. A failed turn keeps its cost.
    pub fn from_end(end: TurnEnd) -> Self {
        let error = if let Some(k) = end.killed {
            Some(format!("killed: {}", k.name()))
        } else if end.is_error {
            let subtype = end.subtype.as_deref().unwrap_or("error");
            Some(match end.result_text.as_deref() {
                Some(t) if !t.is_empty() => format!("{subtype}: {t}"),
                _ => subtype.to_owned(),
            })
        } else if end.result_text.is_none() {
            Some(format!(
                "no result from claude (exit {})",
                end.exit_code.map_or("signal".to_owned(), |c| c.to_string())
            ))
        } else {
            None
        };
        Turn {
            text: if error.is_none() {
                end.result_text.unwrap_or_default()
            } else {
                String::new()
            },
            session_id: end.session_id,
            cost_usd: end.cost_usd,
            exit_code: end.exit_code,
            error,
        }
    }
}

/// The arguments of one fork turn: `fork` makes a new fork of `resume`; otherwise `resume` is
/// the fork itself.
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

impl Claude {
    /// The request for one fork turn in `worktree`, with `prompt` on stdin.
    pub fn request(&self, worktree: &Path, resume: &str, fork: bool, prompt: &str) -> TurnRequest {
        let mut args = vec![
            "-c".to_owned(),
            format!("{} \"$@\"", self.command),
            "sh".to_owned(),
        ];
        args.extend(turn_args(resume, fork));
        TurnRequest {
            program: "sh".into(),
            args,
            cwd: worktree.to_path_buf(),
            env: vec![],
            env_remove: LAUNCH_ENV.iter().map(|v| (*v).to_owned()).collect(),
            stdin: Some(prompt.to_owned()),
            max_total: self.timeout,
            max_quiet: self.timeout,
        }
    }

    /// Start one fork turn on [`spawn`]: bounded and killable like any turn.
    pub fn start(
        &self,
        worktree: &Path,
        resume: &str,
        fork: bool,
        prompt: &str,
    ) -> Result<(RunningTurn, oneshot::Receiver<TurnEnd>)> {
        let (running, _progress, end) =
            spawn(self.request(worktree, resume, fork, prompt)).context("running the fork turn")?;
        Ok((running, end))
    }
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
            env_remove: vec![],
            stdin: None,
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

    #[test]
    fn a_first_fork_turn_forks_and_a_later_one_resumes_the_fork() {
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
    fn the_stream_gives_text_session_and_cost_and_a_failed_turn_keeps_its_cost() {
        let out = r#"{"type":"system","subtype":"init","session_id":"fork-1"}
{"type":"assistant","message":{"content":[{"type":"text","text":"thinking"}]}}
not json at all
{"type":"result","subtype":"success","is_error":false,"result":"Because b keeps the seam.","session_id":"fork-1","total_cost_usd":0.1}"#;
        let t = Turn::from_end(parse_stream(out, Some(0)));
        assert_eq!(t.text, "Because b keeps the seam.");
        assert_eq!(t.session_id.as_deref(), Some("fork-1"));
        assert_eq!(t.cost_usd, Some(0.1));
        assert_eq!(t.error, None);

        let failed = r#"{"type":"system","subtype":"init","session_id":"fork-2"}
{"type":"result","subtype":"error_max_turns","is_error":true,"total_cost_usd":0.05}"#;
        let t = Turn::from_end(parse_stream(failed, Some(1)));
        assert_eq!(t.error.as_deref(), Some("error_max_turns"));
        assert_eq!(
            (t.text.as_str(), t.cost_usd),
            ("", Some(0.05)),
            "a failed turn still cost"
        );
        assert_eq!(
            t.session_id.as_deref(),
            Some("fork-2"),
            "from init when the result has none"
        );

        let t = Turn::from_end(parse_stream("", Some(2)));
        assert!(t.error.unwrap().contains("no result from claude (exit 2)"));
    }

    /// The fork on the shared spawn: the prompt arrives on stdin, the launch's variables do not,
    /// and the turn runs in the worktree.
    #[tokio::test]
    async fn a_fork_turn_reads_its_prompt_from_stdin_without_the_launch_env() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let p = standin(
            dir.path(),
            &format!(
                "{{ echo \"ARGS: $*\"; env | grep '^RUBATO_'; echo \"PWD: $PWD\"; cat; }} > '{}'\necho '{{\"type\":\"result\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"f\",\"total_cost_usd\":0.01}}'",
                log.display()
            ),
        );
        let claude = Claude {
            command: p,
            timeout: Duration::from_secs(10),
        };
        let prompt = "Messages from the plan's room follow.\n\nalice: why b?";
        let (_t, _p, end) = spawn(claude.request(dir.path(), "s1", true, prompt)).unwrap();
        let t = Turn::from_end(end.await.unwrap());
        assert_eq!((t.text.as_str(), t.cost_usd), ("ok", Some(0.01)));
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(
            text.contains("ARGS: -p --resume s1 --fork-session"),
            "{text}"
        );
        assert!(text.ends_with(prompt), "{text}");

        // A launch variable set for the child is still removed: `env_remove` wins.
        let mut r = claude.request(dir.path(), "s1", false, prompt);
        r.env.push(("RUBATO_HOOK_TOKEN".into(), "x".into()));
        let (_t, _p, end) = spawn(r).unwrap();
        end.await.unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(!text.lines().any(|l| l.starts_with("RUBATO_")), "{text}");
        assert!(!text.contains("--fork-session"), "{text}");
    }
}
