//! `POST /api/telemetry/traces` (ADR 0082): refused, or rewritten and forwarded to the server's
//! own OTLP traces endpoint. The browser hears 202 once the batch is accepted; how the export
//! went is a counter and a log line, not the browser's business.

use super::rewrite::{Invalid, rewrite};
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::StatusCode;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::telemetry_relay::{BUCKET_BURST, BUCKET_REFILL_MS, MAX_BODY_BYTES};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How long one export may take.
pub const EXPORT_TIMEOUT: Duration = Duration::from_secs(5);
/// The most logins the bucket remembers; past it, the oldest is forgotten.
const MAX_BUCKETS: usize = 1_000;

/// Per login, the relay's own bucket (`kernel::limits::Buckets`, ADR 0090).
fn buckets() -> &'static crate::kernel::limits::Buckets {
    static B: OnceLock<crate::kernel::limits::Buckets> = OnceLock::new();
    B.get_or_init(|| {
        crate::kernel::limits::Buckets::with_max(BUCKET_BURST, BUCKET_REFILL_MS, MAX_BUCKETS)
    })
}

/// Take one token for `login`, refilling one every `BUCKET_REFILL_MS` up to `BUCKET_BURST`.
pub fn take_token(login: &str, now: Instant) -> bool {
    buckets().take(login, now).is_ok()
}

fn count(app: &App, outcome: &'static str) {
    app.metrics
        .telemetry_relay
        .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
}

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(EXPORT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    })
}

/// The route. Refusals in order: off (404), no login (401), too large (413), past the bucket
/// (429), not OTLP/JSON or no spans or too many (400).
pub async fn relay(State(app): State<App>, req: Request) -> StatusCode {
    let Some(endpoint) = app.config.otel_endpoint.clone() else {
        count(&app, "off");
        return StatusCode::NOT_FOUND;
    };
    let (mut parts, body) = req.into_parts();
    let Ok(CurrentUser(login)) = CurrentUser::from_request_parts(&mut parts, &app).await else {
        count(&app, "unauthenticated");
        return StatusCode::UNAUTHORIZED;
    };
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY_BYTES).await else {
        count(&app, "too_large");
        return StatusCode::PAYLOAD_TOO_LARGE;
    };
    if !take_token(&login, Instant::now()) {
        count(&app, "rate_limited");
        return StatusCode::TOO_MANY_REQUESTS;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let rewritten = serde_json::from_slice(&bytes)
        .map_err(|_| Invalid::NotOtlp)
        .and_then(|v| rewrite(&v, &crate::kernel::telemetry::own_namespace(), now));
    let r = match rewritten {
        Ok(r) => r,
        Err(_) => {
            count(&app, "invalid");
            return StatusCode::BAD_REQUEST;
        }
    };
    if r.dropped > 0 {
        app.metrics
            .telemetry_relay_dropped
            .add(r.dropped as u64, &[]);
    }
    count(&app, "forwarded");
    if r.kept == 0 {
        return StatusCode::ACCEPTED;
    }
    let url = format!("{}/v1/traces", endpoint.trim_end_matches('/'));
    let headers = app.config.otel_headers.clone();
    let task_app = app.clone();
    tokio::spawn(async move {
        let mut req = http().post(&url).json(&r.body);
        for (k, v) in headers.pairs() {
            req = req.header(k, v);
        }
        let failed = match req.send().await {
            Ok(res) if res.status().is_success() => None,
            Ok(res) => Some(res.status().as_u16().to_string()),
            // reqwest's message may carry the URL, never the headers.
            Err(e) => Some(e.without_url().to_string()),
        };
        if let Some(why) = failed {
            count(&task_app, "export_failed");
            tracing::warn!(http.response.status_code = %why, "telemetry relay export failed");
        }
    });
    StatusCode::ACCEPTED
}
