//! The launch arm of the runtime. Order (design §9.1, plan 27): lock the registry → read whether
//! a session is live for the issue → unlock → log `launch started` → decide and act in
//! `spawn_blocking` → record a started session → log → send `Launch` → send `CapHit` when the
//! refusal was the attempt limit (`rubato.cap.hit`; an event, not a log line) → send `Sessions`
//! → wake reconcile. The lock is never held while git or tmux run. Each result is counted in
//! `rubato.launches` (ADR 0070). Commands arrive on
//! the launch task's channel (plan 40), not in the dispatch loop, so hooks keep flowing.

use super::launcher::Launcher;
use crate::kernel::telemetry::Metrics;
use crate::sessions::{Registry, save_launches_if_changed};
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{
    CapHit, CapKind, CompanionMessage, LaunchCommand, LaunchResult, RefusalReason,
};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

/// The launch task (plan 40 S1): one command at a time, off the dispatch loop, so hooks are
/// applied while a launch runs. Essential: ends only when the runtime drops the sender.
pub async fn run(
    mut commands: mpsc::UnboundedReceiver<LaunchCommand>,
    launcher: Arc<Launcher>,
    registry: Arc<Mutex<Registry>>,
    out: mpsc::UnboundedSender<CompanionMessage>,
    poll_tx: mpsc::UnboundedSender<()>,
    headless: Option<crate::sessions::Headless>,
    metrics: Arc<Metrics>,
) -> anyhow::Result<()> {
    while let Some(cmd) = commands.recv().await {
        on_command(
            &cmd,
            &launcher,
            &registry,
            &out,
            &poll_tx,
            headless.as_ref(),
            &metrics,
        )
        .await;
    }
    Ok(())
}

/// `started`, `refused` or `failed`, and a refusal's reason as the wire names it (ADR 0070).
fn outcome_of(result: &LaunchResult) -> (&'static str, Option<String>) {
    match result {
        LaunchResult::Started { .. } => ("started", None),
        LaunchResult::Refused { reason, .. } => (
            "refused",
            serde_json::to_value(reason)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned)),
        ),
        LaunchResult::Failed { .. } => ("failed", None),
    }
}

