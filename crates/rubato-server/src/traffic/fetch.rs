//! The fetch task: one fetch per interval, validated against the limits, swapped into each
//! workspace it feeds in `AppState::traffic`, its gauges recorded, and each workspace's browsers
//! told only when its derived summary changed (design §4, #131, ADR 0075).

use super::source::{Lookback, Source, SourceError};
use crate::kernel::state::{App, TrafficLatest};
use crate::kernel::tenancy::WsKey;
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{Finding, ObservedTraffic, SourceStatus, derive_traffic};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::Instrument;

/// A fetch that has not answered by then has failed; the previous report stays. The task uses
/// the fetch interval less a second instead, so a fetch never overlaps the next.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// The lookback runs on the first tick and then on every tick this many later (ADR 0083).
pub const LOOKBACK_EVERY: u64 = 10;

/// How one fetch ended, as counted on `rubato.traffic.fetches`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// Rejected whole, with the reason: `too_many_edges`, `bad_window`, `name_too_long`, `parse`.
    Rejected(&'static str),
    Failed,
    /// A newer task feeds every workspace this one fed (ADR 0075): nothing was written.
    Superseded,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Rejected(_) => "rejected",
            Outcome::Failed => "failed",
            Outcome::Superseded => "superseded",
        }
    }
}

/// Which workspaces a fetcher feeds: every workspace held (the development file, which is a
/// process switch), or one workspace's own source (ADR 0075).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Targets {
    All,
    One(WsKey),
}

/// The generation of the task that may write each workspace's traffic now (ADR 0075). The
/// supervisor bumps it before it starts a task; a task whose generation is no longer current
/// writes nothing, so a cancelled task's report never lands after its replacement starts.
#[derive(Default)]
pub struct Generations(Mutex<HashMap<String, u64>>);

impl Generations {
    /// A new generation for the task under `ws` (its [`key`]), superseding every earlier one.
    pub fn bump(&self, ws: &str) -> u64 {
        let mut g = self.0.lock().unwrap();
        let n = g.entry(ws.to_owned()).or_default();
        *n += 1;
        *n
    }

    /// The current generation for `ws`, if a task was ever started for it.
    pub fn of(&self, ws: &str) -> Option<u64> {
        self.0.lock().unwrap().get(ws).copied()
    }

    fn current(&self, ws: &str, generation: u64) -> bool {
        self.0.lock().unwrap().get(ws).copied() == Some(generation)
    }
}

/// One source and what the task remembers between fetches: when it started (the report's age
/// before the first good one) and, per workspace, the last summary its browsers were told about.
pub struct Fetcher<S> {
    source: S,
    started: DateTime<Utc>,
    told: BTreeMap<WsKey, ObservedTraffic>,
    timeout: Duration,
    targets: Targets,
    fetch_secs: u32,
    /// The generation this fetcher writes under, per workspace it feeds; none in tests that
    /// drive a fetcher by hand.
    generation: Option<(Arc<Generations>, u64)>,
    /// Ticks so far: the lookback runs when this is a multiple of `LOOKBACK_EVERY`.
    ticks: u64,
    /// The last lookback that answered, carried by every report until the next (ADR 0083).
    lookback: Option<Lookback>,
}

fn set_error(latest: &mut TrafficLatest, error: Option<String>) {
    if let SourceStatus::Configured { error: e, .. } = &mut latest.source {
        *e = error;
    }
}

impl<S: Source> Fetcher<S> {
    /// A fetcher feeding every workspace, as the development file does.
    pub fn new(source: S, now: DateTime<Utc>) -> Self {
        Self {
            source,
            started: now,
            told: BTreeMap::new(),
            timeout: FETCH_TIMEOUT,
            targets: Targets::All,
            fetch_secs: crate::kernel::config::TRAFFIC_FETCH_SECS.1,
            generation: None,
            ticks: 0,
            lookback: None,
        }
    }

    /// The same fetcher, giving a fetch `timeout` before it counts as failed.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The same fetcher, feeding `targets` every `fetch_secs`, writing only while `generation`
    /// is current (the supervisor's tasks).
    pub fn feeding(
        mut self,
        targets: Targets,
        fetch_secs: u32,
        generation: Option<(Arc<Generations>, u64)>,
    ) -> Self {
        self.targets = targets;
        self.fetch_secs = fetch_secs;
        self.generation = generation;
        self
    }

