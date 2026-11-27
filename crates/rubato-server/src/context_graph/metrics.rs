//! The slice's own instruments (ADR 0084). The gauges are observed from what the last tick
//! stored, when the reader collects: a source's age keeps rising while no tick finds it
//! current, also when the timer itself has stopped.

use super::build::Outcome;
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use rubato_graph::Store;
use rubato_protocol::otel::{attr, metric};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

/// What the gauges read: each source's last current check and its size, and the interval.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Observed {
    pub regen_secs: u32,
    /// source → (checked_at, nodes, edges). A source never found current has no time.
    pub sources: BTreeMap<String, (Option<DateTime<Utc>>, u64, u64)>,
}

impl Observed {
    /// Seconds since each source was last found current. One never built counts from when the
    /// server first saw it, so it cannot stay unbuilt in silence.
    pub(super) fn ages(&self, first_seen: DateTime<Utc>, now: DateTime<Utc>) -> Vec<(String, u64)> {
        self.sources
            .iter()
            .map(|(source, (checked, _, _))| {
                let since = checked.unwrap_or(first_seen);
                (source.clone(), (now - since).num_seconds().max(0) as u64)
            })
            .collect()
    }
}

struct Instruments {
    builds: Counter<u64>,
    truncated: Counter<u64>,
    snapshots: Counter<u64>,
    observed: std::sync::Arc<Mutex<Observed>>,
}

fn instruments() -> &'static Instruments {
    static I: OnceLock<Instruments> = OnceLock::new();
    I.get_or_init(|| {
        let m = opentelemetry::global::meter("rubato-server");
        let observed = std::sync::Arc::new(Mutex::new(Observed::default()));
        let started = Utc::now();
        let source = |s: &str| [KeyValue::new(attr::GRAPH_SOURCE, s.to_owned())];
        let o = observed.clone();
        let _age = m
            .u64_observable_gauge(metric::GRAPH_SOURCE_AGE)
            .with_callback(move |g| {
                for (s, age) in o.lock().unwrap().ages(started, Utc::now()) {
                    g.observe(age, &source(&s));
                }
            })
            .build();
        let o = observed.clone();
        let _interval = m
            .u64_observable_gauge(metric::GRAPH_REGEN_INTERVAL)
            .with_callback(move |g| {
                let secs = o.lock().unwrap().regen_secs;
                if secs > 0 {
                    g.observe(u64::from(secs), &[]);
                }
            })
            .build();
        let o = observed.clone();
        let _nodes = m
            .u64_observable_gauge(metric::GRAPH_NODES)
            .with_callback(move |g| {
                for (s, (_, nodes, _)) in &o.lock().unwrap().sources {
                    g.observe(*nodes, &source(s));
                }
            })
            .build();
        let o = observed.clone();
        let _edges = m
            .u64_observable_gauge(metric::GRAPH_EDGES)
            .with_callback(move |g| {
                for (s, (_, _, edges)) in &o.lock().unwrap().sources {
                    g.observe(*edges, &source(s));
                }
            })
            .build();
        Instruments {
            builds: m.u64_counter(metric::GRAPH_BUILDS).build(),
            truncated: m.u64_counter(metric::GRAPH_TRUNCATED).build(),
            snapshots: m.u64_counter(metric::GRAPH_SNAPSHOTS).build(),
            observed,
        }
    })
}

/// One build, by source, outcome and reason.
pub(super) fn build(source: &str, outcome: &Outcome) {
    let mut attrs = vec![
        KeyValue::new(attr::GRAPH_SOURCE, source.to_owned()),
        KeyValue::new(attr::OUTCOME, outcome.as_str()),
    ];
    if !outcome.reason().is_empty() {
        attrs.push(KeyValue::new(attr::DROP_REASON, outcome.reason()));
    }
    instruments().builds.add(1, &attrs);
}

/// One snapshot request, by outcome.
pub(super) fn snapshot(outcome: &'static str) {
    instruments()
        .snapshots
        .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
}

/// A cap cut a source.
pub(super) fn truncated(source: &str, cap: &'static str) {
    instruments().truncated.add(
        1,
        &[
            KeyValue::new(attr::GRAPH_SOURCE, source.to_owned()),
            KeyValue::new(attr::DROP_REASON, cap),
        ],
    );
}

/// What the gauges show from now on: the store's sources as they stand after a tick.
pub(super) async fn observe(store: &Store, regen_secs: u32) {
    let Ok(rows) = store.sources().await else {
        return;
    };
    let next = Observed {
        regen_secs,
        sources: rows
            .into_iter()
            .map(|r| (r.source, (r.checked_at, r.nodes, r.edges)))
            .collect(),
    };
    *instruments().observed.lock().unwrap() = next;
}
