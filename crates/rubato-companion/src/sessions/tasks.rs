//! Discovery tasks: follow tmux-ide events (best-effort) and reconcile pane metadata with
//! `tmux-ide team --json` on a timer and whenever woken.

use super::Registry;
use crate::kernel::telemetry::Metrics;
use crate::kernel::tmux::Tmux;
use crate::kernel::tmux_ide::TmuxIde;
use anyhow::Result;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{CompanionMessage, Limits};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, watch};

/// Each line of `tmux-ide events --follow` wakes reconcile. The command needs an adopted session
/// and exits at once without one (no `events.jsonl`), which is normal on a fresh machine: then
/// discovery runs on the timer alone and this task ends (it is best-effort, design D12).
pub async fn watch_events(
    tmux_ide: TmuxIde,
    poll_tx: mpsc::UnboundedSender<()>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let unavailable = || tracing::info!("tmux-ide events unavailable; timer only");
    let child = tokio::task::spawn_blocking(move || tmux_ide.events_follow()).await;
    let Ok(Ok(mut child)) = child else {
        unavailable();
        return Ok(());
    };
    let Some(stdout) = child.stdout.take() else {
        unavailable();
        return Ok(());
    };
    let stdout = tokio::process::ChildStdout::from_std(stdout)?;
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(_)) => { let _ = poll_tx.send(()); }
                _ => { let _ = child.kill(); unavailable(); return Ok(()); }
            },
            _ = shutdown.changed() => { let _ = child.kill(); return Ok(()); }
        }
    }
}

/// Panes and fleet → registry → snapshot; the snapshot goes to the server only when it changed.
#[allow(clippy::too_many_arguments)]
pub async fn reconcile(
    out: mpsc::UnboundedSender<CompanionMessage>,
    registry: Arc<Mutex<Registry>>,
    metrics: Arc<Metrics>,
    tmux: Tmux,
    tmux_ide: TmuxIde,
    limits: Arc<std::sync::Mutex<Limits>>,
    mut poll_rx: mpsc::UnboundedReceiver<()>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut last_snapshot: Option<String> = None;
    loop {
        let (t, ti) = (tmux.clone(), tmux_ide.clone());
        let (panes, fleet) = tokio::task::spawn_blocking(move || (t.panes(), ti.team())).await?;
        {
            let mut reg = registry.lock().await;
            let now = Utc::now();
            match panes {
                Ok(p) => reg.apply_panes(&p, now),
                Err(e) => tracing::warn!(error = %e, "tmux list-panes failed"),
            }
            match fleet {
                Ok(f) => reg.apply_fleet(&f, now),
                Err(e) => tracing::debug!(error = %e, "tmux-ide team --json failed"),
            }
            reg.sweep(now, 300);
            let snap = reg.snapshot();
            metrics.sessions_discovered.record(snap.len() as u64, &[]);
            for (status, n) in reg.counts() {
                metrics.sessions_by_state.record(
                    n as u64,
                    &[KeyValue::new(
                        attr::SESSION_STATE,
                        format!("{status:?}").to_lowercase(),
                    )],
                );
            }
            let key = serde_json::to_string(&snap).unwrap_or_default();
            if last_snapshot.as_deref() != Some(key.as_str()) {
                last_snapshot = Some(key);
                let _ = out.send(CompanionMessage::Sessions { sessions: snap });
            }
        }
        let secs = limits.lock().unwrap().team_poll_interval_secs.max(1);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
            _ = poll_rx.recv() => {}
            _ = shutdown.changed() => return Ok(()),
        }
    }
}
