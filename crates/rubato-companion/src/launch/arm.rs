//! The launch arm of the runtime. Order (design §9.1): lock the registry → decide and act in
//! `block_in_place` → unlock → record a started session → log → send `Launch` → send `Sessions`
//! → wake reconcile. `block_in_place` under the lock is moved as is (design §11 follow-up).

use super::launcher::Launcher;
use crate::sessions::Registry;
use chrono::Utc;
use rubato_protocol::{CompanionMessage, LaunchCommand};
use tokio::sync::{Mutex, mpsc};

pub async fn on_command(
    cmd: &LaunchCommand,
    launcher: &Launcher<'_>,
    registry: &Mutex<Registry>,
    out: &mpsc::UnboundedSender<CompanionMessage>,
    poll_tx: &mpsc::UnboundedSender<()>,
) {
    let reg_snapshot = registry.lock().await;
    // The launcher reads the registry; hold the lock only for the decision.
    let (result, started) = {
        let reg = &*reg_snapshot;
        tokio::task::block_in_place(|| launcher.handle(cmd, reg))
    };
    drop(reg_snapshot);
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
    let _ = out.send(CompanionMessage::Launch(result));
    let snap = registry.lock().await.snapshot();
    let _ = out.send(CompanionMessage::Sessions { sessions: snap });
    let _ = poll_tx.send(());
}
