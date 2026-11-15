//! The Prometheus adapter (ADR 0040) against a fake query API on a free port, serving the
//! fixtures in `tests/fixtures/prometheus/`. Nothing of our own is mocked: the adapter makes real
//! HTTP requests and the fetch task judges what comes back.

use super::fetch::{Fetcher, Outcome};
use super::prometheus::{PrometheusConfig, PrometheusSource};
use crate::kernel::config::{Config, TrafficEnv};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::{TimeZone, Utc};
use rubato_protocol::traffic::MAX_EDGES;
use rubato_protocol::{SourceStatus, TrafficEdge};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const REQUEST: &str = include_str!("../../tests/fixtures/prometheus/request.json");
const FAILED: &str = include_str!("../../tests/fixtures/prometheus/failed.json");
const MATRIX: &str = include_str!("../../tests/fixtures/prometheus/matrix.json");
const STATUS_ERROR: &str = include_str!("../../tests/fixtures/prometheus/status-error.json");

/// Every answer served, across tests, so the run says how much it checked.
static SERVED: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
enum Mode {
    /// The requests and failures answers.
    Answer(String, String),
    Status(u16, &'static str),
    Slow,
}

/// One request the fake saw: its form fields and its authorization header.
type Seen = (HashMap<String, String>, Option<String>);

struct Fake {
    mode: Mutex<Mode>,
    seen: Mutex<Vec<Seen>>,
}

async fn query(
    State(f): State<Arc<Fake>>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    let form: HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let failed = form
        .get("query")
        .is_some_and(|q| q.contains("request_failed_total"));
    f.seen.lock().unwrap().push((form, auth));
    SERVED.fetch_add(1, Ordering::Relaxed);
    let mode = f.mode.lock().unwrap().clone();
    match mode {
        Mode::Answer(req, fail) => {
            let body = if failed { fail } else { req };
            ([("content-type", "application/json")], body).into_response()
        }
        Mode::Status(code, body) => (StatusCode::from_u16(code).unwrap(), body).into_response(),
        Mode::Slow => {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "{}".into_response()
        }
    }
}

async fn fake(mode: Mode) -> (String, Arc<Fake>) {
    let f = Arc::new(Fake {
        mode: Mutex::new(mode),
        seen: Mutex::new(vec![]),
    });
    let app = axum::Router::new()
        .route("/api/v1/query", axum::routing::post(query))
        .with_state(f.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), f)
}

fn config(url: &str, extra: &[(&str, &str)]) -> PrometheusConfig {
    let mut env: HashMap<String, String> = extra
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    env.insert("RUBATO_TRAFFIC_PROMETHEUS_URL".into(), url.into());
    PrometheusConfig::from_env(|k| env.get(k).cloned(), 60)
        .unwrap()
        .unwrap()
}

fn at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap()
}

fn edge<'a>(edges: &'a [TrafficEdge], client: &str, cns: Option<&str>) -> &'a TrafficEdge {
    edges
        .iter()
        .find(|e| e.client == client && e.client_namespace.as_deref() == cns)
        .unwrap_or_else(|| panic!("no edge from {client} in {cns:?}: {edges:?}"))
}

#[tokio::test]
async fn a_service_graph_answer_becomes_a_report_with_namespaces() {
    let (url, f) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let c = config(
        &url,
        &[
            (
                "RUBATO_TRAFFIC_PROMETHEUS_HEADERS",
                "Authorization=Bearer t0ken",
            ),
            ("RUBATO_TRAFFIC_WINDOW_SECS", "600"),
        ],
    );
    let r = PrometheusSource::new(c).report_at(at()).await.unwrap();
    assert_eq!(r.observed_at, at());
    assert_eq!(r.window_secs, 600);
    assert!(!r.truncated);
    assert!(r.validate().is_ok());
    // Zero and NaN samples make no edge; the rest are sorted by calls.
    assert_eq!(r.edges.len(), 5, "{:?}", r.edges);
    assert_eq!(r.edges[0].client, "web");
    let web = edge(&r.edges, "web", Some("rubato"));
    assert_eq!((web.calls, web.errors), (1200, 12));
    assert_eq!(web.server_namespace.as_deref(), Some("rubato"));
    // A missing or empty namespace label is no namespace: a virtual node from peer.service.
    assert_eq!(
        edge(&r.edges, "rubato-server", Some("rubato")).server_namespace,
        None
    );
    assert_eq!(
        edge(&r.edges, "rubato", Some("rubato")).server_namespace,
        None
    );
    // Failures are joined on all four labels and never exceed the requests.
    let shop = edge(&r.edges, "checkout", Some("shop"));
    assert_eq!((shop.calls, shop.errors), (5, 5));
    let test = edge(&r.edges, "checkout", Some("shop-test"));
    assert_eq!((test.calls, test.errors), (2, 0));

    let seen = f.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for (form, auth) in seen.iter() {
        assert_eq!(auth.as_deref(), Some("Bearer t0ken"));
        assert_eq!(form["time"], at().timestamp().to_string());
    }
    assert_eq!(
        seen[0].0["query"],
        format!(
            "topk({}, sum by (client, server, client_service_namespace, server_service_namespace) (increase(traces_service_graph_request_total[600s])))",
            MAX_EDGES + 1
        )
    );
    assert!(seen[1].0["query"].contains("traces_service_graph_request_failed_total[600s]"));
}

