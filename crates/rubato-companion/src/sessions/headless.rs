//! Headless sessions (ADR 0062, plan 122 §D14): at most one `claude -p` turn per launch, held as a
//! child process group. The child is the authority:
//! - its start sets the session *working* (`source: headless`);
//! - its stream events move `last_seen` (at most every 10 s) and, every 60 s, send a progress hook
//!   so the spend check runs during a long turn (A22);
//! - its end sends a `UserPromptSubmit` and a `Stop` carrying the `result` text through the
//!   companion's own hook channel, unless Claude Code's real hooks already did. Status, the
//!   session log, the spend check and the server's dispatch then follow the hook path unchanged.
//!
//! Turns are kept in `turns.json` (pgid and start time), so a restart kills a turn that outlived
//! the previous run before any new turn starts. Every exit path kills every group.

use super::Registry;
use crate::kernel::claude::{self, KillReason, TurnEnd, TurnProgress, TurnRequest};
use crate::kernel::config::write_atomic_private;
use crate::kernel::telemetry::Metrics;
use anyhow::Result;
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{CompanionMessage, HookEvent, HookEventKind, SessionStatus, StateSource};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

const LAST_SEEN_EVERY: Duration = Duration::from_secs(10);
const SPEND_CHECK_EVERY: Duration = Duration::from_secs(60);

/// The script every turn of a launch runs: the launch's environment and hook settings, then
/// `exec claude "$@"` with the turn's own arguments.
pub fn turn_script_path(state_dir: &Path, launch_id: Uuid) -> PathBuf {
    state_dir
        .join("launches")
        .join(launch_id.to_string())
        .join("turn.sh")
}

fn turns_path(state_dir: &Path) -> PathBuf {
    state_dir.join("turns.json")
}

/// A turn as `turns.json` keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    pub launch_id: Uuid,
    pub pgid: u32,
    pub started_at: DateTime<Utc>,
}

struct Turn {
    running: claude::RunningTurn,
    record: TurnRecord,
    /// Set by a kill from outside the watcher (stop, cap, revoke), for the end's reason.
    kill_reason: Option<KillReason>,
}

pub struct HeadlessConfig {
    pub state_dir: PathBuf,
    pub max_total: Duration,
    pub max_quiet: Duration,
}

struct Inner {
    cfg: HeadlessConfig,
    registry: Arc<Mutex<Registry>>,
    /// The companion's own hook channel: synthetic hooks take the real hooks' path.
    hooks: mpsc::UnboundedSender<HookEvent>,
    out: mpsc::UnboundedSender<CompanionMessage>,
    metrics: Arc<Metrics>,
    turns: std::sync::Mutex<HashMap<Uuid, Turn>>,
}

/// The headless turns of this companion. Cheap to clone.
#[derive(Clone)]
pub struct Headless {
    inner: Arc<Inner>,
}

/// Claude Code's transcript for a session in `cwd`: `~/.claude/projects/<cwd, every character
/// that is not a letter or digit as '-'>/<session>.jsonl`. Spend is summed from it.
pub fn transcript_path_for(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    let escaped: String = cwd
        .display()
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    home.join(".claude")
        .join("projects")
        .join(escaped)
        .join(format!("{session_id}.jsonl"))
}

