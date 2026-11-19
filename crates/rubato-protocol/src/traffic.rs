//! Observed traffic per seam (ADR 0039): the one report every source fills, what the overview
//! carries, and the pure derivation from the two.
//!
//! Service names come from outside the team: any instrumented service chooses its own. They are
//! text only. An edge end maps to the part whose `service` is exactly the namespace and name it
//! emits (ADR 0059), never by id and never by name alone. An end with a namespace that matches
//! no part is a ghost, drawn on the overlay and never made a part by Rubato; an end with neither
//! is only counted.

use crate::ids::MapRef;
use crate::map::{ServiceId, SystemMap};
use crate::progress::Finding;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// At most this many edges in one report; a source cuts there and sets `truncated`. A report
/// with more is rejected whole.
pub const MAX_EDGES: usize = 2_000;
/// The longest service name, in bytes.
pub const MAX_NAME_BYTES: usize = 200;
/// The shortest and longest window a report may cover.
pub const MIN_WINDOW_SECS: u32 = 60;
pub const MAX_WINDOW_SECS: u32 = 86_400;
/// At most this many unmapped service names are listed for the inspector.
pub const UNMAPPED_SAMPLE: usize = 20;
/// A report older than this many fetch intervals is stale (plan #104, A6).
pub const STALE_AFTER_FETCHES: u32 = 3;
/// At most this many ghost parts and edges to them reach the overview, the busiest first
/// (plan #140, A4); past either, `ghosts_truncated`.
pub const MAX_GHOSTS: usize = 50;
pub const MAX_GHOST_EDGES: usize = 200;
/// At most this many dismissals per workspace (ADR 0060).
pub const MAX_DISMISSALS: usize = 500;
/// The lookbacks a source asks, shortest first, for when a quiet part or seam was last seen
/// (ADR 0083). A source skips any no longer than its window.
pub const LOOKBACK_SECS: [u32; 3] = [21_600, 86_400, 604_800];
/// The longest lookback a report may carry.
pub const MAX_LOOKBACK_SECS: u32 = 604_800;

/// Calls observed from one service to another in a report's window: the shape of the
/// OpenTelemetry Collector `servicegraph` connector's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TrafficEdge {
    /// OpenTelemetry `service.name` of the caller.
    pub client: String,
    /// OpenTelemetry `service.name` of the callee.
    pub server: String,
    /// OpenTelemetry `service.namespace` of the caller, as it emits it; absent when it emits
    /// none, as a servicegraph virtual node from `peer.service` does (ADR 0059).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_namespace: Option<String>,
    /// OpenTelemetry `service.namespace` of the callee, as it emits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_namespace: Option<String>,
    /// Requests in the window.
    pub calls: u64,
    /// Of which failed.
    #[serde(default)]
    pub errors: u64,
}

/// Calls seen in a lookback longer than the report's window, but none in the window (ADR 0083).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SeenEdge {
    pub client: String,
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_namespace: Option<String>,
    /// The shortest lookback with calls, in seconds: one of `LOOKBACK_SECS`.
    pub within_secs: u32,
}

/// What any source of observed traffic produces (ADR 0039).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TrafficReport {
    /// When the source's window ended.
    pub observed_at: DateTime<Utc>,
    /// How far back the counts reach.
    pub window_secs: u32,
    pub edges: Vec<TrafficEdge>,
    /// The source cut the edges at `MAX_EDGES`.
    #[serde(default)]
    pub truncated: bool,
    /// Edges with calls in a longer lookback but none in the window (ADR 0083). Empty when the
    /// source asked no lookback, or nothing was seen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seen: Vec<SeenEdge>,
    /// The longest lookback the source asked, when it asked one: `seen` covers up to here, and
    /// a part not in it was not seen that far back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lookback_secs: Option<u32>,
}

/// Why a report was rejected whole. The previous report stays until it goes stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    TooManyEdges,
    BadWindow,
    NameTooLong,
}

impl Rejection {
    /// The reason as logged and counted.
    pub fn as_str(self) -> &'static str {
        match self {
            Rejection::TooManyEdges => "too_many_edges",
            Rejection::BadWindow => "bad_window",
            Rejection::NameTooLong => "name_too_long",
        }
    }
}

impl TrafficReport {
    /// Check the report against the limits (plan #104, A9).
    pub fn validate(&self) -> Result<(), Rejection> {
        if self.edges.len() > MAX_EDGES || self.seen.len() > MAX_EDGES {
            return Err(Rejection::TooManyEdges);
        }
        if !(MIN_WINDOW_SECS..=MAX_WINDOW_SECS).contains(&self.window_secs) {
            return Err(Rejection::BadWindow);
        }
        // A lookback is longer than the window and no longer than a week; so is each `seen`.
        let lookback_ok = |secs: u32| secs > self.window_secs && secs <= MAX_LOOKBACK_SECS;
        if self.lookback_secs.is_some_and(|l| !lookback_ok(l))
            || self.seen.iter().any(|e| {
                !lookback_ok(e.within_secs) || self.lookback_secs.is_none_or(|l| e.within_secs > l)
            })
        {
            return Err(Rejection::BadWindow);
        }
        let names = self
            .edges
            .iter()
            .map(|e| {
                [
                    Some(&e.client),
                    Some(&e.server),
                    e.client_namespace.as_ref(),
                    e.server_namespace.as_ref(),
                ]
            })
            .chain(self.seen.iter().map(|e| {
                [
                    Some(&e.client),
                    Some(&e.server),
                    e.client_namespace.as_ref(),
                    e.server_namespace.as_ref(),
                ]
            }));
        if names.flatten().flatten().any(|s| s.len() > MAX_NAME_BYTES) {
            return Err(Rejection::NameTooLong);
        }
        Ok(())
    }
}

