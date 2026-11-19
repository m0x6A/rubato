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

/// How the fake answers a lookback's queries (ADR 0083), apart from the window's.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// As the window's.
    Same,
    /// The 7 d query never answers in time.
    HangWeek,
    /// Every lookback query fails with this status.
    Status(u16),
}

struct Fake {
    mode: Mutex<Mode>,
    look: Mutex<Look>,
    seen: Mutex<Vec<Seen>>,
}

/// Whether a query is a lookback's: over 6 h, 1 d or 7 d rather than a window.
fn is_lookback(q: &str) -> bool {
    rubato_protocol::traffic::LOOKBACK_SECS
        .iter()
        .any(|s| q.contains(&format!("[{s}s]")))
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
    let q = form.get("query").cloned().unwrap_or_default();
    f.seen.lock().unwrap().push((form, auth));
    SERVED.fetch_add(1, Ordering::Relaxed);
    if is_lookback(&q) {
        let look = *f.look.lock().unwrap();
        match look {
            Look::Same => {}
            Look::HangWeek if q.contains("[604800s]") => {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Look::HangWeek => {}
            Look::Status(code) => {
                return (StatusCode::from_u16(code).unwrap(), "no").into_response();
            }
        }
    }
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
        look: Mutex::new(Look::Same),
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

/// Issue #288: a source that lists namespaces asks only for calls with an end in one of them,
/// and one that lists none asks as before. Must fail on purpose if the list is dropped.
#[tokio::test]
async fn listed_namespaces_narrow_the_query() {
    let (url, f) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let c = config(
        &url,
        &[(
            "RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES",
            " rubato, shop.eu ,rubato",
        )],
    );
    assert_eq!(c.namespaces, ["rubato", "shop.eu"], "trimmed, once each");
    PrometheusSource::new(c).report_at(at()).await.unwrap();
    let seen = f.seen.lock().unwrap();
    assert_eq!(
        seen[0].0["query"],
        format!(
            "topk({}, sum by (client, server, client_service_namespace, server_service_namespace) (increase(traces_service_graph_request_total{{client_service_namespace=~\"rubato|shop\\\\.eu\"}}[3600s]) or increase(traces_service_graph_request_total{{server_service_namespace=~\"rubato|shop\\\\.eu\"}}[3600s])))",
            MAX_EDGES + 1
        )
    );
    assert!(seen[1].0["query"].contains("request_failed_total{client_service_namespace=~"));
    let none = config(&url, &[("RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES", " , ")]);
    assert!(none.namespaces.is_empty());
    assert!(!none.query("request_total").contains('{'), "no matcher");
    println!("checked 2 queries with a list and 1 without");
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
    let latest = app.traffic_of(rubato_protocol::DEFAULT_WORKSPACE).await;
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

/// A source set up wrong is never silent: the supervisor logs it and the workspace's state
/// turns failing, which the map shows as unknown. Its fields are read with the same rules as the
/// variables they were seeded from, so the error names the variable (ADR 0075).
#[tokio::test]
async fn a_bad_source_config_makes_the_state_failing() {
    let (app, _tmp) = app("http://prom:9090").await;
    set_source(
        &app,
        rubato_protocol::DEFAULT_WORKSPACE,
        &[("url", "http://prom:9090"), ("window_secs", "5")],
        1,
    );
    let (tx, rx) = tokio::sync::watch::channel(false);
    let supervisor = tokio::spawn(super::run(app.clone(), rx));
    let e = eventually(|| {
        let app = app.clone();
        async move {
            failing(
                &app.traffic_of(rubato_protocol::DEFAULT_WORKSPACE)
                    .await
                    .source,
            )
        }
    })
    .await;
    assert!(e.contains("RUBATO_TRAFFIC_WINDOW_SECS"), "{e}");
    tx.send(true).unwrap();
    supervisor.await.unwrap().unwrap();
}

/// Give `ws` a Prometheus source with `fields` at `revision`, as the settings slice would.
fn set_source(app: &App, ws: &str, fields: &[(&str, &str)], revision: u64) {
    use crate::kernel::workspaces::{Registry, TrafficSetting};
    let (mut entries, source) = app.workspace_entries();
    let mut found = false;
    for w in &mut entries {
        if w.id == ws {
            found = true;
            w.settings.revision = revision;
            w.settings.traffic = Some(TrafficSetting {
                source: "prometheus".into(),
                fields: fields
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                secrets_sealed: None,
            });
        }
    }
    assert!(found, "{ws} is held");
    app.set_workspaces(Registry { entries, source });
}

/// Poll `f` until it gives a value, for at most five seconds.
async fn eventually<T, F, Fut>(f: F) -> T
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(std::time::Instant::now() < deadline, "not in time");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
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
        vec![
            url,
            ("RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES", "a\"} or up{x=\""),
        ],
        vec![url, ("RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES", "shop|.*")],
        vec![
            url,
            (
                "RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES",
                "n1,n2,n3,n4,n5,n6,n7,n8,n9,n10,n11,n12,n13,n14,n15,n16,n17,n18,n19,n20,n21",
            ),
        ],
        vec![url, ("RUBATO_TRAFFIC_PROMETHEUS_HEADERS", "Basic czNjcmV0")],
    ] {
        let e = PrometheusConfig::from_env(env(&bad), 60).unwrap_err();
        assert!(!e.contains("czNjcmV0"), "{e}");
    }
}

/// Plan 189 N5: the fields and secrets a workspace is seeded with describe the same source as
/// the environment did, so the switch of the settings' origin (PR 2) cannot change it. Each
/// field maps back to its variable; an unset variable is not copied.
#[test]
fn the_seeded_fields_describe_the_source_the_environment_did() {
    use super::prometheus::{FIELDS, SECRETS, seed_from_env};
    let vars: HashMap<&str, &str> = [
        ("RUBATO_TRAFFIC_PROMETHEUS_URL", "http://prom.lan:9090/"),
        (
            "RUBATO_TRAFFIC_PROMETHEUS_HEADERS",
            "X-Scope-OrgID=shop,Authorization=Bearer czNjcmV0",
        ),
        (
            "RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS",
            "client_ns,server_ns",
        ),
        ("RUBATO_TRAFFIC_WINDOW_SECS", "1800"),
    ]
    .into();
    let get = |k: &str| vars.get(k).map(|v| (*v).to_owned());
    let seed = seed_from_env(get).expect("a URL is set");
    assert_eq!(seed.source, "prometheus");
    assert!(
        !seed.fields.contains_key("metric_prefix"),
        "unset: not copied"
    );
    // Back from the seed to the variables, through the adapter's own mapping.
    let back = |k: &str| {
        FIELDS
            .iter()
            .find(|(_, v)| *v == k)
            .and_then(|(f, _)| seed.fields.get(*f).cloned())
            .or_else(|| {
                SECRETS
                    .iter()
                    .find(|(_, v)| *v == k)
                    .and_then(|(f, _)| seed.secrets.get(*f).cloned())
            })
    };
    assert_eq!(
        PrometheusConfig::from_env(back, 60).unwrap(),
        PrometheusConfig::from_env(get, 60).unwrap()
    );
    assert!(
        !format!("{seed:?}").contains("czNjcmV0"),
        "Debug prints no secret"
    );
    assert_eq!(seed_from_env(|_| None), None, "no URL, no source");
    println!(
        "checked {} fields and {} secret",
        seed.fields.len(),
        seed.secrets.len()
    );
}

/// Plan 189 S6: each workspace reads its own source. Changing one workspace's source restarts
/// only its task, a report from the cancelled generation never lands, and clearing a source
/// records `rubato.traffic.configured` 0 for it (ADR 0075).
#[tokio::test(flavor = "current_thread")]
async fn each_workspace_reads_its_own_source() {
    use crate::kernel::telemetry::metric_reader;
    use crate::kernel::workspaces::Registry;
    use rubato_protocol::otel::{attr, metric};
    let (url_a, fake_a) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let (url_b, fake_b) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let (url_c, fake_c) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let tmp = tempfile::tempdir().unwrap();
    let db = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let mut cfg = Config::for_tests(db, "m0x6A");
    // A local server (ADR 0064): its sources may reach the fakes on loopback.
    cfg.public_url = "http://127.0.0.1:18999".into();
    let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let (mut entries, source) = app.workspace_entries();
    let mut shop = entries[0].clone();
    shop.id = "shop".into();
    shop.map_path = "shop.yml".into();
    entries.push(shop);
    app.set_workspaces(Registry { entries, source });
    set_source(&app, "default", &[("url", &url_a)], 1);
    set_source(&app, "shop", &[("url", &url_b)], 1);
    let seen = |f: &Arc<Fake>| f.seen.lock().unwrap().len();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let supervisor = tokio::spawn(super::run(app.clone(), rx));
    let reported = |ws: &'static str| {
        let app = app.clone();
        async move { app.traffic_of(ws).await.report.map(|_| ()) }
    };
    eventually(|| reported("default")).await;
    eventually(|| reported("shop")).await;
    assert!(
        seen(&fake_a) >= 2 && seen(&fake_b) >= 2,
        "each queried its own"
    );
    let gens = app.slots.get::<super::Generations>();
    let (before_default, before_shop) = (gens.of("default"), gens.of("shop"));

    // shop moves to another Prometheus: only shop's task restarts.
    set_source(&app, "shop", &[("url", &url_c)], 2);
    eventually(|| async { (seen(&fake_c) >= 2).then_some(()) }).await;
    assert_eq!(gens.of("default"), before_default, "default untouched");
    assert_ne!(gens.of("shop"), before_shop, "shop restarted");
    // A fetcher of shop's cancelled generation writes nothing.
    let stale = super::Fetcher::new(PrometheusSource::new(config(&url_b, &[])), Utc::now())
        .feeding(
            super::Targets::One("shop".into()),
            60,
            Some((gens.clone(), before_shop.unwrap())),
        );
    let mut stale = stale;
    let before = seen(&fake_b);
    assert_eq!(stale.tick(&app, Utc::now()).await, Outcome::Superseded);
    assert!(seen(&fake_b) > before, "it fetched, and wrote nothing");

    // shop's source is cleared: not configured, and the gauge says 0.
    let (mut entries, source) = app.workspace_entries();
    entries[1].settings.traffic = None;
    entries[1].settings.revision = 3;
    app.set_workspaces(Registry { entries, source });
    eventually(|| {
        let app = app.clone();
        async move {
            matches!(
                app.traffic_of("shop").await.source,
                SourceStatus::NotConfigured
            )
            .then_some(())
        }
    })
    .await;
    let configured = |ws: &str| {
        reader
            .points(metric::TRAFFIC_CONFIGURED)
            .into_iter()
            .find(|p| {
                p.attrs.get(attr::WORKSPACE).map(String::as_str) == (ws != "default").then_some(ws)
            })
            .map(|p| p.value)
    };
    assert_eq!(configured("shop"), Some(0));
    assert_eq!(configured("default"), Some(1));
    tx.send(true).unwrap();
    supervisor.await.unwrap().unwrap();
    println!(
        "checked 3 sources over 2 workspaces: {} answers",
        seen(&fake_a) + seen(&fake_b) + seen(&fake_c)
    );
}

