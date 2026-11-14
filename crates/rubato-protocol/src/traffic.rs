//! Observed traffic per seam (ADR 0039): the one report every source fills, what the overview
//! carries, and the pure derivation from the two.
//!
//! Service names come from outside the team: any instrumented service chooses its own. They are
//! text only. A service maps to the map part whose id equals its OpenTelemetry `service.name`
//! (plan #104, Q2 (a)); a name that matches no part is counted, never made a part.

use crate::map::SystemMap;
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

/// Calls observed from one service to another in a report's window: the shape of the
/// OpenTelemetry Collector `servicegraph` connector's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TrafficEdge {
    /// OpenTelemetry `service.name` of the caller.
    pub client: String,
    /// OpenTelemetry `service.name` of the callee.
    pub server: String,
    /// Requests in the window.
    pub calls: u64,
    /// Of which failed.
    #[serde(default)]
    pub errors: u64,
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
        if self.edges.len() > MAX_EDGES {
            return Err(Rejection::TooManyEdges);
        }
        if !(MIN_WINDOW_SECS..=MAX_WINDOW_SECS).contains(&self.window_secs) {
            return Err(Rejection::BadWindow);
        }
        if self
            .edges
            .iter()
            .any(|e| e.client.len() > MAX_NAME_BYTES || e.server.len() > MAX_NAME_BYTES)
        {
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
    /// Services in the report that name no map part.
    #[serde(default)]
    pub unmapped_services: u32,
    /// Up to 20 of them, sorted. Text only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmapped_sample: Vec<String>,
    /// The source cut the report at its edge limit.
    #[serde(default)]
    pub truncated: bool,
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

/// Turn the latest good report and the map into the overview's traffic and the
/// map-versus-territory findings (ADR 0039, design §3). A source that is not configured,
/// failing, or whose report is older than three fetch intervals makes no seams, no undeclared
/// edges and no findings: unknown is never quiet.
pub fn derive_traffic(
    map: &SystemMap,
    report: Option<&TrafficReport>,
    source: &SourceStatus,
    now: DateTime<Utc>,
) -> (ObservedTraffic, Vec<Finding>) {
    // 1. State. Only a fresh report goes on.
    let (fetch_secs, error) = match source {
        SourceStatus::NotConfigured => return (ObservedTraffic::default(), Vec::new()),
        SourceStatus::Configured { fetch_secs, error } => (*fetch_secs, error.clone()),
    };
    let unknown = |state: TrafficState| ObservedTraffic {
        state,
        observed_at: report.map(|r| r.observed_at),
        window_secs: report.map(|r| r.window_secs),
        error: error.clone(),
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

    // 2. Service to part: the part whose id is the service's name (Q2 (a)).
    let parts: BTreeSet<&str> = map.node_ids();
    let mut unmapped: BTreeSet<&str> = BTreeSet::new();
    // 3. Edges between two mapped parts, summed per direction.
    let mut calls: BTreeMap<(&str, &str), (u64, u64)> = BTreeMap::new();
    for e in &report.edges {
        let (client, server) = (e.client.as_str(), e.server.as_str());
        let mut mapped = true;
        for name in [client, server] {
            if !parts.contains(name) {
                unmapped.insert(name);
                mapped = false;
            }
        }
        if !mapped || client == server {
            continue;
        }
        let c = calls.entry((client, server)).or_default();
        c.0 = c.0.saturating_add(e.calls);
        c.1 = c.1.saturating_add(e.errors);
    }
    // 4. Observed parts: either end of a mapped edge with calls.
    let observed: BTreeSet<&str> = calls
        .iter()
        .filter(|(_, (n, _))| *n > 0)
        .flat_map(|((a, b), _)| [*a, *b])
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
        if f > 0 || r > 0 {
            seams.push(SeamTraffic {
                seam: s.id.clone(),
                forward_calls: f,
                reverse_calls: r,
                errors: ferr.saturating_add(rerr),
            });
        } else if s.from != s.to
            && observed.contains(s.from.as_str())
            && observed.contains(s.to.as_str())
        {
            // 5. Quiet: both parts show traffic elsewhere in the same report.
            findings.push(Finding::SeamQuiet {
                seam: s.id.clone(),
                window_secs: report.window_secs,
            });
        }
    }
    let mut undeclared = Vec::new();
    for (&(from, to), &(n, errors)) in &calls {
        if n == 0 || map.seams.iter().any(|s| s.joins(from, to)) {
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
    let traffic = ObservedTraffic {
        state: TrafficState::Fresh,
        observed_at: Some(report.observed_at),
        window_secs: Some(report.window_secs),
        error: None,
        seams,
        undeclared,
        observed_parts: observed.into_iter().map(str::to_owned).collect(),
        unmapped_services: u32::try_from(unmapped.len()).unwrap_or(u32::MAX),
        unmapped_sample: unmapped
            .into_iter()
            .take(UNMAPPED_SAMPLE)
            .map(str::to_owned)
            .collect(),
        truncated: report.truncated,
    };
    (traffic, findings)
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
                },
                TrafficEdge {
                    client: "c".into(),
                    server: "a".into(),
                    calls: 4,
                    errors: 0,
                },
            ],
            truncated: false,
        }
    }

    fn configured(error: Option<&str>) -> SourceStatus {
        SourceStatus::Configured {
            fetch_secs: 60,
            error: error.map(str::to_owned),
        }
    }

    #[test]
    fn a_fresh_report_lights_seams_and_makes_findings() {
        let (t, f) = derive_traffic(&map(), Some(&report(10)), &configured(None), now());
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
            let (t, f) = derive_traffic(&map(), Some(&r), source, now());
            assert_eq!(t.state, *want, "{name}");
            assert!(t.seams.is_empty(), "{name}: {t:?}");
            assert!(t.undeclared.is_empty(), "{name}: {t:?}");
            assert!(t.observed_parts.is_empty(), "{name}: {t:?}");
            assert!(f.is_empty(), "{name}: {f:?}");
            if *want != TrafficState::NotConfigured {
                assert_eq!(t.observed_at, Some(r.observed_at), "{name}");
            }
        }
        let (t, _) = derive_traffic(&map(), Some(&report(0)), &configured(Some("gone")), now());
        assert_eq!(t.error.as_deref(), Some("gone"));
        // Configured with no report yet: unknown too.
        let (t, f) = derive_traffic(&map(), None, &configured(None), now());
        assert_eq!(t.state, TrafficState::Stale);
        assert!(f.is_empty());
        println!("checked {} non-fresh states and no report", cases.len());
    }

    #[test]
    fn at_exactly_three_intervals_the_report_is_still_fresh() {
        let (t, _) = derive_traffic(&map(), Some(&report(180)), &configured(None), now());
        assert_eq!(t.state, TrafficState::Fresh);
        let (t, _) = derive_traffic(&map(), Some(&report(181)), &configured(None), now());
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
}