    /// The workspaces this fetcher may write now.
    fn live_targets(&self, app: &App) -> Vec<WsKey> {
        let ids: Vec<WsKey> = match &self.targets {
            Targets::All => app
                .workspace_entries()
                .0
                .into_iter()
                .map(|w| w.key())
                .collect(),
            Targets::One(ws) => vec![ws.clone()],
        };
        match &self.generation {
            None => ids,
            Some((gens, n)) => ids
                .into_iter()
                .filter(|ws| gens.current(&key(&self.targets, ws), *n))
                .collect(),
        }
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
        let (outcome, targets) = {
            let mut all = app.traffic.write().await;
            // Checked under the lock the write takes: a task superseded meanwhile writes nothing.
            let targets = self.live_targets(app);
            let Some(first) = targets.first() else {
                return Outcome::Superseded;
            };
            let _enter = span.enter();
            let mut latest = all.get(first).cloned().unwrap_or(TrafficLatest {
                report: None,
                source: SourceStatus::Configured {
                    fetch_secs: self.fetch_secs,
                    error: None,
                },
            });
            let was_failing = matches!(
                latest.source,
                SourceStatus::Configured { error: Some(_), .. }
            );
            let outcome = match fetched {
                Ok(report) => {
                    // Every report carries the last lookback's answer (ADR 0083).
                    let report = with_lookback(self.lookback.as_ref(), report);
                    match report.validate() {
                        Ok(()) => {
                            span.record("edges", report.edges.len());
                            latest.report = Some(report);
                            Outcome::Ok
                        }
                        Err(r) => Outcome::Rejected(r.as_str()),
                    }
                }
                Err(SourceError::Parse(e)) => {
                    span.record("error", e.as_str());
                    Outcome::Rejected("parse")
                }
                Err(SourceError::Failed(e)) => {
                    span.record("error", e.as_str());
                    if !was_failing {
                        tracing::warn!(error = %e, rubato.workspace = first.ws(), "traffic source failing");
                    }
                    set_error(&mut latest, Some(e));
                    Outcome::Failed
                }
            };
            if let Outcome::Rejected(reason) = outcome {
                tracing::warn!(
                    rubato.reason = reason,
                    rubato.workspace = first.ws(),
                    "traffic report rejected"
                );
            }
            // The source answered: it is not failing, whatever it answered.
            if outcome != Outcome::Failed {
                set_error(&mut latest, None);
                if was_failing {
                    tracing::info!(rubato.workspace = first.ws(), "traffic source recovered");
                }
            }
            span.record(attr::OUTCOME, outcome.as_str());
            for ws in &targets {
                all.insert(ws.clone(), latest.clone());
            }
            (outcome, targets)
        };
        let mut attrs = vec![
            KeyValue::new(attr::OUTCOME, outcome.as_str()),
            KeyValue::new(attr::TRAFFIC_SOURCE, self.source.name()),
        ];
        if let Targets::One(ws) = &self.targets {
            attrs.extend(ws_attrs(ws));
        }
        app.metrics.traffic_fetches.add(1, &attrs);
        self.record(app, &targets, now).await;
        // After the window's report is written, never before or inside its fetch: a slow or
        // failing lookback cannot make the window late or failing (ADR 0083).
        let due = self.ticks.is_multiple_of(LOOKBACK_EVERY);
        self.ticks += 1;
        if due && outcome != Outcome::Superseded {
            self.look_back(app, &attrs, now).await;
        }
        outcome
    }

