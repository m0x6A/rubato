//! The fetch task: one fetch per interval, validated against the limits, swapped into
//! `AppState::traffic`, its gauges recorded, and the browsers told only when the derived
//! summary changed (design §4).

use super::source::{DevFileSource, Source, SourceError};
use crate::kernel::state::{App, TrafficLatest};
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{Finding, ObservedTraffic, SourceStatus, derive_traffic};
use std::time::Duration;
use tokio::sync::watch;
use tracing::Instrument;

/// A fetch that has not answered by then has failed; the previous report stays.
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
/// before the first good one) and the last summary the browsers were told about.
pub struct Fetcher<S> {
    source: S,
    started: DateTime<Utc>,
    told: Option<ObservedTraffic>,
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
            told: None,
        }
    }

    /// Fetch once, keep or reject the report, record the gauges and notify on a change.
    pub async fn tick(&mut self, app: &App, now: DateTime<Utc>) -> Outcome {
        let span = tracing::info_span!(
            span::TRAFFIC_FETCH,
            rubato.outcome = tracing::field::Empty,
            edges = tracing::field::Empty,
            error = tracing::field::Empty,
        );
        let fetched = tokio::time::timeout(FETCH_TIMEOUT, self.source.fetch())
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
        app.metrics
            .traffic_fetches
            .add(1, &[KeyValue::new(attr::OUTCOME, outcome.as_str())]);
        self.record(app, now).await;
        outcome
    }

    /// Gauges from the derivation the overview would make now, and `notify("traffic")` only
    /// when the summary, its report time aside, differs from the last one sent.
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
        // Judged against each workspace's map (ADR 0039 per workspace); the browsers are told
        // once, from `default`'s, which unprefixed overviews show.
        let maps: Vec<(String, crate::kernel::state::LoadedMap)> = app
            .maps
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (ws, m) in maps
            .iter()
            .filter(|(w, _)| w != rubato_protocol::DEFAULT_WORKSPACE)
        {
            let (t, f) = derive_traffic(&m.map, latest.report.as_ref(), &latest.source, now);
            let w = [KeyValue::new(
                rubato_protocol::otel::attr::WORKSPACE,
                ws.clone(),
            )];
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
        }
        let Some(map) = maps
            .iter()
            .find(|(w, _)| w == rubato_protocol::DEFAULT_WORKSPACE)
            .map(|(_, m)| m.clone())
        else {
            return;
        };
        let (traffic, findings) =
            derive_traffic(&map.map, latest.report.as_ref(), &latest.source, now);
        let quiet = findings
            .iter()
            .filter(|f| matches!(f, Finding::SeamQuiet { .. }))
            .count();
        app.metrics.traffic_seams_quiet.record(quiet as u64, &[]);
        app.metrics
            .traffic_undeclared_edges
            .record(traffic.undeclared.len() as u64, &[]);
        app.metrics
            .traffic_unmapped_services
            .record(u64::from(traffic.unmapped_services), &[]);
        let summary = ObservedTraffic {
            observed_at: None,
            ..traffic
        };
        if self.told.as_ref() != Some(&summary) {
            self.told = Some(summary);
            app.notify("traffic");
        }
    }
}

/// The traffic task: runs only when a source is configured. Best-effort: if it ends, the
/// report goes stale and the map says *unknown*.
pub async fn run(app: App, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
    let Some(path) = app.config.dev_traffic.clone() else {
        return Ok(());
    };
    tracing::warn!(
        "RUBATO_DEV_TRAFFIC is set: observed traffic comes from that file; development and tests only"
    );
    let mut fetcher = Fetcher::new(DevFileSource::new(path), Utc::now());
    let secs = u64::from(app.config.traffic_fetch_secs);
    loop {
        fetcher.tick(&app, Utc::now()).await;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
            _ = shutdown.changed() => return Ok(()),
        }
    }
}