/// Plan 189 S4 (ADR 0075): a guarded source refuses a name that resolves to loopback and an IP
/// literal on loopback, as a failing source; a local server may reach them; a redirect is not
/// followed.
#[tokio::test]
async fn every_connect_is_checked() {
    use super::Source as _;
    let (url, _f) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let port = url.rsplit(':').next().unwrap().to_owned();
    let failed = |r: Result<rubato_protocol::TrafficReport, super::SourceError>| match r {
        Err(super::SourceError::Failed(e)) => e,
        other => panic!("{other:?}"),
    };
    let named = format!("http://localhost:{port}");
    for target in [named.as_str(), url.as_str()] {
        let e = failed(
            PrometheusSource::guarded(config(target, &[]), false)
                .fetch()
                .await,
        );
        assert!(e.contains("address refused"), "{target}: {e}");
        assert!(
            PrometheusSource::guarded(config(target, &[]), true)
                .fetch()
                .await
                .is_ok(),
            "{target}: a local server may reach it"
        );
    }
    let (moved, _m) = fake(Mode::Status(302, "")).await;
    let e = failed(
        PrometheusSource::guarded(config(&moved, &[]), true)
            .fetch()
            .await,
    );
    assert!(e.contains("redirects are not followed"), "{e}");
    println!("checked 2 refused targets, 2 allowed, 1 redirect");
}