/// Whether the source can be believed. Only `fresh` draws traffic or makes findings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrafficState {
    /// No source is configured.
    #[default]
    NotConfigured,
    /// The last report is recent enough to judge the map by.
    Fresh,
    /// No report yet, or the last good one is older than three fetch intervals.
    Stale,
    /// The last fetch failed.
    Failing,
}

/// Calls on one declared seam in the report's window, each way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SeamTraffic {
    pub seam: String,
    /// Calls whose client is the seam's `from` side.
    pub forward_calls: u64,
    /// Calls whose client is the seam's `to` side.
    pub reverse_calls: u64,
    /// Failed calls, both ways.
    pub errors: u64,
}

/// Calls between two map parts that no seam joins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UndeclaredEdge {
    /// The calling part.
    pub from: String,
    /// The called part.
    pub to: String,
    pub calls: u64,
    pub errors: u64,
}

/// One end of an observed edge on the overlay: a declared part, or a ghost (ADR 0059).
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrafficEnd {
    Part { id: String },
    Ghost { service: ServiceId },
}

/// A service seen in a fresh report, emitting a namespace, that no part names: drawn on the
/// overlay until a person takes it into the map draft (plan #140). Never a map part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GhostPart {
    pub service: ServiceId,
    /// Calls it received in the window.
    pub calls_in: u64,
    /// Calls it made.
    pub calls_out: u64,
    /// Failed calls either way.
    pub errors: u64,
}

/// Calls with a ghost at one end or both. `from` calls `to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GhostEdge {
    pub from: TrafficEnd,
    pub to: TrafficEnd,
    pub calls: u64,
    pub errors: u64,
}

/// What a person can dismiss on the overlay (ADR 0060).
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiscrepancyKey {
    /// A ghost part, and with it every edge to or from it.
    Ghost { service: ServiceId },
    /// An observed edge no seam declares, between parts or with a ghost end.
    Edge { from: TrafficEnd, to: TrafficEnd },
    /// A declared seam that carried no calls.
    QuietSeam { seam: String },
}

impl DiscrepancyKey {
    /// Longest name in the key, for the same limit a report has.
    pub fn fits(&self) -> bool {
        let ok = |s: &str| !s.is_empty() && s.len() <= MAX_NAME_BYTES;
        let svc = |s: &ServiceId| ok(&s.name) && s.namespace.as_deref().is_none_or(ok);
        let end = |e: &TrafficEnd| match e {
            TrafficEnd::Part { id } => ok(id),
            TrafficEnd::Ghost { service } => svc(service),
        };
        match self {
            DiscrepancyKey::Ghost { service } => svc(service),
            DiscrepancyKey::Edge { from, to } => end(from) && end(to),
            DiscrepancyKey::QuietSeam { seam } => ok(seam),
        }
    }
}

/// One dismissal: who dismissed what, and when (ADR 0060). Undo deletes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TrafficDismissal {
    pub id: i64,
    pub target: DiscrepancyKey,
    /// GitHub login.
    pub author: String,
    pub at: DateTime<Utc>,
}

/// `POST /api/traffic/dismissals`: dismiss one discrepancy as yourself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DismissRequest {
    pub target: DiscrepancyKey,
}

pub mod routes {
    /// `POST` dismisses; `DELETE …/{id}` undoes. Map-scoped (ADR 0049).
    pub const TRAFFIC_DISMISSALS: &str = "/api/traffic/dismissals";
}

/// Observed traffic as the overview carries it (`Overview.traffic`, ADR 0039). Never an input
/// to progress, pins, plans or approvals.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ObservedTraffic {
    pub state: TrafficState,
    /// When the last good report's window ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<DateTime<Utc>>,
    /// The last good report's window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_secs: Option<u32>,
    /// Why the last fetch failed, when `failing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Declared seams with calls in the window, in map order. Empty unless `fresh`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seams: Vec<SeamTraffic>,
    /// Calls between two parts no seam joins. Empty unless `fresh`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undeclared: Vec<UndeclaredEdge>,
    /// Parts that called or were called in the window. Empty unless `fresh`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_parts: Vec<String>,
    /// Services seen that no part names and that emit a namespace, busiest first. Empty unless
    /// `fresh`; dismissed ones left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ghosts: Vec<GhostPart>,
    /// Calls to or from a ghost. Empty unless `fresh`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ghost_edges: Vec<GhostEdge>,
    /// More ghosts or ghost edges were seen than the overview carries.
    #[serde(default)]
    pub ghosts_truncated: bool,
    /// Every dismissal in the workspace, whatever the state, so each can be undone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dismissed: Vec<TrafficDismissal>,
    /// Edge ends in the report with no identity to match: no part names them and they emit no
    /// namespace (ADR 0059).
    #[serde(default)]
    pub unmapped_services: u32,
    /// Up to 20 of them, sorted. Text only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmapped_sample: Vec<String>,
    /// The source cut the report at its edge limit.
    #[serde(default)]
    pub truncated: bool,
    /// Parts and declared seams with no calls in the window that were seen in a longer
    /// lookback, with the shortest one, in seconds (ADR 0083). Empty unless `fresh`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_seen: Vec<LastSeen>,
    /// The longest lookback asked, when one was: a quiet part or seam with `service` that is
    /// not in `last_seen` was not seen that far back. Absent unless `fresh` and asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lookback_secs: Option<u32>,
    /// Parts without a `service`, and seams with such an end: nothing can observe them, so
    /// they are never drawn as silent (ADR 0083). Empty unless `fresh`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_instrumented: Vec<MapRef>,
}