pub async fn on_command(
    cmd: &LaunchCommand,
    launcher: &Arc<Launcher>,
    registry: &Mutex<Registry>,
    out: &mpsc::UnboundedSender<CompanionMessage>,
    poll_tx: &mpsc::UnboundedSender<()>,
    headless: Option<&crate::sessions::Headless>,
    metrics: &Metrics,
) {
    let live = registry.lock().await.begin_launch(cmd, Utc::now());
    tracing::info!(launch_id = %cmd.id, issue = %cmd.issue, "launch started");
    let blocking = tokio::task::spawn_blocking({
        let launcher = launcher.clone();
        let cmd = cmd.clone();
        move || launcher.handle(&cmd, live)
    });
    let (result, started) = match blocking.await {
        Ok(handled) => handled,
        // The launcher panicked or the task was cancelled: a result, not a dead dispatch loop.
        Err(e) => (
            LaunchResult::Failed {
                command_id: cmd.id,
                error: format!("launch task: {e}"),
                at: Utc::now(),
            },
            None,
        ),
    };
    let mut first_turn = None;
    {
        let mut reg = registry.lock().await;
        reg.end_launch();
        if let Some(s) = started {
            reg.apply_launch(cmd, &s.pane_id, &s.tmux_session, &s.worktree, Utc::now());
            reg.set_restored_hash(cmd.id, s.draft.bundle_hash.clone());
            if s.headless {
                // No pane to type into or to find again after a restart (ADR 0062).
                if let Some(e) = reg.find_by_launch(cmd.id) {
                    e.pane_id = None;
                    e.tmux_session = None;
                    e.window = None;
                }
                first_turn = Some(s.worktree.clone());
            }
            // Kept across a companion restart (plan 122 §D1).
            save_launches_if_changed(&mut reg, &launcher.state_dir);
        }
    }
    let result = match (first_turn, headless) {
        (Some(worktree), Some(h)) => {
            let args = crate::kernel::claude::first_turn_args(
                &cmd.session_id.to_string(),
                &cmd.prompt,
                &launcher.workspace.companion.permission_mode,
            );
            match h
                .start(
                    cmd.id,
                    &cmd.session_id.to_string(),
                    &worktree,
                    args,
                    cmd.prompt.clone(),
                )
                .await
            {
                Ok(()) => result,
                Err(e) => LaunchResult::Failed {
                    command_id: cmd.id,
                    error: format!("starting the headless turn: {e:#}"),
                    at: Utc::now(),
                },
            }
        }
        _ => result,
    };
    let (outcome, reason) = outcome_of(&result);
    tracing::info!(launch_id = %cmd.id, issue = %cmd.issue, outcome, reason = reason.as_deref(), "{}", rubato_protocol::otel::span::LAUNCH);
    let companion = registry.lock().await.companion_id.to_string();
    let mut attrs = vec![
        KeyValue::new(attr::OUTCOME, outcome),
        KeyValue::new(attr::COMPANION_ID, companion),
    ];
    if let Some(r) = reason {
        attrs.push(KeyValue::new(attr::DROP_REASON, r));
    }
    metrics.launches.add(1, &attrs);
    let attempts_used_up = matches!(
        &result,
        LaunchResult::Refused {
            reason: RefusalReason::AttemptLimitReached,
            ..
        }
    );
    let _ = out.send(CompanionMessage::Launch(result));
    if attempts_used_up {
        let (value, limit) = (f64::from(cmd.attempt), f64::from(cmd.attempt_limit));
        tracing::warn!(issue = %cmd.issue, kind = ?CapKind::Attempts, value, limit, "{}", rubato_protocol::otel::event::CAP_HIT);
        let _ = out.send(CompanionMessage::CapHit(CapHit {
            issue: cmd.issue.clone(),
            session_id: None,
            kind: CapKind::Attempts,
            value,
            limit,
            at: Utc::now(),
        }));
    }
    let snap = registry.lock().await.snapshot();
    let _ = out.send(CompanionMessage::Sessions { sessions: snap });
    let _ = poll_tx.send(());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::{CompanionConfig, HookEndpoint, MapLocation, Workspace};
    use crate::kernel::telemetry::capture;
    use crate::kernel::telemetry::metric_reader;
    use crate::kernel::tmux::Tmux;
    use rubato_protocol::IssueRef;
    use rubato_protocol::otel::metric;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tokio::time::Instant;
    use uuid::Uuid;

    const USER: &str = "m0x6A";

    /// A stand-in `tmux`: writes a marker file, sleeps 1 s, then reports `tmux 0.0.1` for `-V`,
    /// so the launch is refused as `missing_dependency` and needs no tmux server. A child `sh`
    /// writes it, so this process never holds the script open for writing (a fork in a
    /// concurrent test would otherwise make running it fail with "text file busy").
    fn slow_tmux(dir: &Path) -> (PathBuf, PathBuf) {
        let script = dir.join("tmux");
        let marker = dir.join("started");
        let body = format!(
            "#!/bin/sh\n: > '{}'\nsleep 1\necho 'tmux 0.0.1'\n",
            marker.display()
        );
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg(&body)
            .arg(&script)
            .status()
            .unwrap();
        assert!(status.success());
        (script, marker)
    }

    fn launcher(dir: &Path, tmux_command: &Path) -> Arc<Launcher> {
        launcher_with(dir, tmux_command, Duration::from_secs(30))
    }

    fn launcher_with(dir: &Path, tmux_command: &Path, timeout: Duration) -> Arc<Launcher> {
        let command = tmux_command.to_string_lossy().into_owned();
        let tmux = Tmux {
            timeout,
            ..Tmux::new(command.clone(), None)
        };
        Arc::new(Launcher {
            workspace: Workspace {
                server_url: None,
                map: Some(MapLocation {
                    repo: "m0x6A/rubato".into(),
                    path: "docs/design/rubato-map.yml".into(),
                    local: dir.to_path_buf(),
                    decisions: None,
                }),
                maps: vec![],
                repos: Vec::new(),
                otel: None,
                companion: CompanionConfig {
                    tmux_command: command,
                    ..CompanionConfig::default()
                },
            },
            paired_user: USER.into(),
            state_dir: dir.to_path_buf(),
            hook_endpoint: HookEndpoint {
                port: 0,
                token: String::new(),
            },
            tmux,
        })
    }

    fn command() -> LaunchCommand {
        LaunchCommand {
            id: Uuid::from_u128(27),
            user: USER.into(),
            issue: IssueRef::new("m0x6A/rubato", 27),
            repo_url: "https://github.com/m0x6A/rubato.git".into(),
            branch: "issue/27-launch-without-lock".into(),
            base_ref: "origin/main".into(),
            node_ids: Vec::new(),
            spend_cap_usd: 20.0,
            attempt_limit: 3,
            attempt: 1,
            session_id: Uuid::from_u128(1),
            prompt: String::new(),
            otel_resource: BTreeMap::new(),
            map_version: None,
            plan_path: None,
            draft: None,
            planning: Default::default(),
        }
    }

    /// The first message on `out` is the launch result, and it is the refusal the stand-in
    /// causes, so the path went through the tmux version check in the blocking closure.
    fn assert_missing_dependency(out: &mut mpsc::UnboundedReceiver<CompanionMessage>) {
        match out.try_recv() {
            Ok(CompanionMessage::Launch(LaunchResult::Refused {
                command_id,
                reason: RefusalReason::MissingDependency,
                ..
            })) => assert_eq!(command_id, command().id),
            other => panic!("expected a missing_dependency refusal first, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registry_is_free_during_a_slow_launch() {
        // The test body is the runtime's `block_on`, so `on_command` runs on this thread and the
        // arm's own log lines reach this thread's subscriber.
        let (events, _g) = capture::start();
        let dir = tempfile::tempdir().unwrap();
        let (script, marker) = slow_tmux(dir.path());
        let launcher = launcher(dir.path(), &script);
        let registry = Arc::new(Mutex::new(Registry::new(Uuid::nil(), USER)));
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let (poll_tx, _poll_rx) = mpsc::unbounded_channel();

        let probe = tokio::spawn({
            let registry = registry.clone();
            async move {
                let give_up = Instant::now() + Duration::from_secs(5);
                while !marker.exists() {
                    if Instant::now() >= give_up {
                        return Err("the stand-in tmux never started");
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                // The launch is now inside `tmux -V` for a second.
                let deadline = Instant::now() + Duration::from_millis(200);
                loop {
                    if registry.try_lock().is_ok() {
                        return Ok(());
                    }
                    if Instant::now() >= deadline {
                        return Err("the registry lock was held while tmux ran");
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        });

        let cmd = command();
        on_command(
            &cmd,
            &launcher,
            &registry,
            &out_tx,
            &poll_tx,
            None,
            &Metrics::new(),
        )
        .await;

        probe.await.unwrap().unwrap();
        assert_missing_dependency(&mut out_rx);
        let ev = events.lock().unwrap();
        let id = cmd.id.to_string();
        let at = |message: &str| {
            ev.iter().position(|e| {
                e.message == message
                    && e.fields.get("launch_id").map(String::as_str) == Some(id.as_str())
            })
        };
        let started = at("launch started").expect("a `launch started` line with the launch id");
        let result = at(rubato_protocol::otel::span::LAUNCH).expect("the rubato.launch line");
        assert!(started < result, "`launch started` comes before the result");
    }

    /// Plan 40 S4: the stand-in sleeps 1 s before answering; a 300 ms deadline makes the version
    /// check time out, and the launch is `Failed`, not `missing_dependency`.
    #[tokio::test(flavor = "current_thread")]
    async fn a_version_check_that_times_out_is_failed() {
        let (_events, _g) = capture::start();
        let dir = tempfile::tempdir().unwrap();
        let (script, _marker) = slow_tmux(dir.path());
        let launcher = launcher_with(dir.path(), &script, Duration::from_millis(300));
        let registry = Mutex::new(Registry::new(Uuid::nil(), USER));
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let (poll_tx, _poll_rx) = mpsc::unbounded_channel();

        on_command(
            &command(),
            &launcher,
            &registry,
            &out_tx,
            &poll_tx,
            None,
            &Metrics::new(),
        )
        .await;

        match out_rx.try_recv() {
            Ok(CompanionMessage::Launch(LaunchResult::Failed { error, .. })) => {
                assert!(error.contains("timed out after 300ms"), "{error}");
            }
            other => panic!("expected a failed launch first, got {other:?}"),
        }
    }

    /// ADR 0070: every result is counted once in `rubato.launches`, by outcome and, for a refusal,
    /// its reason; the `rubato.launch` line says the same word, not a `Discriminant`.
    #[tokio::test(flavor = "current_thread")]
    async fn each_launch_result_is_counted_with_its_outcome() {
        let (events, _g) = capture::start();
        let (reader, metrics) = metric_reader::start();
        let dir = tempfile::tempdir().unwrap();
        let (script, _marker) = slow_tmux(dir.path());
        let registry = Mutex::new(Registry::new(Uuid::from_u128(5), USER));
        let (out_tx, _out_rx) = mpsc::unbounded_channel();
        let (poll_tx, _poll_rx) = mpsc::unbounded_channel();
        // Refused: `tmux 0.0.1` is below the pin.
        let refusing = launcher(dir.path(), &script);
        on_command(
            &command(),
            &refusing,
            &registry,
            &out_tx,
            &poll_tx,
            None,
            &metrics,
        )
        .await;
        // Failed: `tmux -V` outlives a 300 ms deadline.
        let hanging = launcher_with(dir.path(), &script, Duration::from_millis(300));
        on_command(
            &command(),
            &hanging,
            &registry,
            &out_tx,
            &poll_tx,
            None,
            &metrics,
        )
        .await;

        let mut seen: Vec<(String, Option<String>, u64)> = reader
            .points(metric::LAUNCHES)
            .into_iter()
            .map(|p| {
                assert_eq!(
                    p.attrs.get(attr::COMPANION_ID).map(String::as_str),
                    Some(Uuid::from_u128(5).to_string().as_str())
                );
                (
                    p.attrs[attr::OUTCOME].clone(),
                    p.attrs.get(attr::DROP_REASON).cloned(),
                    p.value,
                )
            })
            .collect();
        seen.sort();
        assert_eq!(
            seen,
            [
                ("failed".to_owned(), None, 1),
                (
                    "refused".to_owned(),
                    Some("missing_dependency".to_owned()),
                    1
                ),
            ]
        );
        let outcomes: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.message == rubato_protocol::otel::span::LAUNCH)
            .map(|e| e.fields["outcome"].clone())
            .collect();
        assert_eq!(outcomes, ["refused", "failed"]);
        assert_eq!(
            outcome_of(&LaunchResult::Started {
                command_id: Uuid::nil(),
                session_id: Uuid::nil(),
                worktree: String::new(),
                tmux_session: String::new(),
                at: Utc::now(),
                draft_restored: false,
                restore_skipped: None,
            }),
            ("started", None)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runs_on_a_current_thread_runtime() {
        // Not read here. tracing caches a callsite as uninteresting when a thread with no
        // subscriber reaches it first while one other subscriber exists, which would hide
        // `launch started` from the test above when the two run side by side.
        let (_events, _g) = capture::start();
        let dir = tempfile::tempdir().unwrap();
        let (script, _marker) = slow_tmux(dir.path());
        let launcher = launcher(dir.path(), &script);
        let registry = Mutex::new(Registry::new(Uuid::nil(), USER));
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let (poll_tx, _poll_rx) = mpsc::unbounded_channel();

        on_command(
            &command(),
            &launcher,
            &registry,
            &out_tx,
            &poll_tx,
            None,
            &Metrics::new(),
        )
        .await;

        assert_missing_dependency(&mut out_rx);
    }
}