impl Headless {
    pub fn new(
        cfg: HeadlessConfig,
        registry: Arc<Mutex<Registry>>,
        hooks: mpsc::UnboundedSender<HookEvent>,
        out: mpsc::UnboundedSender<CompanionMessage>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                cfg,
                registry,
                hooks,
                out,
                metrics,
                turns: std::sync::Mutex::new(HashMap::new()),
            }),
        }
    }

    /// A turn of this launch is running: a message waits (`not_idle`).
    pub fn is_running(&self, launch_id: Uuid) -> bool {
        self.inner.turns.lock().unwrap().contains_key(&launch_id)
    }

    /// Start a turn of `launch_id`'s session with `args` (`claude::first_turn_args` or
    /// `resume_args`). `prompt` is what the turn was given, for the session log. Refuses while
    /// another turn of the launch runs: never two turns on one transcript.
    pub async fn start(
        &self,
        launch_id: Uuid,
        session_id: &str,
        worktree: &Path,
        args: Vec<String>,
        prompt: String,
    ) -> Result<()> {
        if self.is_running(launch_id) {
            anyhow::bail!("a turn of launch {launch_id} is already running");
        }
        let script = turn_script_path(&self.inner.cfg.state_dir, launch_id);
        let mut all = vec![script.display().to_string()];
        all.extend(args);
        let (running, progress, end) = claude::spawn(TurnRequest {
            program: "sh".into(),
            args: all,
            cwd: worktree.to_owned(),
            env: vec![],
            env_remove: vec![],
            stdin: None,
            max_total: self.inner.cfg.max_total,
            max_quiet: self.inner.cfg.max_quiet,
        })?;
        let record = TurnRecord {
            launch_id,
            pgid: running.pgid,
            started_at: running.started_at,
        };
        self.inner.turns.lock().unwrap().insert(
            launch_id,
            Turn {
                running,
                record: record.clone(),
                kill_reason: None,
            },
        );
        self.save();
        {
            let mut reg = self.inner.registry.lock().await;
            if let Some(e) = reg.find_by_launch(launch_id) {
                e.status = SessionStatus::Working;
                e.source = StateSource::Headless;
                e.last_hook_idle = false;
                e.last_seen = record.started_at;
                e.stale_reported = false;
            }
            let snap = reg.snapshot();
            let _ = self
                .inner
                .out
                .send(CompanionMessage::Sessions { sessions: snap });
        }
        tracing::info!(%launch_id, %session_id, pgid = record.pgid, "headless turn started");
        let this = self.clone();
        let session_id = session_id.to_owned();
        let worktree = worktree.to_owned();
        tokio::spawn(async move {
            this.watch(launch_id, session_id, worktree, prompt, progress, end)
                .await;
        });
        Ok(())
    }

    async fn watch(
        &self,
        launch_id: Uuid,
        session_id: String,
        worktree: PathBuf,
        prompt: String,
        mut progress: mpsc::UnboundedReceiver<TurnProgress>,
        mut end: tokio::sync::oneshot::Receiver<TurnEnd>,
    ) {
        let started = std::time::Instant::now();
        let mut seen_at = started - LAST_SEEN_EVERY;
        let mut spend_at = started;
        let mut progress_open = true;
        let ended = loop {
            tokio::select! {
                p = progress.recv(), if progress_open => match p {
                    Some(TurnProgress::Activity(at)) => {
                        if seen_at.elapsed() >= LAST_SEEN_EVERY {
                            seen_at = std::time::Instant::now();
                            if let Some(e) = self.inner.registry.lock().await.find_by_launch(launch_id) {
                                e.last_seen = at;
                                e.stale_reported = false;
                            }
                        }
                        if spend_at.elapsed() >= SPEND_CHECK_EVERY {
                            spend_at = std::time::Instant::now();
                            self.hook(&session_id, &worktree, "HeadlessProgress", None, None);
                        }
                    }
                    None => progress_open = false,
                },
                e = &mut end => break e.ok(),
            }
        };
        let turn = self.inner.turns.lock().unwrap().remove(&launch_id);
        self.save();
        let Some(turn) = turn else { return };
        let end = ended.unwrap_or(TurnEnd {
            exit_code: None,
            result_text: None,
            cost_usd: None,
            is_error: true,
            subtype: None,
            session_id: None,
            killed: turn.kill_reason,
            ended_at: Utc::now(),
        });
        let reason = turn.kill_reason.or(end.killed);
        self.finish(
            launch_id,
            &session_id,
            &worktree,
            prompt,
            &turn.record,
            &end,
            reason,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish(
        &self,
        launch_id: Uuid,
        session_id: &str,
        worktree: &Path,
        prompt: String,
        record: &TurnRecord,
        end: &TurnEnd,
        reason: Option<KillReason>,
    ) {
        let outcome = match (reason, end.result_text.is_some(), end.exit_code) {
            (Some(KillReason::Timeout), ..) => "timeout",
            (Some(_), ..) => "killed",
            (None, true, Some(0)) => "ok",
            (None, false, _) => "no_result",
            _ => "failed",
        };
        self.inner
            .metrics
            .headless_turns
            .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
        if let Some(r) = reason {
            self.inner
                .metrics
                .headless_killed
                .add(1, &[KeyValue::new(attr::DROP_REASON, r.name())]);
            tracing::warn!(%launch_id, %session_id, reason = r.name(), "headless turn killed");
        }
        tracing::info!(
            %launch_id, %session_id, exit = ?end.exit_code, outcome,
            duration_secs = (end.ended_at - record.started_at).num_seconds(),
            cost_usd = ?end.cost_usd,
            "headless turn ended"
        );
        // Did Claude Code's own hooks report this turn? Then they are the record, and the
        // companion adds nothing (they are only cross-checked).
        let real_stop = {
            let reg = self.inner.registry.lock().await;
            reg.by_launch(launch_id).is_some_and(|e| {
                e.last_hook == Some(HookEventKind::Stop)
                    && e.last_hook_at.is_some_and(|t| t > record.started_at)
            })
        };
        if !real_stop {
            // One prompt id for the pair, as Claude Code gives them: the session log keys its
            // entries by it, and a room turn's reply is found through it (plan 122 §D8).
            let prompt_id = Uuid::new_v4().to_string();
            self.hook(
                session_id,
                worktree,
                "UserPromptSubmit",
                Some(("prompt", prompt)),
                Some(&prompt_id),
            );
            self.hook(
                session_id,
                worktree,
                "Stop",
                end.result_text
                    .clone()
                    .map(|t| ("last_assistant_message", t)),
                Some(&prompt_id),
            );
        }
        if matches!(reason, Some(KillReason::Stop | KillReason::Revoke)) {
            self.hook(session_id, worktree, "SessionEnd", None, None);
            let mut reg = self.inner.registry.lock().await;
            if let Some(e) = reg.find_by_launch(launch_id) {
                e.status = SessionStatus::Ended;
                e.ended = true;
                reg.launches_changed = true;
            }
            super::save_launches_if_changed(&mut reg, &self.inner.cfg.state_dir);
        }
    }

    /// A hook as Claude Code would have sent it, into the companion's own hook channel.
    fn hook(
        &self,
        session_id: &str,
        cwd: &Path,
        name: &str,
        text: Option<(&str, String)>,
        prompt_id: Option<&str>,
    ) {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let mut p = serde_json::json!({
            "session_id": session_id,
            "hook_event_name": name,
            "cwd": cwd.display().to_string(),
            "transcript_path": transcript_path_for(&home, cwd, session_id).display().to_string(),
        });
        if let Some((k, v)) = text {
            p[k] = serde_json::Value::String(v);
        }
        if let Some(id) = prompt_id {
            p["prompt_id"] = serde_json::Value::String(id.to_owned());
        }
        if let Some(ev) = HookEvent::from_payload(p, Utc::now()) {
            let _ = self.inner.hooks.send(ev);
        }
    }

    /// Kill a launch's running turn. Returns false when none runs.
    pub fn kill(&self, launch_id: Uuid, reason: KillReason) -> bool {
        let mut turns = self.inner.turns.lock().unwrap();
        let Some(t) = turns.get_mut(&launch_id) else {
            return false;
        };
        t.kill_reason = Some(reason);
        t.running.kill(reason);
        true
    }

    /// Kill every running turn (revoke, shutdown). Returns how many.
    pub fn stop_all(&self, reason: KillReason) -> usize {
        let ids: Vec<Uuid> = self.inner.turns.lock().unwrap().keys().copied().collect();
        ids.into_iter().filter(|id| self.kill(*id, reason)).count()
    }

    fn save(&self) {
        let records: Vec<TurnRecord> = self
            .inner
            .turns
            .lock()
            .unwrap()
            .values()
            .map(|t| t.record.clone())
            .collect();
        let written = serde_json::to_vec_pretty(&records)
            .map_err(anyhow::Error::from)
            .and_then(|b| write_atomic_private(&turns_path(&self.inner.cfg.state_dir), &b));
        if let Err(e) = written {
            tracing::error!(error = %format!("{e:#}"), "turns.json not written");
        }
    }
}

/// At start, before any turn: kill each turn a previous run left running, if it is still the same
/// process group, and clear the file. Returns the turns killed.
pub fn reap_previous(state_dir: &Path, metrics: &Metrics) -> usize {
    let path = turns_path(state_dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return 0;
    };
    let records: Vec<TurnRecord> = serde_json::from_slice(&bytes).unwrap_or_default();
    let now = Utc::now();
    let mut killed = 0;
    for r in &records {
        if claude::same_group_alive(r.pgid, r.started_at, now) {
            claude::kill_group(r.pgid);
            killed += 1;
            metrics.headless_killed.add(
                1,
                &[KeyValue::new(
                    attr::DROP_REASON,
                    KillReason::CompanionRestarted.name(),
                )],
            );
            tracing::warn!(launch_id = %r.launch_id, pgid = r.pgid, reason = "companion_restarted", "headless turn killed");
        }
    }
    let _ = std::fs::remove_file(&path);
    killed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transcript_path_escapes_the_cwd_as_claude_code_does() {
        let p = transcript_path_for(
            Path::new("/home/dev"),
            Path::new("/home/dev/pantry-worktrees/issue-7.restock"),
            "s1",
        );
        assert_eq!(
            p,
            PathBuf::from(
                "/home/dev/.claude/projects/-home-dev-pantry-worktrees-issue-7-restock/s1.jsonl"
            )
        );
    }

    #[test]
    fn reap_kills_a_live_group_of_ours_and_spares_a_reused_pid() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30"])
            .process_group(0)
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let pgid = child.id();
        let ours = TurnRecord {
            launch_id: Uuid::from_u128(1),
            pgid,
            started_at: Utc::now(),
        };
        let reused = TurnRecord {
            launch_id: Uuid::from_u128(2),
            pgid,
            started_at: Utc::now() - chrono::Duration::hours(3),
        };
        let metrics = Metrics::from_meter(&opentelemetry::global::meter("t"));
        std::fs::write(
            turns_path(dir.path()),
            serde_json::to_vec(&[reused]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            reap_previous(dir.path(), &metrics),
            0,
            "a pid started at another time is not ours"
        );
        assert!(child.try_wait().unwrap().is_none(), "spared");

        std::fs::write(turns_path(dir.path()), serde_json::to_vec(&[ours]).unwrap()).unwrap();
        assert_eq!(reap_previous(dir.path(), &metrics), 1);
        let status = child.wait().unwrap();
        assert!(!status.success(), "killed");
        assert!(!turns_path(dir.path()).exists(), "the file is cleared");
    }

    use std::os::unix::process::CommandExt;
}