#[tokio::test]
async fn labels_and_prefix_come_from_config() {
    let (url, f) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let c = config(
        &url,
        &[
            ("RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS", "cns, sns"),
            (
                "RUBATO_TRAFFIC_PROMETHEUS_METRIC_PREFIX",
                "tempo_service_graph_",
            ),
        ],
    );
    let r = PrometheusSource::new(c).report_at(at()).await.unwrap();
    // The fixture's labels are not the configured ones, so no end has a namespace.
    assert!(r.edges.iter().all(|e| e.client_namespace.is_none()));
    let q = &f.seen.lock().unwrap()[0].0["query"];
    assert!(q.contains("sum by (client, server, cns, sns)"), "{q}");
    assert!(
        q.contains("increase(tempo_service_graph_request_total[3600s])"),
        "{q}"
    );
}

#[tokio::test]
async fn more_series_than_a_report_holds_are_cut_and_marked() {
    let result: Vec<String> = (0..=MAX_EDGES)
        .map(|i| {
            format!(
                r#"{{"metric":{{"client":"c{i}","server":"s","client_service_namespace":"n","server_service_namespace":"n"}},"value":[1794657600,"{}"]}}"#,
                i + 1
            )
        })
        .collect();
    let many = format!(
        r#"{{"status":"success","data":{{"resultType":"vector","result":[{}]}}}}"#,
        result.join(",")
    );
    let empty = r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;
    let (url, _) = fake(Mode::Answer(many, empty.into())).await;
    let r = PrometheusSource::new(config(&url, &[]))
        .report_at(at())
        .await
        .unwrap();
    assert!(r.truncated);
    assert_eq!(r.edges.len(), MAX_EDGES);
    assert_eq!(
        r.edges[0].calls,
        MAX_EDGES as u64 + 1,
        "the largest are kept"
    );
    assert!(r.validate().is_ok());
}

/// An app whose traffic source is configured, for the fetch task's verdicts.
async fn app(url: &str) -> (App, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let db = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let mut cfg = Config::for_tests(db, "m0x6A");
    cfg.traffic_env = TrafficEnv::from_vars(
        [("RUBATO_TRAFFIC_PROMETHEUS_URL".to_owned(), url.to_owned())].into_iter(),
    );
    let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
    (AppState::new(pool, cfg, Arc::new(FakeGitHub::new())), tmp)
}

async fn tick(mode: Mode, timeout: Option<Duration>) -> (Outcome, SourceStatus, bool) {
    let (url, _) = fake(mode).await;
    let (app, _tmp) = app(&url).await;
    let mut c = config(&url, &[]);
    if let Some(t) = timeout {
        c.timeout = t;
    }
    let mut f = Fetcher::new(PrometheusSource::new(c), Utc::now());
    let outcome = f.tick(&app, Utc::now()).await;
    let latest = app.traffic.read().await.clone();
    (outcome, latest.source, latest.report.is_some())
}

fn failing(s: &SourceStatus) -> Option<String> {
    match s {
        SourceStatus::Configured { error, .. } => error.clone(),
        SourceStatus::NotConfigured => panic!("configured"),
    }
}

