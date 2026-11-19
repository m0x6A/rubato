//! The relay (ADR 0082): the rewrite lets nothing through but what it names, the refusals come
//! in order and are counted once each, and a batch reaches the OTLP endpoint as `web` with the
//! configured headers. The OTLP receiver is a real HTTP server on a free port.

use super::*;
use crate::kernel::config::{Config, OtlpHeaders};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::AppState;
use crate::kernel::telemetry::metric_reader;
use axum::http::HeaderMap;
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::telemetry_relay::{MAX_BODY_BYTES, MAX_SPANS, routes::TELEMETRY_TRACES};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NOW: u128 = 1_763_560_000_000_000_000;
const TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

fn span(span_id: &str, route: &str) -> Value {
    json!({
        "traceId": TRACE,
        "spanId": span_id,
        "name": "anything the browser likes",
        "kind": 3,
        "startTimeUnixNano": (NOW - 5_000_000).to_string(),
        "endTimeUnixNano": NOW.to_string(),
        "attributes": [
            {"key": "http.request.method", "value": {"stringValue": "GET"}},
            {"key": "url.path", "value": {"stringValue": route}},
            {"key": "http.response.status_code", "value": {"intValue": "200"}},
            {"key": "user.secret", "value": {"stringValue": "do not forward"}},
        ],
        "status": {"code": 2, "message": "free text that must stay behind"},
    })
}

fn batch(spans: Vec<Value>, service: &str) -> Value {
    json!({"resourceSpans": [{
        "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": service}}]},
        "scopeSpans": [{"scope": {"name": "web"}, "spans": spans}],
    }]})
}

fn spans_of(v: &Value) -> Vec<Value> {
    v["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn the_rewrite_makes_web_spans_with_only_what_it_names() {
    // The browser claims to be the server: it becomes web, in Rubato's namespace.
    let r = rewrite(
        &batch(
            vec![span("00f067aa0ba902b7", "/api/w/{ws}/pins")],
            "rubato-server",
        ),
        "rubato",
        NOW,
    )
    .unwrap();
    assert_eq!((r.kept, r.dropped), (1, 0));
    let resource = &r.body["resourceSpans"][0]["resource"]["attributes"];
    assert_eq!(
        resource,
        &json!([
            {"key": "service.name", "value": {"stringValue": "web"}},
            {"key": "service.namespace", "value": {"stringValue": "rubato"}},
        ])
    );
    let s = &spans_of(&r.body)[0];
    assert_eq!(s["name"], "GET /api/w/{ws}/pins");
    assert_eq!(s["traceId"], TRACE);
    assert_eq!(s["status"], json!({"code": 2}), "the message stays behind");
    let keys: Vec<&str> = s["attributes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "http.request.method",
            "url.path",
            "http.response.status_code"
        ]
    );
    assert!(
        !r.body.to_string().contains("free text") && !r.body.to_string().contains("do not forward")
    );
}

#[test]
fn spans_the_relay_cannot_vouch_for_are_dropped() {
    let mut server_kind = span("00f067aa0ba902b1", "/api/overview");
    server_kind["kind"] = json!(2);
    let mut far = span("00f067aa0ba902b2", "/api/overview");
    far["startTimeUnixNano"] = json!("1000");
    let mut bad_id = span("not-hex-at-all!!", "/api/overview");
    bad_id["traceId"] = json!("00000000000000000000000000000000");
    let spans = vec![
        span("00f067aa0ba902b0", "/api/overview"),
        span("00f067aa0ba902b3", "/api/issues/916/whatever"),
        span("00f067aa0ba902b4", "https://elsewhere.example/api/x"),
        span("00f067aa0ba902b5", TELEMETRY_TRACES),
        server_kind,
        far,
        bad_id,
    ];
    let r = rewrite(&batch(spans, "web"), "rubato", NOW).unwrap();
    assert_eq!((r.kept, r.dropped), (1, 6), "{}", r.body);
}

#[test]
fn a_post_that_is_not_a_batch_is_refused_whole() {
    assert_eq!(
        rewrite(&json!({"hello": 1}), "rubato", NOW),
        Err(Invalid::NotOtlp)
    );
    assert_eq!(
        rewrite(&batch(vec![], "web"), "rubato", NOW),
        Err(Invalid::NoSpans)
    );
    let many = (0..=MAX_SPANS)
        .map(|i| span(&format!("{:016x}", i + 1), "/api/overview"))
        .collect();
    assert_eq!(
        rewrite(&batch(many, "web"), "rubato", NOW),
        Err(Invalid::TooMany)
    );
}

#[test]
fn route_templates_are_words_and_placeholders_only() {
    for ok in [
        "/api/overview",
        "/api/w/{ws}/pins",
        "/api/pulls/{owner}/{repo}/{number}",
        "/api/launches/{id}/messages",
    ] {
        assert!(is_route(ok), "{ok}");
    }
    for bad in [
        "/api/issues/916",
        "/api/Overview",
        "/overview",
        "/api/{}",
        "/api/a b",
        "/api/x?y=1",
        TELEMETRY_TRACES,
    ] {
        assert!(!is_route(bad), "{bad}");
    }
}

#[test]
fn the_bucket_lets_a_burst_through_and_refills() {
    let t0 = Instant::now();
    let login = "bucket-test";
    let first: Vec<bool> = (0..5).map(|_| take_token(login, t0)).collect();
    assert_eq!(first, [true, true, true, true, false], "a burst of four");
    assert!(
        take_token(login, t0 + Duration::from_millis(2_000)),
        "one more after two seconds"
    );
    assert!(!take_token(login, t0 + Duration::from_millis(2_100)));
    // Another login has its own bucket.
    assert!(take_token("bucket-test-2", t0));
}