    /// Ask the lookback once, under the fetch's timeout. On an answer, keep it and put it on the
    /// stored report of each workspace still fed; on a failure keep the previous one, or none.
    async fn look_back(&mut self, app: &App, attrs: &[KeyValue], now: DateTime<Utc>) {
        let asked = tokio::time::timeout(self.timeout, self.source.lookback()).await;
        let (outcome, error) = match asked {
            Ok(Ok(Some(l))) => {
                self.lookback = Some(l);
                ("ok", None)
            }
            // The source has no lookback: nothing to count or say.
            Ok(Ok(None)) => return,
            Ok(Err(e)) => ("failed", Some(e.to_string())),
            Err(_) => ("timeout", Some("traffic lookback timed out".to_owned())),
        };
        let mut a: Vec<KeyValue> = attrs
            .iter()
            .filter(|k| k.key.as_str() != attr::OUTCOME)
            .cloned()
            .collect();
        a.push(KeyValue::new(attr::OUTCOME, outcome));
        app.metrics.traffic_lookback.add(1, &a);
        let seen = self.lookback.as_ref().map_or(0, |l| l.seen.len());
        match error {
            Some(e) => {
                tracing::warn!(rubato.outcome = outcome, rubato.traffic.seen = seen, error = %e, "traffic lookback")
            }
            None => tracing::info!(
                rubato.outcome = outcome,
                rubato.traffic.seen = seen,
                "traffic lookback"
            ),
        }
        if outcome != "ok" {
            return;
        }
        let targets = {
            let mut all = app.traffic.write().await;
            let targets = self.live_targets(app);
            for ws in &targets {
                let Some(latest) = all.get_mut(ws) else {
                    continue;
                };
                let Some(report) = latest.report.take() else {
                    continue;
                };
                let with = with_lookback(self.lookback.as_ref(), report.clone());
                // A lookback that would make the report invalid is not put on it.
                latest.report = Some(if with.validate().is_ok() {
                    with
                } else {
                    report
                });
            }
            targets
        };
        self.record(app, &targets, now).await;
    }

    /// Gauges from the derivation each workspace's overview would make now, and
    /// `notify_ws("traffic", ws)` for each workspace whose summary, its report time aside, differs
    /// from the last one it was told about (#131; ADR 0049: a change of one map is scoped to it).
    async fn record(&mut self, app: &App, targets: &[WsKey], now: DateTime<Utc>) {
        let mut changed = Vec::new();
        for ws in targets {
            let latest = app.traffic_of(ws).await;
            let w = ws_attrs(ws);
            let since = latest
                .report
                .as_ref()
                .map_or(self.started, |r| r.observed_at);
            let age = (now - since).num_seconds().max(0);
            app.metrics
                .traffic_report_age
                .record(u64::try_from(age).unwrap_or(0), &w);
            let Some(m) = app.map_of(ws).await else {
                continue;
            };
            // Judged against the workspace's map and its dismissals (ADR 0039, 0060), and each
            // workspace told on its own. A dismissal also notifies its workspace from the handler
            // (`dismiss.rs`); the next tick tells it once more, since `dismissed` is in the summary.
            let dismissed = super::dismiss::list(&app.db, ws).await.unwrap_or_else(|e| {
                tracing::warn!(error = %e, rubato.workspace = ws.ws(), "traffic dismissals unreadable");
                Vec::new()
            });
            let (t, f) = derive_traffic(
                &m.map,
                latest.report.as_ref(),
                &latest.source,
                &dismissed,
                now,
            );
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
        self.told.retain(|ws, _| targets.contains(ws));
        for ws in changed {
            app.notify_ws("traffic", &ws);
        }
    }
}

/// The report with the last lookback's answer on it (ADR 0083), only what is longer than its
/// window. Without a lookback, or with one no longer than the window, it claims nothing.
pub fn with_lookback(
    lookback: Option<&Lookback>,
    mut report: rubato_protocol::TrafficReport,
) -> rubato_protocol::TrafficReport {
    report.seen.clear();
    report.lookback_secs = None;
    if let Some(l) = lookback.filter(|l| l.lookback_secs > report.window_secs) {
        report.seen = l
            .seen
            .iter()
            .filter(|e| e.within_secs > report.window_secs)
            .cloned()
            .collect();
        report.lookback_secs = Some(l.lookback_secs);
    }
    report
}

/// `rubato.workspace` for any workspace but `default`, whose series stay as they were before
/// workspaces (ADR 0047), and `rubato.tenant` for any tenant but `default`, whose series stay as
/// they were before tenants (ADR 0087).
pub fn ws_attrs(ws: &WsKey) -> Vec<KeyValue> {
    let mut out = Vec::new();
    let first_tenant = ws.tenant.as_str() == crate::kernel::tenancy::DEFAULT_TENANT;
    if !first_tenant {
        out.push(KeyValue::new(attr::TENANT, ws.tenant.to_string()));
    }
    if !first_tenant || ws.ws() != rubato_protocol::DEFAULT_WORKSPACE {
        out.push(KeyValue::new(attr::WORKSPACE, ws.ws().to_owned()));
    }
    out
}

/// The key a task's generation is kept under: the workspace for its own source, one shared key
/// for the development file that feeds them all.
pub fn key(targets: &Targets, ws: &WsKey) -> String {
    match targets {
        Targets::All => "*".into(),
        Targets::One(_) => ws.to_string(),
    }
}
