//! The slice end to end over `RUBATO_DEV_TRAFFIC` (plan #104): the overview served over HTTP,
//! the web fan-out, the limits, a missing file, an old file, and the instruments read back.

use super::{DevFileSource, Fetcher, Outcome};
use crate::kernel::config::Config;
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{AppState, LoadedMap};
use crate::kernel::telemetry::{capture, metric_reader};
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::progress::routes;
use rubato_protocol::{Finding, Overview, SystemMap, TrafficState, WebEvent};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::broadcast::error::TryRecvError;

const FIXTURE: &str = include_str!("../../tests/fixtures/traffic/dev-traffic.json");
const MAP: &str = include_str!("../../../../docs/design/rubato-map.yml");

fn traffic_findings(o: &Overview) -> Vec<Finding> {
    o.findings
        .iter()
        .filter(|f| {
            matches!(
                f,
                Finding::SeamQuiet { .. } | Finding::UndeclaredTraffic { .. }
            )
        })
        .cloned()
        .collect()
}

/// Only `GET /api/overview`, through the overview slice's root: a slice never names the
/// composition root, so not `app::router`.
fn overview_only(app: crate::kernel::state::App) -> axum::Router {
    axum::Router::new()
        .route(
            routes::OVERVIEW,
            axum::routing::get(crate::overview::overview),
        )
        .with_state(app)
}

