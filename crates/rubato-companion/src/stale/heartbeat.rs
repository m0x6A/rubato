//! The heartbeat the server's trust check and `CompanionSilent` read.

use crate::kernel::telemetry::Metrics;
use crate::kernel::tmux_ide::TmuxIde;
use anyhow::Result;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{CompanionMessage, Heartbeat, Limits};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

pub async fn heartbeat(
    out: mpsc::UnboundedSender<CompanionMessage>,
    companion_id: Uuid,
    user: String,
    metrics: Arc<Metrics>,
    tmux_ide: TmuxIde,
    mut limits: watch::Receiver<Limits>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let hostname = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "unknown".into());
    let tmux_ide_version = tokio::task::spawn_blocking(move || tmux_ide.version().ok())
        .await
        .ok()
        .flatten();
    loop {
        let now = Utc::now();
        let _ = out.send(CompanionMessage::Heartbeat(Heartbeat {
            companion_id,
            user: user.clone(),
            at: now,
            hostname: hostname.clone(),
            companion_version: crate::kernel::VERSION.to_owned(),
            tmux_ide_version: tmux_ide_version.clone(),
            // Named from tmux once tmux-ide is gone (plan 147, PR 2).
            tmux_version: None,
            claude_version: None,
        }));
        metrics.heartbeat.record(
            now.timestamp() as u64,
            &[
                KeyValue::new(attr::COMPANION_ID, companion_id.to_string()),
                KeyValue::new(attr::USER, user.clone()),
            ],
        );
        let secs = limits.borrow_and_update().heartbeat_interval_secs.max(1);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
            // A `Welcome` with new limits: beat now and wait the new interval from here.
            Ok(()) = limits.changed() => {}
            _ = shutdown.changed() => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::tmux::Tmux;

    /// Fails on purpose without the `limits.changed()` arm: the second beat waits an hour.
    #[tokio::test]
    async fn a_new_welcome_wakes_the_heartbeat() {
        let (limits_tx, limits_rx) = watch::channel(Limits {
            heartbeat_interval_secs: 3600,
            ..Limits::default()
        });
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let (out, mut sent) = mpsc::unbounded_channel();
        let tmux_ide = TmuxIde::new("/nonexistent/tmux-ide", Tmux::new("tmux", None));
        let task = tokio::spawn(heartbeat(
            out,
            Uuid::nil(),
            "m0x6A".into(),
            Arc::new(Metrics::new()),
            tmux_ide,
            limits_rx,
            shutdown_rx,
        ));
        let first = tokio::time::timeout(Duration::from_secs(5), sent.recv()).await;
        assert!(matches!(first, Ok(Some(CompanionMessage::Heartbeat(_)))));
        // What `server_link::on_welcome` does with a `Welcome` carrying new limits.
        limits_tx.send_replace(Limits {
            heartbeat_interval_secs: 1,
            ..Limits::default()
        });
        let second = tokio::time::timeout(Duration::from_secs(2), sent.recv()).await;
        assert!(
            matches!(second, Ok(Some(CompanionMessage::Heartbeat(_)))),
            "the new limits wake the heartbeat: {second:?}"
        );
        task.abort();
    }
}
