//! `derive_traffic` over the fixtures in `fixtures/traffic/` (plan #104, ADR 0039). Each case
//! names a report, a clock and what the overview must carry; the cases under `must-fail/` are
//! wrong on purpose and must be rejected, so the comparison can fail.

use chrono::{DateTime, Utc};
use rubato_protocol::{
    Finding, ObservedTraffic, Overview, SourceStatus, SystemMap, TrafficReport, TrafficState,
    derive_traffic,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Case {
    why: String,
    now: DateTime<Utc>,
    fetch_secs: u32,
    report: TrafficReport,
    expect: Expect,
}

#[derive(Deserialize)]
struct Expect {
    traffic: ObservedTraffic,
    findings: Vec<Finding>,
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/traffic")
}

fn map() -> SystemMap {
    serde_json::from_str(&std::fs::read_to_string(dir().join("map.json")).unwrap()).unwrap()
}

fn cases(dir: &Path) -> Vec<(String, Case)> {
    let mut out: Vec<(String, Case)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter(|p| {
            !matches!(
                p.file_name().and_then(|n| n.to_str()),
                Some("map.json" | "old-server-overview.json")
            )
        })
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let case = serde_json::from_str(&std::fs::read_to_string(&p).unwrap())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            (name, case)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// What is wrong with one case, if anything.
fn judge(map: &SystemMap, case: &Case) -> Option<String> {
    let source = SourceStatus::Configured {
        fetch_secs: case.fetch_secs,
        error: None,
    };
    let (traffic, findings) = derive_traffic(map, Some(&case.report), &source, case.now);
    if traffic != case.expect.traffic {
        return Some(format!(
            "traffic\n  got  {traffic:?}\n  want {:?}",
            case.expect.traffic
        ));
    }
    if findings != case.expect.findings {
        return Some(format!(
            "findings\n  got  {findings:?}\n  want {:?}",
            case.expect.findings
        ));
    }
    None
}

#[test]
fn every_fixture_derives_what_it_expects() {
    let map = map();
    let all = cases(&dir());
    assert!(all.len() >= 4, "too few fixtures: {}", all.len());
    let mut edges = 0;
    let mut problems = Vec::new();
    for (name, case) in &all {
        edges += case.report.edges.len();
        if let Some(p) = judge(&map, case) {
            problems.push(format!("{name} ({}): {p}", case.why));
        }
    }
    println!("checked {} traffic fixtures, {edges} edges", all.len());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_must_fail_fixtures_are_rejected() {
    let map = map();
    let all = cases(&dir().join("must-fail"));
    assert!(!all.is_empty(), "no must-fail fixture");
    for (name, case) in &all {
        assert!(
            judge(&map, case).is_some(),
            "{name} ({}) passed, but must fail",
            case.why
        );
    }
    println!("rejected {} must-fail traffic fixtures", all.len());
}

/// An overview from a server built before #104 has no `traffic`: not configured, never quiet.
#[test]
fn an_old_server_overview_reads_as_not_configured() {
    let text = std::fs::read_to_string(dir().join("old-server-overview.json")).unwrap();
    let o: Overview = serde_json::from_str(&text).unwrap();
    assert_eq!(o.traffic.state, TrafficState::NotConfigured);
    assert!(o.traffic.seams.is_empty());
    assert_eq!(o.findings.len(), 1);
}