/// A quiet part or seam, and the shortest lookback it was seen in (ADR 0083).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LastSeen {
    #[serde(rename = "ref")]
    pub at: MapRef,
    pub within_secs: u32,
}

/// The source as the server knows it, next to its latest report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceStatus {
    NotConfigured,
    Configured {
        /// Seconds between fetches; the report is stale after three of them.
        fetch_secs: u32,
        /// The last fetch's error; `None` when it succeeded or none has run.
        error: Option<String>,
    },
}

/// One edge end, resolved against the map (ADR 0059).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Resolved<'a> {
    Part(&'a str),
    Ghost(ServiceId),
    /// No part names it and it emits no namespace: counted only.
    Unidentified(String),
}

/// Turn the latest good report and the map into the overview's traffic and the
/// map-versus-territory findings (ADR 0039, design §3; plan #140 design §4). A source that is not
/// configured, failing, or whose report is older than three fetch intervals makes no seams, no
/// undeclared edges, no ghosts and no findings: unknown is never quiet. `dismissed` (ADR 0060)
/// leaves what it names out of the drawn lists and the findings, and is always listed.
pub fn derive_traffic(
    map: &SystemMap,
    report: Option<&TrafficReport>,
    source: &SourceStatus,
    dismissed: &[TrafficDismissal],
    now: DateTime<Utc>,
) -> (ObservedTraffic, Vec<Finding>) {
    // 1. State. Only a fresh report goes on.
    let (fetch_secs, error) = match source {
        SourceStatus::NotConfigured => {
            let t = ObservedTraffic {
                dismissed: dismissed.to_vec(),
                ..ObservedTraffic::default()
            };
            return (t, Vec::new());
        }
        SourceStatus::Configured { fetch_secs, error } => (*fetch_secs, error.clone()),
    };
    let unknown = |state: TrafficState| ObservedTraffic {
        state,
        observed_at: report.map(|r| r.observed_at),
        window_secs: report.map(|r| r.window_secs),
        error: error.clone(),
        dismissed: dismissed.to_vec(),
        ..ObservedTraffic::default()
    };
    if error.is_some() {
        return (unknown(TrafficState::Failing), Vec::new());
    }
    let Some(report) = report else {
        return (unknown(TrafficState::Stale), Vec::new());
    };
    let max_age = i64::from(fetch_secs) * i64::from(STALE_AFTER_FETCHES);
    if now - report.observed_at > chrono::Duration::seconds(max_age) {
        return (unknown(TrafficState::Stale), Vec::new());
    }
    let hidden: BTreeSet<&DiscrepancyKey> = dismissed.iter().map(|d| &d.target).collect();

    // 2. Each end to a part by the identity it emits, or a ghost, or nothing.
    let by_service: BTreeMap<&ServiceId, &str> = map
        .nodes
        .iter()
        .filter_map(|n| n.service.as_ref().map(|s| (s, n.id.as_str())))
        .collect();
    let resolve = |namespace: &Option<String>, name: &str| -> Resolved<'_> {
        let sid = ServiceId {
            namespace: namespace.clone(),
            name: name.to_owned(),
        };
        match by_service.get(&sid) {
            Some(id) => Resolved::Part(id),
            None if namespace.is_some() => Resolved::Ghost(sid),
            None => Resolved::Unidentified(name.to_owned()),
        }
    };
    let mut unmapped: BTreeSet<String> = BTreeSet::new();
    // 3. Edges summed per direction: between two parts, and with a ghost at an end.
    let mut calls: BTreeMap<(&str, &str), (u64, u64)> = BTreeMap::new();
    let mut ghostly: BTreeMap<(TrafficEnd, TrafficEnd), (u64, u64)> = BTreeMap::new();
    for e in &report.edges {
        let client = resolve(&e.client_namespace, &e.client);
        let server = resolve(&e.server_namespace, &e.server);
        if client == server {
            continue;
        }
        let end = |r: &Resolved<'_>| match r {
            Resolved::Part(id) => Some(TrafficEnd::Part {
                id: (*id).to_owned(),
            }),
            Resolved::Ghost(s) => Some(TrafficEnd::Ghost { service: s.clone() }),
            Resolved::Unidentified(_) => None,
        };
        match (&client, &server) {
            (Resolved::Unidentified(a), Resolved::Unidentified(b)) => {
                unmapped.insert(a.clone());
                unmapped.insert(b.clone());
            }
            (Resolved::Unidentified(n), _) | (_, Resolved::Unidentified(n)) => {
                unmapped.insert(n.clone());
            }
            (Resolved::Part(a), Resolved::Part(b)) => {
                let c = calls.entry((a, b)).or_default();
                c.0 = c.0.saturating_add(e.calls);
                c.1 = c.1.saturating_add(e.errors);
            }
            _ => {
                let (Some(from), Some(to)) = (end(&client), end(&server)) else {
                    continue;
                };
                let c = ghostly.entry((from, to)).or_default();
                c.0 = c.0.saturating_add(e.calls);
                c.1 = c.1.saturating_add(e.errors);
            }
        }
    }
    // 4. Ghosts, busiest first, without the dismissed; then the edges to the ghosts kept.
    let mut ghosts: BTreeMap<ServiceId, GhostPart> = BTreeMap::new();
    for ((from, to), &(n, errors)) in &ghostly {
        if n == 0 {
            continue;
        }
        for (end, out) in [(from, true), (to, false)] {
            if let TrafficEnd::Ghost { service } = end {
                let g = ghosts.entry(service.clone()).or_insert_with(|| GhostPart {
                    service: service.clone(),
                    calls_in: 0,
                    calls_out: 0,
                    errors: 0,
                });
                if out {
                    g.calls_out = g.calls_out.saturating_add(n);
                } else {
                    g.calls_in = g.calls_in.saturating_add(n);
                }
                g.errors = g.errors.saturating_add(errors);
            }
        }
    }
    let mut ghosts: Vec<GhostPart> = ghosts
        .into_values()
        .filter(|g| {
            !hidden.contains(&DiscrepancyKey::Ghost {
                service: g.service.clone(),
            })
        })
        .collect();
    ghosts.sort_by(|a, b| {
        (b.calls_in + b.calls_out)
            .cmp(&(a.calls_in + a.calls_out))
            .then_with(|| a.service.cmp(&b.service))
    });
    let mut ghosts_truncated = ghosts.len() > MAX_GHOSTS;
    ghosts.truncate(MAX_GHOSTS);
    let kept: BTreeSet<&ServiceId> = ghosts.iter().map(|g| &g.service).collect();
    let shown = |e: &TrafficEnd| match e {
        TrafficEnd::Part { .. } => true,
        TrafficEnd::Ghost { service } => kept.contains(service),
    };
    let mut ghost_edges: Vec<GhostEdge> = ghostly
        .iter()
        .filter(|((from, to), (n, _))| {
            *n > 0
                && shown(from)
                && shown(to)
                && !hidden.contains(&DiscrepancyKey::Edge {
                    from: from.clone(),
                    to: to.clone(),
                })
        })
        .map(|((from, to), &(calls, errors))| GhostEdge {
            from: from.clone(),
            to: to.clone(),
            calls,
            errors,
        })
        .collect();
    ghost_edges.sort_by_key(|e| std::cmp::Reverse(e.calls));
    ghosts_truncated |= ghost_edges.len() > MAX_GHOST_EDGES;
    ghost_edges.truncate(MAX_GHOST_EDGES);

    // 5. Observed parts: an end of an edge with calls, to a part or a ghost. An end with no
    // identity says nothing about the part it talks to.
    let observed: BTreeSet<&str> = calls
        .iter()
        .filter(|(_, (n, _))| *n > 0)
        .flat_map(|((a, b), _)| [*a, *b])
        .chain(
            ghostly
                .iter()
                .filter(|(_, (n, _))| *n > 0)
                .flat_map(|((a, b), _)| [a, b])
                .filter_map(|e| match e {
                    TrafficEnd::Part { id } => map.node(id).map(|n| n.id.as_str()),
                    TrafficEnd::Ghost { .. } => None,
                }),
        )
        .collect();

    let mut seams = Vec::new();
    let mut findings = Vec::new();
    for s in &map.seams {
        let (f, ferr) = calls
            .get(&(s.from.as_str(), s.to.as_str()))
            .copied()
            .unwrap_or_default();
        let (r, rerr) = calls
            .get(&(s.to.as_str(), s.from.as_str()))
            .copied()
            .unwrap_or_default();
        let declares = |id: &str| map.node(id).is_some_and(|n| n.service.is_some());
        if f > 0 || r > 0 {
            seams.push(SeamTraffic {
                seam: s.id.clone(),
                forward_calls: f,
                reverse_calls: r,
                errors: ferr.saturating_add(rerr),
            });
        } else if s.from != s.to
            && declares(&s.from)
            && declares(&s.to)
            && observed.contains(s.from.as_str())
            && observed.contains(s.to.as_str())
            && !hidden.contains(&DiscrepancyKey::QuietSeam { seam: s.id.clone() })
        {
            // 6. Quiet: both parts say which service they are and show traffic elsewhere.
            findings.push(Finding::SeamQuiet {
                seam: s.id.clone(),
                window_secs: report.window_secs,
            });
        }
    }
    let mut undeclared = Vec::new();
    for (&(from, to), &(n, errors)) in &calls {
        if n == 0
            || map.seams.iter().any(|s| s.joins(from, to))
            || hidden.contains(&DiscrepancyKey::Edge {
                from: TrafficEnd::Part {
                    id: from.to_owned(),
                },
                to: TrafficEnd::Part { id: to.to_owned() },
            })
        {
            continue;
        }
        findings.push(Finding::UndeclaredTraffic {
            from: from.to_owned(),
            to: to.to_owned(),
            calls: n,
        });
        undeclared.push(UndeclaredEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            calls: n,
            errors,
        });
    }
    for g in &ghosts {
        findings.push(Finding::ObservedOnlyPart {
            service: g.service.clone(),
            calls: g.calls_in.saturating_add(g.calls_out),
        });
    }
    // 7. When each quiet part and seam was last seen, and what cannot be seen (ADR 0083).
    let (last_seen, not_instrumented) = seen_and_blind(map, report, &observed, &seams, &resolve);
    let traffic = ObservedTraffic {
        state: TrafficState::Fresh,
        observed_at: Some(report.observed_at),
        window_secs: Some(report.window_secs),
        error: None,
        seams,
        undeclared,
        observed_parts: observed.into_iter().map(str::to_owned).collect(),
        ghosts,
        ghost_edges,
        ghosts_truncated,
        dismissed: dismissed.to_vec(),
        unmapped_services: u32::try_from(unmapped.len()).unwrap_or(u32::MAX),
        unmapped_sample: unmapped.into_iter().take(UNMAPPED_SAMPLE).collect(),
        truncated: report.truncated,
        last_seen,
        lookback_secs: report.lookback_secs,
        not_instrumented,
    };
    (traffic, findings)
}

