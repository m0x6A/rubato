//! Discovery: reconcile pane metadata from `tmux list-panes` (and `ps` for wrapped agents) with
//! the registry, on a timer and whenever a launch wakes it. State comes from hooks only; tmux-ide's
//! `team --json` and `events --follow` are gone (ADR 0070).

use super::Registry;
use crate::kernel::telemetry::Metrics;
use crate::kernel::tmux::Tmux;
use anyhow::Result;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{CompanionMessage, Limits, SessionState};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, watch};
use tracing::Instrument;

/// Panes → registry → snapshot; the snapshot goes to the server only when it changed. Each cycle
/// runs in a `rubato.reconcile` span with the companion id and the agent panes it found, and an
/// error status when `tmux list-panes` fails.
pub async fn reconcile(
    out: mpsc::UnboundedSender<CompanionMessage>,
    registry: Arc<Mutex<Registry>>,
    metrics: Arc<Metrics>,
    tmux: Tmux,
    mut limits: watch::Receiver<Limits>,
    mut poll_rx: mpsc::UnboundedReceiver<()>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut last_snapshot: Option<Vec<SessionState>> = None;
    let companion = registry.lock().await.companion_id;
    loop {
        let span = tracing::info_span!(
            span::RECONCILE,
            rubato.companion_id = %companion,
            rubato.reconcile.panes = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
            otel.status_description = tracing::field::Empty,
        );
        cycle(&out, &registry, &metrics, &tmux, &mut last_snapshot, &span)
            .instrument(span.clone())
            .await?;
        let secs = limits.borrow_and_update().team_poll_interval_secs.max(1);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
            // A `Welcome` with new limits: poll now and wait the new interval from here.
            Ok(()) = limits.changed() => {}
            _ = poll_rx.recv() => {}
            _ = shutdown.changed() => return Ok(()),
        }
    }
}

/// One reconcile cycle. A failing `tmux list-panes` keeps every entry (plan 40 N7) and marks the
/// span as an error; the snapshot still goes out if hooks changed it.
async fn cycle(
    out: &mpsc::UnboundedSender<CompanionMessage>,
    registry: &Mutex<Registry>,
    metrics: &Metrics,
    tmux: &Tmux,
    last_snapshot: &mut Option<Vec<SessionState>>,
    span: &tracing::Span,
) -> Result<()> {
    let t = tmux.clone();
    let panes = tokio::task::spawn_blocking(move || t.panes()).await?;
    let wrapped = match &panes {
        Ok(p) => wrapped_agents(p).await,
        Err(_) => Default::default(),
    };
    {
        let mut reg = registry.lock().await;
        let now = Utc::now();
        match panes {
            Ok(p) => {
                let agents = p
                    .iter()
                    .filter(|p| {
                        super::discovery::looks_like_agent(&p.current_command)
                            || wrapped.contains(&p.pane_id)
                    })
                    .count();
                span.record("rubato.reconcile.panes", agents as u64);
                reg.apply_panes_with(&p, &wrapped, now);
            }
            Err(e) => {
                span.record("otel.status_code", "ERROR");
                span.record("otel.status_description", format!("{e:#}"));
                tracing::warn!(error = %e, "tmux list-panes failed");
            }
        }
        reg.sweep(now, 300);
    }
    // Identity again where a checkout changed it: git off the lock (design 80 §5).
    let checks = registry.lock().await.identity_checks();
    let found = tokio::task::spawn_blocking(move || crate::sessions::recheck(checks)).await?;
    let snap = {
        let mut reg = registry.lock().await;
        reg.apply_identities(found);
        record_gauges(&reg, metrics);
        reg.snapshot()
    };
    // Compared as structs, off the lock.
    if last_snapshot.as_ref() != Some(&snap) {
        *last_snapshot = Some(snap.clone());
        let _ = out.send(CompanionMessage::Sessions { sessions: snap });
    }
    Ok(())
}

/// Panes whose own command is not Claude Code but whose process tree runs it (a wrapper
/// script). One `ps` when there is such a pane at all; a failed `ps` finds none.
async fn wrapped_agents(panes: &[crate::kernel::tmux::Pane]) -> std::collections::HashSet<String> {
    let others: Vec<(String, u32)> = panes
        .iter()
        .filter(|p| !super::discovery::looks_like_agent(&p.current_command))
        .map(|p| (p.pane_id.clone(), p.pid))
        .collect();
    if others.is_empty() {
        return Default::default();
    }
    let roots: Vec<u32> = others.iter().map(|(_, pid)| *pid).collect();
    match tokio::task::spawn_blocking(crate::kernel::ps::table).await {
        Ok(Ok(table)) => {
            let hit = crate::kernel::ps::roots_running_claude(&table, &roots);
            others
                .into_iter()
                .filter(|(_, pid)| hit.contains(pid))
                .map(|(id, _)| id)
                .collect()
        }
        Ok(Err(e)) => {
            tracing::debug!(error = %format!("{e:#}"), "ps failed; wrapped agents not looked for");
            Default::default()
        }
        Err(_) => Default::default(),
    }
}