/// Lookback queries the fake has answered so far.
fn lookbacks(f: &Fake) -> usize {
    f.seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(form, _)| form.get("query").is_some_and(|q| is_lookback(q)))
        .count()
}

/// ADR 0083: every edge any lookback saw, in the window or not, with the shortest lookback; the
/// longest one asked. A one-hour window asks 6 h, 1 d and 7 d.
#[tokio::test]
async fn a_lookback_keeps_every_edge_it_saw_with_the_shortest_lookback() {
    let (url, f) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let l = PrometheusSource::new(config(&url, &[]))
        .lookback_at(at())
        .await
        .unwrap()
        .expect("a lookback");
    assert_eq!(l.lookback_secs, 604_800);
    // The fixture's five edges with calls, each first seen in the 6 h lookback.
    assert_eq!(l.seen.len(), 5, "{:?}", l.seen);
    assert!(
        l.seen.iter().all(|e| e.within_secs == 21_600),
        "{:?}",
        l.seen
    );
    assert!(
        l.seen.iter().any(|e| e.client == "web"),
        "edges in the window are kept too"
    );
    assert_eq!(lookbacks(&f), 3, "6 h, 1 d and 7 d");
    // A window of a day skips the lookbacks no longer than it.
    let (url, f) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let day = config(&url, &[("RUBATO_TRAFFIC_WINDOW_SECS", "86400")]);
    let l = PrometheusSource::new(day)
        .lookback_at(at())
        .await
        .unwrap()
        .unwrap();
    assert!(l.seen.iter().all(|e| e.within_secs == 604_800));
    assert_eq!(lookbacks(&f), 1);
}

