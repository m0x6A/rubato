//! The fetch task: one fetch per interval, validated against the limits, swapped into
//! `AppState::traffic`, its gauges recorded, and each workspace's browsers told only when its
//! derived summary changed (design §4, #131).

use super::prometheus::{PrometheusConfig, PrometheusSource};
use super::source::{ConfiguredSource, DevFileSource, Source, SourceError};
use crate::kernel::state::{App, TrafficLatest};
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{Finding, ObservedTraffic, SourceStatus, derive_traffic};
use std::time::Duration;
use tokio::sync::watch;
use tracing::Instrument;

/// A fetch that has not answered by then has failed; the previous report stays. The task uses
/// the fetch interval less a second instead, so a fetch never overlaps the next.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// How one fetch ended, as counted on `rubato.traffic.fetches`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// Rejected whole, with the reason: `too_many_edges`, `bad_window`, `name_too_long`, `parse`.
    Rejected(&'static str),
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Rejected(_) => "rejected",
            Outcome::Failed => "failed",
        }
    }
}

/// One source and what the task remembers between fetches: when it started (the report's age
/// before the first good one) and, per workspace, the last summary its browsers were told about.
pub struct Fetcher<S> {
    source: S,
    started: DateTime<Utc>,
    told: std::collections::BTreeMap<String, ObservedTraffic>,
    timeout: Duration,
}

fn set_error(latest: &mut TrafficLatest, error: Option<String>) {
    if let SourceStatus::Configured { error: e, .. } = &mut latest.source {
        *e = error;
    }
}

impl<S: Source> Fetcher<S> {
    pub fn new(source: S, now: DateTime<Utc>) -> Self {
        Self {
            source,
            started: now,
            told: std::collections::BTreeMap::new(),
            timeout: FETCH_TIMEOUT,
        }
    }

    /// The same fetcher, giving a fetch `timeout` before it counts as failed.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Fetch once, keep or reject the report, record the gauges and notify on a change.
    pub async fn tick(&mut self, app: &App, now: DateTime<Utc>) -> Outcome {
        let span = tracing::info_span!(
            span::TRAFFIC_FETCH,
            rubato.traffic.source = self.source.name(),
            rubato.outcome = tracing::field::Empty,
            edges = tracing::field::Empty,
            error = tracing::field::Empty,
        );
        let fetched = tokio::time::timeout(self.timeout, self.source.fetch())
            .instrument(span.clone())
            .await
            .unwrap_or_else(|_| Err(SourceError::Failed("traffic source timed out".into())));
        let outcome = {
            let mut latest = app.traffic.write().await;
            let _enter = span.enter();
            let was_failing = matches!(
                latest.source,
                SourceStatus::Configured { error: Some(_), .. }
            );
            let outcome = match fetched {
                Ok(report) => match report.validate() {
                    Ok(()) => {
                        span.record("edges", report.edges.len());
                        latest.report = Some(report);
                        Outcome::Ok
                    }
                    Err(r) => Outcome::Rejected(r.as_str()),
                },
                Err(SourceError::Parse(e)) => {
                    span.record("error", e.as_str());
                    Outcome::Rejected("parse")
                }
                Err(SourceError::Failed(e)) => {
                    span.record("error", e.as_str());
                    if !was_failing {
                        tracing::warn!(error = %e, "traffic source failing");
                    }
                    set_error(&mut latest, Some(e));
                    Outcome::Failed
                }
            };
            if let Outcome::Rejected(reason) = outcome {
                tracing::warn!(rubato.reason = reason, "traffic report rejected");
            }
            // The source answered: it is not failing, whatever it answered.
            if outcome != Outcome::Failed {
                set_error(&mut latest, None);
                if was_failing {
                    tracing::info!("traffic source recovered");
                }
            }
            span.record(attr::OUTCOME, outcome.as_str());
            outcome
        };
        app.metrics.traffic_fetches.add(
            1,
            &[
                KeyValue::new(attr::OUTCOME, outcome.as_str()),
                KeyValue::new(attr::TRAFFIC_SOURCE, self.source.name()),
            ],
        );
        self.record(app, now).await;
        outcome
    }

