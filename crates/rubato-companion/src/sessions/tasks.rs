//! Discovery tasks: follow tmux-ide events (best-effort) and reconcile pane metadata with
//! `tmux-ide team --json` on a timer and whenever woken.

use super::Registry;
use crate::kernel::telemetry::Metrics;
use crate::kernel::tmux::Tmux;
use crate::kernel::tmux_ide::{Fleet, TmuxIde};
use anyhow::Result;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{CompanionMessage, Limits, StateSource};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, watch};
use tracing::Instrument;

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
    let mut team_poll = TeamPoll::default();
    loop {
        let t = tmux.clone();
        let panes = tokio::task::spawn_blocking(move || t.panes()).await?;
        let fleet = team_poll.poll(&tmux_ide).await?;
        {
            let mut reg = registry.lock().await;
            let now = Utc::now();
            match panes {
                Ok(p) => reg.apply_panes(&p, now),
                Err(e) => tracing::warn!(error = %e, "tmux list-panes failed"),
            }
            if let Some(f) = fleet {
                reg.apply_fleet(&f, now);
            }
            reg.sweep(now, 300);
            let snap = reg.snapshot();
            record_gauges(&reg, &metrics);
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

/// `tmux-ide team --json`, one poll at a time. Remembers whether the last poll failed, so a
/// broken tmux-ide is one `warn!` and one `info!` on recovery, not a line per poll.
#[derive(Default)]
struct TeamPoll {
    failing: bool,
}

impl TeamPoll {
    /// One poll inside a `rubato.team_poll` span; a failure sets the span's error status.
    /// `None` when the poll failed: discovery then runs on panes and hooks alone.
    async fn poll(&mut self, tmux_ide: &TmuxIde) -> Result<Option<Fleet>> {
        let span = tracing::info_span!(
            span::TEAM_POLL,
            otel.status_code = tracing::field::Empty,
            otel.status_description = tracing::field::Empty,
        );
        let ti = tmux_ide.clone();
        let fleet = tokio::task::spawn_blocking(move || ti.team())
            .instrument(span.clone())
            .await?;
        let _in_span = span.enter();
        match fleet {
            Ok(f) => {
                if std::mem::take(&mut self.failing) {
                    tracing::info!("tmux-ide team --json recovered");
                }
                Ok(Some(f))
            }
            Err(e) => {
                span.record("otel.status_code", "ERROR");
                span.record("otel.status_description", format!("{e:#}"));
                if !std::mem::replace(&mut self.failing, true) {
                    tracing::warn!(error = %e, "tmux-ide team --json failed");
                }
                Ok(None)
            }
        }
    }
}

/// The source as the protocol spells it on the wire (snake_case).
fn source_name(source: StateSource) -> &'static str {
    match source {
        StateSource::Hook => "hook",
        StateSource::TmuxIde => "tmux_ide",
        StateSource::Launch => "launch",
        StateSource::Scraped => "scraped",
    }
}

/// `rubato.sessions.discovered` per source and `rubato.sessions.by_state` per status, each with
/// this companion's id and each including its zeros.
fn record_gauges(reg: &Registry, metrics: &Metrics) {
    let companion = reg.companion_id.to_string();
    for (source, n) in reg.counts_by_source() {
        metrics.sessions_discovered.record(
            n as u64,
            &[
                KeyValue::new(attr::SESSION_SOURCE, source_name(source)),
                KeyValue::new(attr::COMPANION_ID, companion.clone()),
            ],
        );
    }
    for (status, n) in reg.counts() {
        metrics.sessions_by_state.record(
            n as u64,
            &[
                KeyValue::new(attr::SESSION_STATE, format!("{status:?}").to_lowercase()),
                KeyValue::new(attr::COMPANION_ID, companion.clone()),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::{capture, metric_reader};
    use rubato_protocol::HookEvent;
    use rubato_protocol::otel::metric;
    use std::os::unix::fs::PermissionsExt;
    use uuid::Uuid;

    #[test]
    fn session_gauges_carry_the_companion_id() {
        let id = Uuid::new_v4();
        let mut reg = Registry::new(id, "m0x6A");
        let (reader, metrics) = metric_reader::start();
        let hook = |name: &str| {
            HookEvent::from_payload(
                serde_json::json!({"session_id": "s1", "hook_event_name": name, "cwd": "/x"}),
                Utc::now(),
            )
            .unwrap()
        };
        reg.apply_hook(&hook("PreToolUse"));
        record_gauges(&reg, &metrics);

        let by_state = reader.points(metric::SESSIONS_BY_STATE);
        assert_eq!(by_state.len(), 5, "one point per status: {by_state:?}");
        let discovered = reader.points(metric::SESSIONS_DISCOVERED);
        assert_eq!(discovered.len(), 4, "one point per source: {discovered:?}");
        for p in by_state.iter().chain(&discovered) {
            assert_eq!(
                p.attrs.get(attr::COMPANION_ID),
                Some(&id.to_string()),
                "{p:?}"
            );
        }
        let state = |points: &[metric_reader::Point], s: &str| {
            points
                .iter()
                .find(|p| p.attrs.get(attr::SESSION_STATE).map(String::as_str) == Some(s))
                .map(|p| p.value)
        };
        assert_eq!(state(&by_state, "working"), Some(1));
        assert_eq!(state(&by_state, "idle"), Some(0));
        let sources: std::collections::BTreeMap<&str, u64> = discovered
            .iter()
            .map(|p| (p.attrs[attr::SESSION_SOURCE].as_str(), p.value))
            .collect();
        assert_eq!(
            sources,
            [("hook", 1), ("launch", 0), ("scraped", 0), ("tmux_ide", 0)].into()
        );

        // The turn ends: working goes back to 0 instead of keeping its last value.
        reg.apply_hook(&hook("Stop"));
        record_gauges(&reg, &metrics);
        let by_state = reader.points(metric::SESSIONS_BY_STATE);
        assert_eq!(state(&by_state, "working"), Some(0));
        assert_eq!(state(&by_state, "idle"), Some(1));
    }

    #[test]
    fn source_names_are_the_protocol_names() {
        for source in [
            StateSource::Hook,
            StateSource::TmuxIde,
            StateSource::Launch,
            StateSource::Scraped,
        ] {
            assert_eq!(
                serde_json::to_value(source).unwrap(),
                serde_json::json!(source_name(source))
            );
        }
    }

    /// A `tmux-ide` stand-in: a shell script with the given body.
    fn stand_in(dir: &std::path::Path, name: &str, body: &str) -> TmuxIde {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        TmuxIde::new(path.to_string_lossy().into_owned(), Tmux::new("tmux", None))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failing_team_poll_warns_once() {
        let (events, _g) = capture::start();
        let tmp = tempfile::tempdir().unwrap();
        let broken = stand_in(tmp.path(), "broken", "echo no tmux-ide here >&2; exit 1");
        let working = stand_in(tmp.path(), "working", r#"echo '{"projects":[]}'"#);
        let count = |level: tracing::Level, message: &str| {
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.level == level && e.message == message)
                .count()
        };
        let mut poll = TeamPoll::default();

        assert!(poll.poll(&broken).await.unwrap().is_none());
        assert!(poll.poll(&broken).await.unwrap().is_none());
        assert_eq!(
            count(tracing::Level::WARN, "tmux-ide team --json failed"),
            1
        );
        let warned = events.lock().unwrap()[0].clone();
        assert!(
            warned.fields["error"].contains("no tmux-ide here"),
            "{warned:?}"
        );

        assert_eq!(poll.poll(&working).await.unwrap(), Some(Fleet::default()));
        assert_eq!(poll.poll(&working).await.unwrap(), Some(Fleet::default()));
        assert_eq!(
            count(tracing::Level::INFO, "tmux-ide team --json recovered"),
            1
        );

        // A second outage is reported again.
        assert!(poll.poll(&broken).await.unwrap().is_none());
        assert_eq!(
            count(tracing::Level::WARN, "tmux-ide team --json failed"),
            2
        );
        assert_eq!(events.lock().unwrap().len(), 3);
    }
}