/// Every `Changed` event waiting, by reason.
fn drain(rx: &mut tokio::sync::broadcast::Receiver<WebEvent>) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(WebEvent::Changed { reason, .. }) => out.push(reason),
            Err(TryRecvError::Empty) => return out,
            Err(e) => panic!("{e}"),
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn the_dev_file_feeds_the_overview_and_the_instruments() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("traffic.json");
    std::fs::write(&file, FIXTURE).unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let mut cfg = Config::for_tests(url, "m0x6A");
    cfg.dev_traffic = Some(file.clone());
    let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let map: SystemMap = serde_yaml_ng::from_str(MAP).unwrap();
    *app.map.write().await = Some(LoadedMap {
        map,
        version: "v".into(),
        loaded_at: chrono::Utc::now(),
        yaml: String::new(),
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, overview_only(served)).await.unwrap() });
    let http = reqwest::Client::new();
    let overview = || async {
        http.get(format!("{base}{}", routes::OVERVIEW))
            .send()
            .await
            .unwrap()
            .json::<Overview>()
            .await
            .unwrap()
    };
    let mut web = app.web_tx.subscribe();
    let (events, _guard) = capture::start();
    let mut fetcher = Fetcher::new(DevFileSource::new(file.clone()), chrono::Utc::now());
    let now = chrono::Utc::now;

    // Configured, nothing fetched yet: unknown, not quiet.
    let o = overview().await;
    assert_eq!(o.traffic.state, TrafficState::Stale);
    assert!(traffic_findings(&o).is_empty());

    // The fixture: two lit seams, one quiet, one undeclared edge, one unmapped service.
    assert_eq!(fetcher.tick(&app, now()).await, Outcome::Ok);
    assert_eq!(drain(&mut web), ["traffic"]);
    let o = overview().await;
    assert_eq!(o.traffic.state, TrafficState::Fresh);
    let lit: Vec<(&str, u64)> = o
        .traffic
        .seams
        .iter()
        .map(|s| (s.seam.as_str(), s.forward_calls))
        .collect();
    assert_eq!(lit, [("server-github", 300), ("web-server", 1200)]);
    assert_eq!(o.traffic.unmapped_services, 1);
    assert_eq!(o.traffic.unmapped_sample, ["checkout"]);
    assert_eq!(
        traffic_findings(&o),
        [
            Finding::SeamQuiet {
                seam: "companion-server".into(),
                window_secs: 3600
            },
            Finding::UndeclaredTraffic {
                from: "rubato-companion".into(),
                to: "github".into(),
                calls: 4
            },
        ]
    );
    let gauge = |name: &str| {
        let p = reader.points(name);
        assert_eq!(p.len(), 1, "{name}: {p:?}");
        p[0].value
    };
    assert_eq!(gauge(metric::TRAFFIC_SEAMS_QUIET), 1);
    assert_eq!(gauge(metric::TRAFFIC_UNDECLARED_EDGES), 1);
    assert_eq!(gauge(metric::TRAFFIC_UNMAPPED_SERVICES), 1);
    assert!(gauge(metric::TRAFFIC_REPORT_AGE) < 60);
    assert_eq!(reader.points(metric::TRAFFIC_REPORT_AGE)[0].unit, "");

    // Rewritten with the same counts (a newer mtime): no fan-out.
    std::fs::write(&file, FIXTURE).unwrap();
    assert_eq!(fetcher.tick(&app, now()).await, Outcome::Ok);
    assert_eq!(
        drain(&mut web),
        Vec::<String>::new(),
        "same counts, no notify"
    );
    // A count changed: fan-out.
    std::fs::write(&file, FIXTURE.replace("1200", "1201")).unwrap();
    assert_eq!(fetcher.tick(&app, now()).await, Outcome::Ok);
    assert_eq!(drain(&mut web), ["traffic"]);

    // Outside the limits: rejected whole, with the reason, and the previous report kept.
    let edge = r#"{ "client": "web", "server": "rubato-server", "calls": 1 }"#;
    let many = format!(
        r#"{{ "window_secs": 3600, "edges": [{}] }}"#,
        vec![edge; 2_001].join(",")
    );
    let long = format!(
        r#"{{ "window_secs": 3600, "edges": [{{ "client": "{}", "server": "web", "calls": 1 }}] }}"#,
        "s".repeat(300)
    );
    for (body, reason) in [
        (many, "too_many_edges"),
        (FIXTURE.replace("3600", "30"), "bad_window"),
        (long, "name_too_long"),
        ("{ not json".to_owned(), "parse"),
    ] {
        std::fs::write(&file, body).unwrap();
        assert_eq!(fetcher.tick(&app, now()).await, Outcome::Rejected(reason));
        let o = overview().await;
        assert_eq!(o.traffic.state, TrafficState::Fresh, "{reason}");
        assert_eq!(
            o.traffic.seams.iter().map(|s| s.forward_calls).max(),
            Some(1201),
            "{reason}: the previous report is kept"
        );
        let logged = events.lock().unwrap().iter().any(|e| {
            e.message == "traffic report rejected"
                && e.fields.get(attr::DROP_REASON).map(String::as_str) == Some(reason)
        });
        assert!(logged, "{reason} logged");
    }
    assert_eq!(drain(&mut web), Vec::<String>::new());

    // The file is gone: failing, nothing drawn, no traffic findings, the last good time kept.
    std::fs::remove_file(&file).unwrap();
    assert_eq!(fetcher.tick(&app, now()).await, Outcome::Failed);
    assert_eq!(drain(&mut web), ["traffic"]);
    let o = overview().await;
    assert_eq!(o.traffic.state, TrafficState::Failing);
    assert!(
        o.traffic
            .error
            .as_deref()
            .unwrap()
            .contains("dev traffic file")
    );
    assert!(o.traffic.seams.is_empty() && o.traffic.undeclared.is_empty());
    assert!(o.traffic.observed_at.is_some());
    assert!(traffic_findings(&o).is_empty());
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.message == "traffic source failing")
    );

    // Back, but written an hour ago: stale.
    std::fs::write(&file, FIXTURE).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    assert_eq!(fetcher.tick(&app, now()).await, Outcome::Ok);
    let o = overview().await;
    assert_eq!(o.traffic.state, TrafficState::Stale);
    assert!(traffic_findings(&o).is_empty());
    assert!(gauge(metric::TRAFFIC_REPORT_AGE) >= 3600);
    assert_eq!(gauge(metric::TRAFFIC_SEAMS_QUIET), 0);
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.message == "traffic source recovered")
    );

    // Each outcome counted: four ok, four rejected, one failed.
    let fetches: Vec<(String, u64)> = reader
        .points(metric::TRAFFIC_FETCHES)
        .into_iter()
        .map(|p| (p.attrs[attr::OUTCOME].clone(), p.value))
        .collect();
    let count = |o: &str| fetches.iter().find(|(k, _)| k == o).map(|(_, v)| *v);
    assert_eq!(count("ok"), Some(4), "{fetches:?}");
    assert_eq!(count("rejected"), Some(4), "{fetches:?}");
    assert_eq!(count("failed"), Some(1), "{fetches:?}");
    println!("checked 9 fetches and {} overviews", 9);
}

/// No source configured: the overview says so and the task does not run.
#[tokio::test(flavor = "current_thread")]
async fn without_a_source_traffic_is_not_configured() {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let cfg = Config::for_tests(url, "m0x6A");
    let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
    let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    assert_eq!(
        app.traffic.read().await.source,
        rubato_protocol::SourceStatus::NotConfigured
    );
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let r = tokio::time::timeout(Duration::from_secs(1), super::run(app, rx)).await;
    assert!(r.expect("returns at once").is_ok());
}