/// ADR 0083: the lookback runs on the first tick and then every tenth, after the window's
/// report is written; the report carries its answer.
#[tokio::test]
async fn the_lookback_runs_on_the_first_tick_and_every_tenth() {
    let (url, f) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    let (app, _tmp) = app(&url).await;
    let mut fetcher = Fetcher::new(PrometheusSource::new(config(&url, &[])), Utc::now());
    let mut asked = Vec::new();
    for _ in 0..11 {
        assert_eq!(fetcher.tick(&app, Utc::now()).await, Outcome::Ok);
        asked.push(lookbacks(&f));
    }
    assert_eq!(
        asked,
        vec![3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 6],
        "first tick, then the eleventh"
    );
    let latest = app.traffic_of(rubato_protocol::DEFAULT_WORKSPACE).await;
    let report = latest.report.unwrap();
    assert_eq!(report.lookback_secs, Some(604_800));
    assert_eq!(report.seen.len(), 5);
    assert!(report.validate().is_ok());
}

/// Must not break (plan #300): a 7 d query that hangs never makes the window late or failing.
/// The lookback times out under the fetch's timeout; the report is fresh and claims nothing.
#[tokio::test]
async fn a_hanging_lookback_leaves_the_window_fresh_and_claims_nothing() {
    let (url, f) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    *f.look.lock().unwrap() = Look::HangWeek;
    let (app, _tmp) = app(&url).await;
    let mut fetcher = Fetcher::new(PrometheusSource::new(config(&url, &[])), Utc::now())
        .with_timeout(Duration::from_millis(500));
    let started = std::time::Instant::now();
    assert_eq!(fetcher.tick(&app, Utc::now()).await, Outcome::Ok);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let latest = app.traffic_of(rubato_protocol::DEFAULT_WORKSPACE).await;
    assert_eq!(failing(&latest.source), None);
    let report = latest.report.expect("the window's report was written");
    assert_eq!(report.lookback_secs, None, "no answer, no claim");
    assert!(report.seen.is_empty());
}

/// A failing lookback keeps the previous answer; with none, nothing is claimed.
#[tokio::test]
async fn a_failing_lookback_keeps_the_previous_answer() {
    let (url, f) = fake(Mode::Answer(REQUEST.into(), FAILED.into())).await;
    *f.look.lock().unwrap() = Look::Status(500);
    let (app, _tmp) = app(&url).await;
    let mut fetcher = Fetcher::new(PrometheusSource::new(config(&url, &[])), Utc::now());
    assert_eq!(fetcher.tick(&app, Utc::now()).await, Outcome::Ok);
    let report = |app: &App| {
        let app = app.clone();
        async move {
            app.traffic_of(rubato_protocol::DEFAULT_WORKSPACE)
                .await
                .report
                .unwrap()
        }
    };
    assert_eq!(
        report(&app).await.lookback_secs,
        None,
        "failing since start: unknown"
    );
    *f.look.lock().unwrap() = Look::Same;
    for _ in 0..10 {
        fetcher.tick(&app, Utc::now()).await;
    }
    assert_eq!(report(&app).await.lookback_secs, Some(604_800));
    *f.look.lock().unwrap() = Look::Status(500);
    for _ in 0..10 {
        fetcher.tick(&app, Utc::now()).await;
    }
    let r = report(&app).await;
    assert_eq!(r.lookback_secs, Some(604_800), "the previous answer stays");
    assert_eq!(r.seen.len(), 5);
}