/// The quiet parts and seams of a fresh report seen in a lookback, each with the shortest one,
/// and the parts and seams nothing can observe. Live parts and seams are in neither list.
fn seen_and_blind<'m>(
    map: &'m SystemMap,
    report: &TrafficReport,
    observed: &BTreeSet<&str>,
    seams: &[SeamTraffic],
    resolve: &dyn Fn(&Option<String>, &str) -> Resolved<'m>,
) -> (Vec<LastSeen>, Vec<MapRef>) {
    let live = |id: &str| {
        map.node(id)
            .is_some_and(|n| observed.contains(n.id.as_str()))
    };
    let instrumented = |id: &str| map.node(id).is_some_and(|n| n.service.is_some());
    // The shortest lookback per part, and per ordered pair of parts.
    let mut part_seen: BTreeMap<&str, u32> = BTreeMap::new();
    let mut pair_seen: BTreeMap<(&str, &str), u32> = BTreeMap::new();
    for e in &report.seen {
        let (Resolved::Part(a), Resolved::Part(b)) = (
            resolve(&e.client_namespace, &e.client),
            resolve(&e.server_namespace, &e.server),
        ) else {
            // A ghost or an unidentified end still says its known end was seen.
            for (ns, name) in [
                (&e.client_namespace, &e.client),
                (&e.server_namespace, &e.server),
            ] {
                if let Resolved::Part(id) = resolve(ns, name) {
                    let w = part_seen.entry(id).or_insert(e.within_secs);
                    *w = (*w).min(e.within_secs);
                }
            }
            continue;
        };
        if a == b {
            continue;
        }
        for id in [a, b] {
            let w = part_seen.entry(id).or_insert(e.within_secs);
            *w = (*w).min(e.within_secs);
        }
        let w = pair_seen.entry((a, b)).or_insert(e.within_secs);
        *w = (*w).min(e.within_secs);
    }
    let lit: BTreeSet<&str> = seams.iter().map(|s| s.seam.as_str()).collect();
    let mut last_seen = Vec::new();
    let mut blind = Vec::new();
    for n in map.nodes.iter().filter(|n| !n.is_retired()) {
        if !instrumented(&n.id) {
            blind.push(MapRef::node(n.id.clone()));
        } else if !live(&n.id)
            && let Some(&w) = part_seen.get(n.id.as_str())
        {
            last_seen.push(LastSeen {
                at: MapRef::node(n.id.clone()),
                within_secs: w,
            });
        }
    }
    for s in map.seams.iter().filter(|s| !s.is_retired()) {
        if !instrumented(&s.from) || !instrumented(&s.to) {
            blind.push(MapRef::seam(s.id.clone()));
            continue;
        }
        if lit.contains(s.id.as_str()) {
            continue;
        }
        let both = [
            (s.from.as_str(), s.to.as_str()),
            (s.to.as_str(), s.from.as_str()),
        ];
        if let Some(w) = both.iter().filter_map(|k| pair_seen.get(k)).min() {
            last_seen.push(LastSeen {
                at: MapRef::seam(s.id.clone()),
                within_secs: *w,
            });
        }
    }
    (last_seen, blind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{MapNode, NodeKind, Seam};
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 12, 0, 0).unwrap()
    }

    fn node(id: &str) -> MapNode {
        MapNode {
            id: id.into(),
            kind: NodeKind::Context,
            repo: None,
            path: None,
            description: None,
            service: Some(ServiceId::new(None, id)),
            retired: None,
        }
    }

    fn seam(id: &str, from: &str, to: &str) -> Seam {
        Seam {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            carries: None,
            owner: from.into(),
            guard: None,
            description: None,
            flows: vec![],
            retired: None,
        }
    }

    fn map() -> SystemMap {
        SystemMap {
            version: 1,
            name: "t".into(),
            owner_rule: None,
            repos: vec![],
            nodes: ["a", "b", "c"].into_iter().map(node).collect(),
            seams: vec![seam("a-b", "a", "b"), seam("b-c", "b", "c")],
            scenarios: vec![],
        }
    }

    fn report(age_secs: i64) -> TrafficReport {
        TrafficReport {
            observed_at: now() - chrono::Duration::seconds(age_secs),
            window_secs: 3600,
            edges: vec![
                TrafficEdge {
                    client: "a".into(),
                    server: "b".into(),
                    calls: 10,
                    errors: 1,
                    client_namespace: None,
                    server_namespace: None,
                },
                TrafficEdge {
                    client: "c".into(),
                    server: "a".into(),
                    calls: 4,
                    errors: 0,
                    client_namespace: None,
                    server_namespace: None,
                },
            ],
            truncated: false,
            seen: vec![],
            lookback_secs: None,
        }
    }

    fn seen(client: &str, server: &str, within_secs: u32) -> SeenEdge {
        SeenEdge {
            client: client.into(),
            server: server.into(),
            client_namespace: None,
            server_namespace: None,
            within_secs,
        }
    }

    /// a, b and c carry calls; d carries a service but no calls in the window; e carries no
    /// service. Seams c-d (quiet, both instrumented) and d-e (an end without service).
    fn quiet_map() -> SystemMap {
        let mut m = map();
        m.nodes.push(node("d"));
        m.nodes.push(MapNode {
            service: None,
            ..node("e")
        });
        m.seams.push(seam("c-d", "c", "d"));
        m.seams.push(seam("d-e", "d", "e"));
        m
    }

    #[test]
    fn a_quiet_part_and_seam_say_when_they_were_last_seen() {
        let r = TrafficReport {
            seen: vec![
                seen("c", "d", 86_400),
                seen("d", "c", 21_600),
                seen("d", "nobody", 604_800),
            ],
            lookback_secs: Some(604_800),
            ..report(10)
        };
        r.validate().unwrap();
        let (t, _) = derive_traffic(&quiet_map(), Some(&r), &configured(None), &[], now());
        assert_eq!(t.lookback_secs, Some(604_800));
        // d: the shortest lookback of any edge it is an end of; c-d: of either direction.
        assert_eq!(
            t.last_seen,
            vec![
                LastSeen {
                    at: MapRef::node("d"),
                    within_secs: 21_600
                },
                LastSeen {
                    at: MapRef::seam("c-d"),
                    within_secs: 21_600
                },
            ],
            "{t:?}"
        );
        // Live parts and seams are never "last seen": they are observed.
        assert!(
            !t.last_seen
                .iter()
                .any(|l| l.at == MapRef::node("a") || l.at == MapRef::seam("a-b"))
        );
    }

    #[test]
    fn a_part_without_service_and_its_seams_are_not_instrumented_never_quiet() {
        let r = TrafficReport {
            seen: vec![seen("d", "e", 21_600)],
            lookback_secs: Some(604_800),
            ..report(10)
        };
        let (t, f) = derive_traffic(&quiet_map(), Some(&r), &configured(None), &[], now());
        assert_eq!(
            t.not_instrumented,
            vec![MapRef::node("e"), MapRef::seam("d-e")],
            "{t:?}"
        );
        assert!(
            !t.last_seen.iter().any(|l| l.at == MapRef::seam("d-e")),
            "{t:?}"
        );
        assert!(
            !f.iter()
                .any(|f| matches!(f, Finding::SeamQuiet { seam, .. } if seam == "d-e")),
            "{f:?}"
        );
    }

    #[test]
    fn without_a_lookback_nothing_is_last_seen_and_unknown_stays_unknown() {
        // An old source: no lookback asked, nothing claimed about how long a part was quiet.
        let (t, _) = derive_traffic(
            &quiet_map(),
            Some(&report(10)),
            &configured(None),
            &[],
            now(),
        );
        assert!(t.last_seen.is_empty() && t.lookback_secs.is_none(), "{t:?}");
        // Any non-fresh state carries none of it.
        let r = TrafficReport {
            seen: vec![seen("c", "d", 21_600)],
            lookback_secs: Some(604_800),
            ..report(10)
        };
        let (t, _) = derive_traffic(
            &quiet_map(),
            Some(&r),
            &configured(Some("gone")),
            &[],
            now(),
        );
        assert!(
            t.last_seen.is_empty() && t.lookback_secs.is_none() && t.not_instrumented.is_empty(),
            "{t:?}"
        );
        // A report written before ADR 0083 reads with neither field.
        let old = r#"{"observed_at":"2026-11-13T12:00:00Z","window_secs":3600,"edges":[]}"#;
        let old: TrafficReport = serde_json::from_str(old).unwrap();
        assert!(old.seen.is_empty() && old.lookback_secs.is_none());
    }

    fn edge(client: (&str, Option<&str>), server: (&str, Option<&str>), calls: u64) -> TrafficEdge {
        TrafficEdge {
            client: client.0.into(),
            client_namespace: client.1.map(str::to_owned),
            server: server.0.into(),
            server_namespace: server.1.map(str::to_owned),
            calls,
            errors: 0,
        }
    }

    /// Rubato's own map (ADR 0082, plan #300): the web's relayed spans light `web-server`, and
    /// with the server naming its reads, `server-plan-file` is not falsely quiet once
    /// `plan-file` carries a service.
    #[test]
    fn on_rubatos_own_map_the_web_and_the_servers_reads_light_their_seams() {
        let own: SystemMap =
            serde_yaml_ng::from_str(include_str!("../../../docs/design/rubato-map.yml")).unwrap();
        let rubato = Some("rubato");
        let base = vec![
            edge(("web", rubato), ("rubato-server", rubato), 600),
            edge(("rubato", rubato), ("plan-file", None), 3),
        ];
        let r = |edges: Vec<TrafficEdge>| TrafficReport { edges, ..report(0) };
        let (t, f) = derive_traffic(&own, Some(&r(base.clone())), &configured(None), &[], now());
        assert!(
            t.seams
                .iter()
                .any(|s| s.seam == "web-server" && s.forward_calls == 600),
            "{t:?}"
        );
        assert!(t.seams.iter().any(|s| s.seam == "skill-plan-file"), "{t:?}");
        assert!(
            t.ghosts.is_empty(),
            "web must be a part, not a ghost: {t:?}"
        );
        // Without the server's own plan-file reads, both ends are observed and declare a
        // service: the seam reads quiet. That is the false finding the server's reads prevent.
        assert!(
            f.iter().any(
                |f| matches!(f, Finding::SeamQuiet { seam, .. } if seam == "server-plan-file")
            ),
            "{f:?}"
        );
        let mut named = base;
        named.push(edge(("rubato-server", rubato), ("plan-file", None), 12));
        let (t, f) = derive_traffic(&own, Some(&r(named)), &configured(None), &[], now());
        assert!(
            t.seams.iter().any(|s| s.seam == "server-plan-file"),
            "{t:?}"
        );
        assert!(
            !f.iter().any(
                |f| matches!(f, Finding::SeamQuiet { seam, .. } if seam == "server-plan-file")
            ),
            "{f:?}"
        );
        // livekit and speech-provider carry no service: not instrumented, with their seams.
        for id in ["livekit", "speech-provider"] {
            assert!(
                t.not_instrumented.contains(&MapRef::node(id)),
                "{id}: {t:?}"
            );
        }
        assert!(
            t.not_instrumented.contains(&MapRef::seam("web-livekit")),
            "{t:?}"
        );
        assert!(!t.not_instrumented.contains(&MapRef::node("web")), "{t:?}");
    }

    #[test]
    fn a_lookback_must_be_longer_than_the_window_and_at_most_a_week() {
        let ok = TrafficReport {
            seen: vec![seen("c", "d", 21_600)],
            lookback_secs: Some(604_800),
            ..report(0)
        };
        assert_eq!(ok.validate(), Ok(()));
        let bad = [
            (
                "seen no longer than the window",
                TrafficReport {
                    seen: vec![seen("c", "d", 3600)],
                    ..ok.clone()
                },
            ),
            (
                "seen without a lookback",
                TrafficReport {
                    lookback_secs: None,
                    ..ok.clone()
                },
            ),
            (
                "seen beyond the lookback",
                TrafficReport {
                    lookback_secs: Some(21_600),
                    seen: vec![seen("c", "d", 86_400)],
                    ..ok.clone()
                },
            ),
            (
                "a lookback over a week",
                TrafficReport {
                    lookback_secs: Some(MAX_LOOKBACK_SECS + 1),
                    seen: vec![],
                    ..ok.clone()
                },
            ),
        ];
        for (name, r) in bad {
            assert_eq!(r.validate(), Err(Rejection::BadWindow), "{name}");
        }
        let long = TrafficReport {
            seen: vec![seen(&"x".repeat(MAX_NAME_BYTES + 1), "d", 21_600)],
            ..ok
        };
        assert_eq!(long.validate(), Err(Rejection::NameTooLong));
    }

    fn configured(error: Option<&str>) -> SourceStatus {
        SourceStatus::Configured {
            fetch_secs: 60,
            error: error.map(str::to_owned),
        }
    }

    #[test]
    fn a_fresh_report_lights_seams_and_makes_findings() {
        let (t, f) = derive_traffic(&map(), Some(&report(10)), &configured(None), &[], now());
        assert_eq!(t.state, TrafficState::Fresh);
        assert_eq!(t.seams.len(), 1, "{t:?}");
        assert_eq!(t.undeclared.len(), 1, "{t:?}");
        assert_eq!(f.len(), 2, "{f:?}");
    }

    /// Unknown, not quiet: every non-fresh state returns nothing to draw and no finding, and
    /// keeps the last good report's time.
    #[test]
    fn every_non_fresh_state_is_unknown_never_quiet() {
        let r = report(0);
        let cases = [
            (
                "not_configured",
                SourceStatus::NotConfigured,
                0,
                TrafficState::NotConfigured,
            ),
            ("stale", configured(None), 3 * 60 + 1, TrafficState::Stale),
            (
                "failing",
                configured(Some("gone")),
                0,
                TrafficState::Failing,
            ),
        ];
        for (name, source, age, want) in &cases {
            let r = TrafficReport {
                observed_at: now() - chrono::Duration::seconds(*age),
                ..r.clone()
            };
            let (t, f) = derive_traffic(&map(), Some(&r), source, &[], now());
            assert_eq!(t.state, *want, "{name}");
            assert!(t.seams.is_empty(), "{name}: {t:?}");
            assert!(t.undeclared.is_empty(), "{name}: {t:?}");
            assert!(t.observed_parts.is_empty(), "{name}: {t:?}");
            assert!(f.is_empty(), "{name}: {f:?}");
            if *want != TrafficState::NotConfigured {
                assert_eq!(t.observed_at, Some(r.observed_at), "{name}");
            }
        }
        let (t, _) = derive_traffic(
            &map(),
            Some(&report(0)),
            &configured(Some("gone")),
            &[],
            now(),
        );
        assert_eq!(t.error.as_deref(), Some("gone"));
        // Configured with no report yet: unknown too.
        let (t, f) = derive_traffic(&map(), None, &configured(None), &[], now());
        assert_eq!(t.state, TrafficState::Stale);
        assert!(f.is_empty());
        println!("checked {} non-fresh states and no report", cases.len());
    }

    #[test]
    fn at_exactly_three_intervals_the_report_is_still_fresh() {
        let (t, _) = derive_traffic(&map(), Some(&report(180)), &configured(None), &[], now());
        assert_eq!(t.state, TrafficState::Fresh);
        let (t, _) = derive_traffic(&map(), Some(&report(181)), &configured(None), &[], now());
        assert_eq!(t.state, TrafficState::Stale);
    }

    #[test]
    fn limits_reject_a_report_whole() {
        let mut r = report(0);
        assert_eq!(r.validate(), Ok(()));
        r.window_secs = 30;
        assert_eq!(r.validate(), Err(Rejection::BadWindow));
        let mut r = report(0);
        r.edges[0].client = "x".repeat(MAX_NAME_BYTES + 100);
        assert_eq!(r.validate(), Err(Rejection::NameTooLong));
        let mut r = report(0);
        r.edges[1].server_namespace = Some("n".repeat(MAX_NAME_BYTES + 1));
        assert_eq!(r.validate(), Err(Rejection::NameTooLong));
        let mut r = report(0);
        r.edges = vec![r.edges[0].clone(); MAX_EDGES + 1];
        assert_eq!(r.validate(), Err(Rejection::TooManyEdges));
        r.edges.truncate(MAX_EDGES);
        assert_eq!(r.validate(), Ok(()));
    }

    /// An overview from a server that predates #104 has no `traffic`: it reads as not configured.
    #[test]
    fn missing_traffic_reads_as_not_configured() {
        let t: ObservedTraffic = serde_json::from_str(r#"{"state":"fresh"}"#).unwrap();
        assert_eq!(t.state, TrafficState::Fresh);
        assert_eq!(
            ObservedTraffic::default().state,
            TrafficState::NotConfigured
        );
    }

    /// Dismissals are listed in every state, so each can be undone, and hide nothing then:
    /// there is nothing drawn to hide.
    #[test]
    fn dismissals_are_listed_whatever_the_state() {
        let d = vec![TrafficDismissal {
            id: 7,
            target: DiscrepancyKey::QuietSeam { seam: "a-b".into() },
            author: "m0x6A".into(),
            at: now(),
        }];
        for source in [SourceStatus::NotConfigured, configured(Some("gone"))] {
            let (t, f) = derive_traffic(&map(), Some(&report(0)), &source, &d, now());
            assert_eq!(t.dismissed, d);
            assert!(f.is_empty() && t.ghosts.is_empty());
        }
        let (t, _) = derive_traffic(&map(), Some(&report(500)), &configured(None), &d, now());
        assert_eq!((t.state, t.dismissed.len()), (TrafficState::Stale, 1));
    }

    #[test]
    fn a_key_longer_than_a_name_does_not_fit() {
        let long = "x".repeat(MAX_NAME_BYTES + 1);
        assert!(DiscrepancyKey::QuietSeam { seam: "a-b".into() }.fits());
        assert!(!DiscrepancyKey::QuietSeam { seam: long.clone() }.fits());
        assert!(
            !DiscrepancyKey::Ghost {
                service: ServiceId::new(Some(&long), "x")
            }
            .fits()
        );
        assert!(
            !DiscrepancyKey::Ghost {
                service: ServiceId::new(None, "")
            }
            .fits()
        );
    }
}