#[tokio::test]
async fn the_fetch_task_judges_each_answer() {
    let (o, s, kept) = tick(Mode::Answer(REQUEST.into(), FAILED.into()), None).await;
    assert_eq!((o, failing(&s), kept), (Outcome::Ok, None, true));

    // The source did not answer usefully: failing, shown as unknown.
    let (o, s, kept) = tick(Mode::Status(500, "boom"), None).await;
    assert_eq!(o, Outcome::Failed);
    assert_eq!(
        failing(&s).as_deref(),
        Some("prometheus request_total: HTTP 500")
    );
    assert!(!kept);
    let (o, s, _) = tick(Mode::Status(400, STATUS_ERROR), None).await;
    assert_eq!(o, Outcome::Failed, "{s:?}");
    let (o, s, _) = tick(Mode::Status(200, STATUS_ERROR), None).await;
    assert_eq!(o, Outcome::Failed);
    assert!(failing(&s).unwrap().contains("unknown function"), "{s:?}");
    let (o, s, _) = tick(Mode::Slow, Some(Duration::from_millis(300))).await;
    assert_eq!(o, Outcome::Failed);
    assert_eq!(
        failing(&s).as_deref(),
        Some("prometheus request_total: timed out")
    );

    // It answered with something that is not a report: rejected, not failing.
    let (o, s, kept) = tick(Mode::Status(200, "not json"), None).await;
    assert_eq!(
        (o, failing(&s), kept),
        (Outcome::Rejected("parse"), None, false)
    );
    println!(
        "served {} fake query API answers",
        SERVED.load(Ordering::Relaxed)
    );
}

/// Must fail: a range query's matrix is not the instant vector the adapter asked for. A
/// lenient adapter that read it would report calls from a different window.
#[tokio::test]
async fn a_matrix_answer_is_rejected_as_parse() {
    let (o, s, kept) = tick(Mode::Answer(MATRIX.into(), MATRIX.into()), None).await;
    assert_eq!(o, Outcome::Rejected("parse"));
    assert_eq!(failing(&s), None);
    assert!(!kept);
}

/// A source set up wrong is never silent: the task logs it and the state turns failing, which
/// the map shows as unknown.
#[tokio::test]
async fn a_bad_source_config_makes_the_state_failing() {
    let tmp = tempfile::tempdir().unwrap();
    let db = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let mut cfg = Config::for_tests(db, "m0x6A");
    cfg.traffic_env = TrafficEnv::from_vars(
        [
            ("RUBATO_TRAFFIC_PROMETHEUS_URL", "http://prom:9090"),
            ("RUBATO_TRAFFIC_WINDOW_SECS", "5"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned())),
    );
    let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
    let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    let (_tx, rx) = tokio::sync::watch::channel(false);
    super::run(app.clone(), rx).await.unwrap();
    let e = failing(&app.traffic.read().await.source).expect("failing");
    assert!(e.contains("RUBATO_TRAFFIC_WINDOW_SECS"), "{e}");
}

#[test]
fn config_is_checked_and_headers_never_print() {
    let env = |pairs: &[(&str, &str)]| {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| m.get(k).cloned()
    };
    assert_eq!(PrometheusConfig::from_env(env(&[]), 60), Ok(None));
    let url = ("RUBATO_TRAFFIC_PROMETHEUS_URL", "http://prom:9090/");
    let c = PrometheusConfig::from_env(env(&[url]), 60)
        .unwrap()
        .unwrap();
    assert_eq!(c.url, "http://prom:9090");
    assert_eq!(c.window_secs, 3600);
    assert_eq!(c.timeout, Duration::from_secs(10));
    assert_eq!(
        (
            c.client_namespace_label.as_str(),
            c.server_namespace_label.as_str()
        ),
        ("client_service_namespace", "server_service_namespace")
    );
    // Both queries fit in the interval: at 15 s each gets 7 s.
    let c = PrometheusConfig::from_env(env(&[url]), 15)
        .unwrap()
        .unwrap();
    assert_eq!(c.timeout, Duration::from_secs(7));
    let secret = (
        "RUBATO_TRAFFIC_PROMETHEUS_HEADERS",
        "Authorization=Basic czNjcmV0",
    );
    let c = PrometheusConfig::from_env(env(&[url, secret]), 60)
        .unwrap()
        .unwrap();
    assert!(!format!("{c:?}").contains("czNjcmV0"), "{c:?}");
    for bad in [
        vec![("RUBATO_TRAFFIC_PROMETHEUS_URL", "prom:9090")],
        vec![("RUBATO_TRAFFIC_PROMETHEUS_URL", "ftp://prom")],
        vec![url, ("RUBATO_TRAFFIC_WINDOW_SECS", "30")],
        vec![url, ("RUBATO_TRAFFIC_WINDOW_SECS", "an hour")],
        vec![url, ("RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS", "one")],
        vec![url, ("RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS", "a b,c")],
        vec![
            url,
            ("RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS", "same,same"),
        ],
        vec![url, ("RUBATO_TRAFFIC_PROMETHEUS_METRIC_PREFIX", "x{y}")],
        vec![url, ("RUBATO_TRAFFIC_PROMETHEUS_HEADERS", "Basic czNjcmV0")],
    ] {
        let e = PrometheusConfig::from_env(env(&bad), 60).unwrap_err();
        assert!(!e.contains("czNjcmV0"), "{e}");
    }
}