    /// Gauges from the derivation each workspace's overview would make now, and
    /// `notify_ws("traffic", ws)` for each workspace whose summary, its report time aside, differs
    /// from the last one it was told about (#131; ADR 0049: a change of one map is scoped to it).
    async fn record(&mut self, app: &App, now: DateTime<Utc>) {
        let latest = app.traffic.read().await.clone();
        let since = latest
            .report
            .as_ref()
            .map_or(self.started, |r| r.observed_at);
        let age = (now - since).num_seconds().max(0);
        app.metrics
            .traffic_report_age
            .record(u64::try_from(age).unwrap_or(0), &[]);
        // Judged against each workspace's map and its dismissals (ADR 0039, 0060), and each
        // workspace told on its own. A dismissal also notifies its workspace from the handler
        // (`dismiss.rs`); the next tick tells it once more, since `dismissed` is in the summary.
        let maps: Vec<(String, std::sync::Arc<crate::kernel::state::LoadedMap>)> = app
            .maps
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut changed = Vec::new();
        for (ws, m) in &maps {
            let dismissed = super::dismiss::list(&app.db, ws).await.unwrap_or_else(|e| {
                tracing::warn!(error = %e, rubato.workspace = ws.as_str(), "traffic dismissals unreadable");
                Vec::new()
            });
            let (t, f) = derive_traffic(
                &m.map,
                latest.report.as_ref(),
                &latest.source,
                &dismissed,
                now,
            );
            let is_default = ws == rubato_protocol::DEFAULT_WORKSPACE;
            let w: Vec<KeyValue> = if is_default {
                vec![]
            } else {
                vec![KeyValue::new(attr::WORKSPACE, ws.clone())]
            };
            let quiet = f
                .iter()
                .filter(|f| matches!(f, Finding::SeamQuiet { .. }))
                .count();
            app.metrics.traffic_seams_quiet.record(quiet as u64, &w);
            app.metrics
                .traffic_undeclared_edges
                .record(t.undeclared.len() as u64, &w);
            app.metrics
                .traffic_unmapped_services
                .record(u64::from(t.unmapped_services), &w);
            app.metrics
                .traffic_ghost_parts
                .record(t.ghosts.len() as u64, &w);
            app.metrics
                .traffic_dismissals
                .record(t.dismissed.len() as u64, &w);
            let summary = ObservedTraffic {
                observed_at: None,
                ..t
            };
            if self.told.get(ws) != Some(&summary) {
                self.told.insert(ws.clone(), summary);
                changed.push(ws.clone());
            }
        }
        // Defensive: no code path removes a loaded map today.
        self.told.retain(|ws, _| maps.iter().any(|(m, _)| m == ws));
        for ws in changed {
            app.notify_ws("traffic", &ws);
        }
    }
}

/// The traffic task: runs only when a source is configured. Best-effort: if it ends, the
/// report goes stale and the map says *unknown*.
pub async fn run(app: App, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
    let fetch_secs = app.config.traffic_fetch_secs;
    let source = if let Some(path) = app.config.dev_traffic.clone() {
        tracing::warn!(
            "RUBATO_DEV_TRAFFIC is set: observed traffic comes from that file; development and tests only"
        );
        ConfiguredSource::DevFile(DevFileSource::new(path))
    } else {
        match PrometheusConfig::from_env(|k| app.config.traffic_env.get(k), fetch_secs) {
            Ok(Some(c)) => {
                tracing::info!(url = %c.url, window_secs = c.window_secs, "traffic from the service graph");
                ConfiguredSource::Prometheus(PrometheusSource::new(c))
            }
            Ok(None) => return Ok(()),
            Err(e) => {
                // A source set up wrong is a failing source, shown as unknown, never as quiet.
                tracing::error!(error = %e, "traffic source failing");
                set_error(&mut *app.traffic.write().await, Some(e));
                return Ok(());
            }
        }
    };
    let mut fetcher = Fetcher::new(source, Utc::now()).with_timeout(Duration::from_secs(
        u64::from(fetch_secs.saturating_sub(1).max(1)),
    ));
    let secs = u64::from(fetch_secs);
    loop {
        fetcher.tick(&app, Utc::now()).await;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
            _ = shutdown.changed() => return Ok(()),
        }
    }
}