/// What the fake OTLP receiver got: each body and its authorization header.
type Got = Arc<Mutex<Vec<(Value, Option<String>)>>>;

async fn receiver() -> (String, Got) {
    let got: Got = Arc::default();
    let g = got.clone();
    let app = axum::Router::new().route(
        "/otlp/v1/traces",
        axum::routing::post(move |headers: HeaderMap, body: String| {
            let g = g.clone();
            async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                g.lock()
                    .unwrap()
                    .push((serde_json::from_str(&body).unwrap(), auth));
                "{}"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/otlp"), got)
}

/// The server with the relay, as `login` (none: no login), exporting to `endpoint`.
async fn serve(
    login: Option<&str>,
    endpoint: Option<&str>,
) -> (String, metric_reader::Reader, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = crate::kernel::db::connect(&url).await.unwrap();
    let mut cfg = Config::for_tests(url, login.unwrap_or("nobody"));
    cfg.dev_user = login.map(str::to_owned);
    cfg.otel_endpoint = endpoint.map(str::to_owned);
    cfg.otel_headers = OtlpHeaders::parse("Authorization=Basic%20dG9rZW4=");
    let (reader, metrics) = metric_reader::start();
    let mut app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route(TELEMETRY_TRACES, axum::routing::post(relay))
                .with_state(app),
        )
        .await
        .unwrap()
    });
    (format!("http://{addr}{TELEMETRY_TRACES}"), reader, tmp)
}

fn outcomes(reader: &metric_reader::Reader) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = reader
        .points(metric::TELEMETRY_RELAY)
        .into_iter()
        .map(|p| (p.attrs[attr::OUTCOME].clone(), p.value))
        .collect();
    v.sort();
    v
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[tokio::test]
async fn a_batch_reaches_the_endpoint_as_web_with_the_servers_headers() {
    let (endpoint, got) = receiver().await;
    let (url, reader, _tmp) = serve(Some("relay-forward"), Some(&endpoint)).await;
    let mut s = span("00f067aa0ba902b7", "/api/overview");
    s["startTimeUnixNano"] = json!((now_nanos() - 1_000_000).to_string());
    s["endTimeUnixNano"] = json!(now_nanos().to_string());
    let res = reqwest::Client::new()
        .post(&url)
        .json(&batch(vec![s], "web"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);
    for _ in 0..200 {
        if !got.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let got = got.lock().unwrap();
    assert_eq!(got.len(), 1);
    let (body, auth) = &got[0];
    assert_eq!(auth.as_deref(), Some("Basic dG9rZW4="));
    assert_eq!(
        body["resourceSpans"][0]["resource"]["attributes"][0]["value"]["stringValue"],
        "web"
    );
    assert_eq!(spans_of(body)[0]["name"], "GET /api/overview");
    assert_eq!(outcomes(&reader), [("forwarded".to_owned(), 1)]);
}

#[tokio::test]
async fn refusals_come_in_order_and_each_counts_once() {
    let (endpoint, got) = receiver().await;
    let post = |url: String, body: Vec<u8>| async move {
        reqwest::Client::new()
            .post(url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    };
    // Off: no OTLP endpoint, whoever asks.
    let (url, reader, _t1) = serve(Some("relay-off"), None).await;
    assert_eq!(post(url, b"{}".to_vec()).await, 404);
    assert_eq!(outcomes(&reader), [("off".to_owned(), 1)]);
    // No login.
    let (url, reader, _t2) = serve(None, Some(&endpoint)).await;
    assert_eq!(post(url, b"{}".to_vec()).await, 401);
    assert_eq!(outcomes(&reader), [("unauthenticated".to_owned(), 1)]);
    // Too large, not a batch, then past the bucket of four.
    let (url, reader, _t3) = serve(Some("relay-limits"), Some(&endpoint)).await;
    assert_eq!(post(url.clone(), vec![b' '; MAX_BODY_BYTES + 1]).await, 413);
    assert_eq!(post(url.clone(), b"[1,2,3]".to_vec()).await, 400);
    assert_eq!(
        post(
            url.clone(),
            serde_json::to_vec(&batch(vec![], "web")).unwrap()
        )
        .await,
        400
    );
    assert_eq!(
        post(
            url.clone(),
            serde_json::to_vec(&batch(vec![span("00f067aa0ba902b7", "/api/x/1")], "web")).unwrap()
        )
        .await,
        202
    );
    // The 413 came before the bucket; the two 400s and the 202 took three tokens, this the fourth.
    assert_eq!(post(url.clone(), b"{}".to_vec()).await, 400);
    assert_eq!(post(url.clone(), b"{}".to_vec()).await, 429);
    assert_eq!(
        outcomes(&reader),
        [
            ("forwarded".to_owned(), 1),
            ("invalid".to_owned(), 3),
            ("rate_limited".to_owned(), 1),
            ("too_large".to_owned(), 1)
        ]
    );
    // The one forwarded post kept no span (its route has a number): nothing was exported.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(got.lock().unwrap().is_empty());
    assert_eq!(
        reader
            .points(metric::TELEMETRY_RELAY_DROPPED)
            .iter()
            .map(|p| p.value)
            .sum::<u64>(),
        1
    );
}

#[test]
fn otlp_headers_print_their_names_only() {
    let h = OtlpHeaders::parse("Authorization=Basic%20c2VjcmV0+/=, X-Scope=team");
    assert_eq!(format!("{h:?}"), r#"["Authorization", "X-Scope"]"#);
    let pairs: Vec<(&str, &str)> = h.pairs().collect();
    assert_eq!(
        pairs,
        [("Authorization", "Basic c2VjcmV0+/="), ("X-Scope", "team")]
    );
}
