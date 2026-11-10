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
    limits: Arc<std::sync::Mutex<Limits>>,
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
            claude_version: None,
        }));
        metrics.heartbeat.record(
            now.timestamp() as u64,
            &[
                KeyValue::new(attr::COMPANION_ID, companion_id.to_string()),
                KeyValue::new(attr::USER, user.clone()),
            ],
        );
        let secs = limits.lock().unwrap().heartbeat_interval_secs.max(1);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
            _ = shutdown.changed() => return Ok(()),
        }
    }
}
