//! The launch arm of the runtime. Order (design §9.1, plan 27): lock the registry → read whether
//! a session is live for the issue → unlock → log `launch started` → decide and act in
//! `spawn_blocking` → record a started session → log → send `Launch` → send `CapHit` when the
//! refusal was the attempt limit (`rubato.cap.hit`; an event, not a log line) → send `Sessions`
//! → wake reconcile. The lock is never held while git, tmux-ide or tmux run.

use super::launcher::Launcher;
use crate::sessions::Registry;
use chrono::Utc;
use rubato_protocol::{
    CapHit, CapKind, CompanionMessage, LaunchCommand, LaunchResult, RefusalReason,
};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

pub async fn on_command(
    cmd: &LaunchCommand,
    launcher: &Arc<Launcher>,
    registry: &Mutex<Registry>,
    out: &mpsc::UnboundedSender<CompanionMessage>,
    poll_tx: &mpsc::UnboundedSender<()>,
) {
    let live = registry.lock().await.live_for_issue(&cmd.issue);
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
    if let Some(s) = started {
        registry.lock().await.apply_launch(
            cmd,
            &s.pane_id,
            &s.tmux_session,
            &s.worktree,
            Utc::now(),
        );
    }
    tracing::info!(launch_id = %cmd.id, issue = %cmd.issue, outcome = ?std::mem::discriminant(&result), "{}", rubato_protocol::otel::span::LAUNCH);
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
    use crate::kernel::tmux::Tmux;
    use crate::kernel::tmux_ide::TmuxIde;
    use rubato_protocol::IssueRef;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tokio::time::Instant;
    use uuid::Uuid;

    const USER: &str = "m0x6A";

    /// A stand-in `tmux-ide`: writes a marker file, sleeps 1 s, then reports version `0.0.1`, so
    /// the launch is refused as `missing_dependency` and needs no tmux server. A child `sh`
    /// writes it, so this process never holds the script open for writing (a fork in a
    /// concurrent test would otherwise make running it fail with "text file busy").
    fn slow_tmux_ide(dir: &Path) -> (PathBuf, PathBuf) {
        let script = dir.join("tmux-ide");
        let marker = dir.join("started");
        let body = format!(
            "#!/bin/sh\n: > '{}'\nsleep 1\necho 'tmux-ide 0.0.1'\n",
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

    fn launcher(dir: &Path, tmux_ide_command: &Path) -> Arc<Launcher> {
        let tmux = Tmux::new("tmux", None);
        let command = tmux_ide_command.to_string_lossy().into_owned();
        Arc::new(Launcher {
            workspace: Workspace {
                server_url: None,
                map: MapLocation {
                    repo: "m0x6A/rubato".into(),
                    path: "docs/design/rubato-map.yml".into(),
                    local: dir.to_path_buf(),
                    decisions: None,
                },
                repos: Vec::new(),
                otel: None,
                companion: CompanionConfig {
                    tmux_ide_command: command.clone(),
                    ..CompanionConfig::default()
                },
            },
            paired_user: USER.into(),
            state_dir: dir.to_path_buf(),
            hook_endpoint: HookEndpoint {
                port: 0,
                token: String::new(),
            },
            tmux_ide: TmuxIde::new(command, tmux.clone()),
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
        }
    }

    /// The first message on `out` is the launch result, and it is the refusal the stand-in
    /// causes, so the path went through the tmux-ide version check in the blocking closure.
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
        let (script, marker) = slow_tmux_ide(dir.path());
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
                        return Err("the stand-in tmux-ide never started");
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                // The launch is now inside tmux-ide for a second.
                let deadline = Instant::now() + Duration::from_millis(200);
                loop {
                    if registry.try_lock().is_ok() {
                        return Ok(());
                    }
                    if Instant::now() >= deadline {
                        return Err("the registry lock was held while tmux-ide ran");
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        });

        let cmd = command();
        on_command(&cmd, &launcher, &registry, &out_tx, &poll_tx).await;

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

    #[tokio::test(flavor = "current_thread")]
    async fn runs_on_a_current_thread_runtime() {
        // Not read here. tracing caches a callsite as uninteresting when a thread with no
        // subscriber reaches it first while one other subscriber exists, which would hide
        // `launch started` from the test above when the two run side by side.
        let (_events, _g) = capture::start();
        let dir = tempfile::tempdir().unwrap();
        let (script, _marker) = slow_tmux_ide(dir.path());
        let launcher = launcher(dir.path(), &script);
        let registry = Mutex::new(Registry::new(Uuid::nil(), USER));
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let (poll_tx, _poll_rx) = mpsc::unbounded_channel();

        on_command(&command(), &launcher, &registry, &out_tx, &poll_tx).await;

        assert_missing_dependency(&mut out_rx);
    }
}