/// `rubato.sessions.discovered` per source, with this companion's id and including its zeros.
/// `rubato.sessions.by_state` is the server's, recorded from each snapshot this companion sends
/// (#43), so a sum over it counts each session once.
fn record_gauges(reg: &Registry, metrics: &Metrics) {
    let companion = reg.companion_id.to_string();
    for (source, n) in reg.counts_by_source() {
        metrics.sessions_discovered.record(
            n as u64,
            &[
                KeyValue::new(attr::SESSION_SOURCE, source.name()),
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

        let discovered = reader.points(metric::SESSIONS_DISCOVERED);
        assert_eq!(
            discovered.len(),
            rubato_protocol::StateSource::ALL.len() - 1,
            "one point per source a companion produces (not tmux_ide, ADR 0070): {discovered:?}"
        );
        for p in &discovered {
            assert_eq!(
                p.attrs.get(attr::COMPANION_ID),
                Some(&id.to_string()),
                "{p:?}"
            );
        }
        let sources: std::collections::BTreeMap<&str, u64> = discovered
            .iter()
            .map(|p| (p.attrs[attr::SESSION_SOURCE].as_str(), p.value))
            .collect();
        assert_eq!(
            sources,
            [("headless", 0), ("hook", 1), ("launch", 0), ("scraped", 0)].into()
        );

        // The session moves on: `discovered` follows, and `by_state` is never recorded here.
        // It is the server's, from each snapshot (#43); recording it on both sides made every
        // sum count each session twice.
        reg.apply_hook(&hook("Stop"));
        record_gauges(&reg, &metrics);
        assert_eq!(
            reader.points(metric::SESSIONS_DISCOVERED).len(),
            rubato_protocol::StateSource::ALL.len() - 1
        );
        let by_state = reader.points(metric::SESSIONS_BY_STATE);
        assert!(
            by_state.is_empty(),
            "the server owns this gauge: {by_state:?}"
        );
    }

    /// A stand-in `tmux` that answers `list-sessions` (a server is running) and fails
    /// `list-panes`: a failing `list-sessions` reads as no server and no panes, not an error
    /// (`kernel/tmux.rs`). Written by a child `sh` (ETXTBSY, #217).
    fn panes_fail(dir: &std::path::Path) -> Tmux {
        let script = dir.join("tmux");
        let body =
            "#!/bin/sh\n[ \"$1\" = list-sessions ] && exit 0\necho no panes here >&2\nexit 1\n";
        let ok = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg(body)
            .arg(&script)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        Tmux::new(script.to_string_lossy(), None)
    }

    /// ADR 0070: one `rubato.reconcile` span per cycle, with the companion id, an error status when
    /// `tmux list-panes` fails, and the agent panes found when it works. The span's error status is
    /// what a trace search for failed cycles finds (#41); the OpenTelemetry layer maps
    /// `otel.status_code` to the exported status.
    #[tokio::test(flavor = "current_thread")]
    async fn a_reconcile_cycle_is_a_span_marked_as_an_error_when_tmux_fails() {
        let (spans, _g) = capture::spans();
        let tmp = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let registry = Arc::new(Mutex::new(Registry::new(id, "m0x6A")));
        let (out, mut sent) = mpsc::unbounded_channel();
        let (_limits_tx, limits_rx) = watch::channel(Limits::default());
        let (poll_tx, poll_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(reconcile(
            out,
            registry,
            Arc::new(Metrics::new()),
            panes_fail(tmp.path()),
            limits_rx,
            poll_rx,
            shutdown_rx,
        ));
        // The first cycle sends the (empty) snapshot; a wake-up runs a second.
        let first = tokio::time::timeout(Duration::from_secs(5), sent.recv()).await;
        assert!(
            matches!(first, Ok(Some(CompanionMessage::Sessions { .. }))),
            "{first:?}"
        );
        poll_tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        shutdown_tx.send(true).unwrap();
        task.await.unwrap().unwrap();

        let spans = spans.lock().unwrap();
        let cycles: Vec<_> = spans
            .iter()
            .filter(|s| s.message == span::RECONCILE)
            .collect();
        assert!(cycles.len() >= 2, "one span per cycle: {spans:?}");
        for c in &cycles {
            assert_eq!(
                c.fields.get("rubato.companion_id"),
                Some(&id.to_string()),
                "{c:?}"
            );
            assert_eq!(
                c.fields.get("otel.status_code").map(String::as_str),
                Some("ERROR"),
                "{c:?}"
            );
            assert!(
                c.fields["otel.status_description"].contains("no panes here"),
                "{c:?}"
            );
        }
        println!("checked {} reconcile spans", cycles.len());
    }

    /// With no tmux server at all, a cycle is not an error: there are no panes (plan 40 N7).
    #[tokio::test(flavor = "current_thread")]
    async fn a_cycle_with_no_tmux_server_is_not_an_error() {
        let (spans, _g) = capture::spans();
        let tmp = tempfile::tempdir().unwrap();
        let registry = Arc::new(Mutex::new(Registry::new(Uuid::nil(), "m0x6A")));
        let (out, _sent) = mpsc::unbounded_channel();
        let mut last = None;
        let none = Tmux::new(tmp.path().join("no-tmux").to_string_lossy(), None);
        let span = tracing::info_span!(
            span::RECONCILE,
            rubato.reconcile.panes = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
            otel.status_description = tracing::field::Empty,
        );
        cycle(&out, &registry, &Metrics::new(), &none, &mut last, &span)
            .instrument(span.clone())
            .await
            .unwrap();
        drop(span);
        let spans = spans.lock().unwrap();
        let c = spans
            .iter()
            .find(|s| s.message == span::RECONCILE)
            .expect("the span");
        assert!(!c.fields.contains_key("otel.status_code"), "{c:?}");
        assert_eq!(
            c.fields.get("rubato.reconcile.panes").map(String::as_str),
            Some("0"),
            "{c:?}"
        );
    }
}
