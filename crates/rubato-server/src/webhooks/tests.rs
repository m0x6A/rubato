//! The handler on its own route (design 208 D2, D4, D12), with the in-memory metric reader. The
//! route's place in the real router (404 when off, outside the same-origin layer) is tested in
//! `app.rs`; what a delivery makes the poll loop read, in `tests/webhooks.rs`.

use super::*;
use crate::kernel::config::Config;
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::AppState;
use crate::kernel::telemetry::metric_reader;
use axum::Router;
use axum::routing::post;
use rubato_protocol::otel::{attr, metric};
use std::sync::Arc;

const SECRET: &str = "test-webhook-secret";

struct Served {
    url: String,
    app: App,
    reader: metric_reader::Reader,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

async fn serve() -> Served {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = crate::kernel::db::connect(&url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut cfg = Config::for_tests(url, "m0x6A");
    cfg.github_webhook_secret =
        crate::kernel::config::WebhookSecret::from_env_value(Some(SECRET.into()));
    let mut app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let map: rubato_protocol::SystemMap =
        serde_yaml_ng::from_str(include_str!("../../../../docs/design/rubato-map.yml")).unwrap();
    app.set_map(
        rubato_protocol::DEFAULT_WORKSPACE,
        crate::kernel::state::LoadedMap {
            map,
            version: "v".into(),
            loaded_at: chrono::Utc::now(),
            yaml: String::new(),
        },
    )
    .await;
    // The route as `app.rs` mounts it.
    let router = Router::new()
        .route(
            PATH,
            post(receive).layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .with_state(app.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Served {
        url: base,
        app,
        reader,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

fn pr_closed(branch: &str) -> String {
    serde_json::json!({
        "action": "closed",
        "pull_request": {"number": 1005, "head": {"ref": branch}},
        "repository": {"full_name": "m0x6A/rubato"},
    })
    .to_string()
}

impl Served {
    async fn post(
        &self,
        id: &str,
        event: &str,
        body: &str,
        signature: Option<String>,
    ) -> (u16, String) {
        let mut req = self
            .http
            .post(format!("{}{PATH}", self.url))
            .header("x-github-delivery", id)
            .header("x-github-event", event)
            .header("content-type", "application/json")
            .body(body.to_owned());
        if let Some(s) = signature {
            req = req.header("x-hub-signature-256", s);
        }
        let r = req.send().await.unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    async fn signed(&self, id: &str, event: &str, body: &str) -> (u16, String) {
        self.post(
            id,
            event,
            body,
            Some(sign(SECRET.as_bytes(), body.as_bytes())),
        )
        .await
    }

    fn deliveries(&self, outcome: &str) -> u64 {
        self.reader
            .points(metric::GITHUB_WEBHOOK_DELIVERIES)
            .into_iter()
            .filter(|p| p.attrs.get(attr::OUTCOME).map(String::as_str) == Some(outcome))
            .map(|p| p.value)
            .sum()
    }

    async fn rows(&self) -> i64 {
        store::count(&self.app.db).await.unwrap()
    }
}

/// S2: a bad, missing or malformed signature is 401, stores nothing, wakes nothing, and counts.
#[tokio::test]
async fn a_bad_signature_is_refused_and_counted() {
    let s = serve().await;
    let body = pr_closed("issue/5-x");
    let wrong = sign(b"not the secret", body.as_bytes());
    let cases = [Some(wrong), None, Some("sha256=zz".to_owned())];
    for sig in &cases {
        let (status, text) = s.post("d-1", "pull_request", &body, sig.clone()).await;
        assert_eq!(status, 401, "{sig:?}");
        assert_eq!(text, r#"{"reason":"bad_signature"}"#);
    }
    assert_eq!(s.rows().await, 0, "nothing stored");
    assert!(pending(&s.app).await.is_empty());
    assert_eq!(s.deliveries("rejected"), cases.len() as u64);
    let woke = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        s.app.webhook_wake.notified(),
    )
    .await;
    assert!(woke.is_err(), "no wake for a refused delivery");
    // Fails on purpose if the check is skipped: the same body signed right is accepted.
    assert_eq!(s.signed("d-1", "pull_request", &body).await.0, 202);
    println!("checked {} refused signatures", cases.len());
}

/// S2: signed but not JSON, or without GitHub's headers, is 400; over 5 MiB is 413. All counted.
#[tokio::test]
async fn malformed_and_too_large_are_counted() {
    let s = serve().await;
    assert_eq!(s.signed("d-2", "issues", "not json").await.0, 400);
    let body = pr_closed("issue/5-x");
    let r = s
        .http
        .post(format!("{}{PATH}", s.url))
        .header("x-github-event", "pull_request")
        .header(
            "x-hub-signature-256",
            sign(SECRET.as_bytes(), body.as_bytes()),
        )
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400, "no X-GitHub-Delivery");
    assert_eq!(s.deliveries("malformed"), 2);
    let big = "x".repeat(6 * 1024 * 1024);
    assert_eq!(s.signed("d-3", "push", &big).await.0, 413);
    assert_eq!(s.deliveries("too_large"), 1);
    assert_eq!(s.rows().await, 0);
}

/// S4: the same id again, or the same body under a new id, is a duplicate: one row, one wake.
#[tokio::test]
async fn a_redelivery_changes_nothing() {
    let s = serve().await;
    let body = pr_closed("issue/5-x");
    assert_eq!(
        s.signed("d-4", "pull_request", &body).await,
        (202, r#"{"outcome":"accepted"}"#.to_owned())
    );
    s.app.webhook_wake.notified().await;
    assert_eq!(
        s.signed("d-4", "pull_request", &body).await,
        (200, r#"{"outcome":"duplicate"}"#.to_owned())
    );
    assert_eq!(
        s.signed("d-5", "check_suite", &body).await,
        (200, r#"{"outcome":"duplicate"}"#.to_owned()),
        "a captured body under fresh headers"
    );
    assert_eq!(s.rows().await, 1);
    let p = pending(&s.app).await;
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].target.issue.as_ref().map(|i| i.number), Some(5));
    assert!(p[0].target.list_pulls);
    assert_eq!(s.deliveries("accepted"), 1);
    assert_eq!(s.deliveries("duplicate"), 2);
    let woke = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        s.app.webhook_wake.notified(),
    )
    .await;
    assert!(woke.is_err(), "a duplicate does not wake the loop");
}

/// D12: ignored deliveries are stored and answered 200, and do not move `last_accepted_at`; the
/// gauges are recorded only while webhooks are on.
#[tokio::test]
async fn ignored_deliveries_and_the_gauges() {
    let s = serve().await;
    let ping = r#"{"zen":"Design for failure.","repository":{"full_name":"m0x6A/rubato"}}"#;
    assert_eq!(
        s.signed("d-6", "ping", ping).await,
        (200, r#"{"outcome":"ignored"}"#.to_owned())
    );
    let other =
        r#"{"action":"opened","issue":{"number":1},"repository":{"full_name":"someone/else"}}"#;
    assert_eq!(s.signed("d-7", "issues", other).await.0, 200);
    record(&s.app).await;
    let last = s.reader.points(metric::GITHUB_WEBHOOK_LAST_ACCEPTED_AT);
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].value, 0, "nothing accepted yet");
    assert_eq!(last[0].unit, "", "no unit, so no _seconds suffix");
    assert_eq!(
        s.signed("d-8", "pull_request", &pr_closed("issue/5-x"))
            .await
            .0,
        202
    );
    record(&s.app).await;
    let last = s.reader.points(metric::GITHUB_WEBHOOK_LAST_ACCEPTED_AT);
    assert!(last[0].value > 0);
    let pending_age = s.reader.points(metric::GITHUB_WEBHOOK_PENDING_OLDEST);
    assert_eq!(pending_age.len(), 1);
    let by_event: Vec<String> = s
        .reader
        .points(metric::GITHUB_WEBHOOK_DELIVERIES)
        .into_iter()
        .filter_map(|p| p.attrs.get(attr::GITHUB_EVENT).cloned())
        .collect();
    assert!(by_event.contains(&"ping".to_owned()) && by_event.contains(&"pull_request".to_owned()));
}
