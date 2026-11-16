//! The heartbeat the server's trust check and `CompanionSilent` read.

use crate::kernel::telemetry::Metrics;
use crate::kernel::tmux::Tmux;
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
    // None on a headless companion, which runs no tmux (ADR 0062).
    tmux: Option<Tmux>,
    mut limits: watch::Receiver<Limits>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let hostname = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "unknown".into());
    // Read once at start, like the tmux-ide version was; the launch checks it each time.
    let tmux_version = match tmux {
        Some(t) => tokio::task::spawn_blocking(move || t.version())
            .await
            .ok()
            .and_then(|v| match v {
                Ok(v) => {
                    tracing::info!(rubato.tmux.version = %v, "tmux found");
                    Some(v)
                }
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "tmux -V failed; launches will be refused");
                    None
                }
            }),
        None => None,
    };
    loop {
        let now = Utc::now();
        let _ = out.send(CompanionMessage::Heartbeat(Heartbeat {
            companion_id,
            user: user.clone(),
            at: now,
            hostname: hostname.clone(),
            companion_version: crate::kernel::VERSION.to_owned(),
            // Never sent since ADR 0070; still read from older companions.
            tmux_ide_version: None,
            tmux_version: tmux_version.clone(),
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

    /// A stand-in `tmux` that prints `out`, written by a child `sh` (ETXTBSY, #217).
    fn standin(dir: &std::path::Path, out: &str) -> Tmux {
        let script = dir.join("tmux");
        let ok = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg(format!("#!/bin/sh\necho '{out}'\n"))
            .arg(&script)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        Tmux::new(script.to_string_lossy(), None)
    }

    async fn first_beat(tmux: Option<Tmux>) -> Heartbeat {
        let (_limits_tx, limits_rx) = watch::channel(Limits::default());
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let (out, mut sent) = mpsc::unbounded_channel();
        let task = tokio::spawn(heartbeat(
            out,
            Uuid::nil(),
            "m0x6A".into(),
            Arc::new(Metrics::new()),
            tmux,
            limits_rx,
            shutdown_rx,
        ));
        let first = tokio::time::timeout(Duration::from_secs(5), sent.recv()).await;
        task.abort();
        match first {
            Ok(Some(CompanionMessage::Heartbeat(h))) => h,
            other => panic!("expected a heartbeat, got {other:?}"),
        }
    }

    /// ADR 0070: the heartbeat names the tmux version and no longer a tmux-ide one.
    #[tokio::test]
    async fn the_heartbeat_names_the_tmux_version() {
        let dir = tempfile::tempdir().unwrap();
        let h = first_beat(Some(standin(dir.path(), "tmux 3.8"))).await;
        assert_eq!(h.tmux_version.as_deref(), Some("3.8"));
        assert_eq!(h.tmux_ide_version, None);
    }

    /// ADR 0062: a headless companion runs no tmux, so its heartbeat names none.
    #[tokio::test]
    async fn a_headless_heartbeat_names_no_tmux() {
        let h = first_beat(None).await;
        assert_eq!(h.tmux_version, None);
        assert_eq!(h.tmux_ide_version, None);
    }

    /// Fails on purpose without the `limits.changed()` arm: the second beat waits an hour.
    #[tokio::test]
    async fn a_new_welcome_wakes_the_heartbeat() {
        let (limits_tx, limits_rx) = watch::channel(Limits {
            heartbeat_interval_secs: 3600,
            ..Limits::default()
        });
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let (out, mut sent) = mpsc::unbounded_channel();
        let task = tokio::spawn(heartbeat(
            out,
            Uuid::nil(),
            "m0x6A".into(),
            Arc::new(Metrics::new()),
            None,
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
