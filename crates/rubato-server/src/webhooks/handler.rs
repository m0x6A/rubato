//! `POST /webhooks/github` (ADR 0074): verify, reduce, store once, wake the poll loop, answer.
//! The handler makes no GitHub call and writes only its own table; the poll loop reads what a
//! delivery names (design 208 D6).

use super::event::{self, Context};
use super::store;
use super::verify;
use crate::kernel::state::App;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::rejection::BytesRejection;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use sha2::{Digest, Sha256};

/// The path GitHub posts to.
pub const PATH: &str = "/webhooks/github";
/// The largest body read (A2); GitHub caps payloads at 25 MB.
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

/// Unsigned header values as they may be logged: at most 64 of `[A-Za-z0-9._-]`.
fn loggable(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(64)
        .collect()
}

fn answer(status: StatusCode, outcome: &str) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"outcome":"{outcome}"}}"#),
    )
        .into_response()
}

fn count(app: &App, event: &str, outcome: &'static str) {
    app.metrics.webhook_deliveries.add(
        1,
        &[
            KeyValue::new(attr::GITHUB_EVENT, event::event_label(event)),
            KeyValue::new(attr::OUTCOME, outcome),
        ],
    );
}

fn refuse(
    app: &App,
    headers: &HeaderMap,
    status: StatusCode,
    outcome: &'static str,
    reason: &str,
) -> Response {
    let ev = loggable(headers, "x-github-event");
    tracing::warn!(
        status = status.as_u16(),
        reason,
        delivery = %loggable(headers, "x-github-delivery"),
        event = %ev,
        "webhook refused"
    );
    count(app, &ev, outcome);
    let body = if outcome == "rejected" {
        r#"{"reason":"bad_signature"}"#.to_owned()
    } else {
        format!(r#"{{"reason":"{outcome}"}}"#)
    };
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

pub async fn receive(
    State(app): State<App>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Some(secret) = app.config.github_webhook_secret.clone() else {
        // Not routed without a secret (app.rs); answered as an unknown path all the same.
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            r#"{"error":"unknown api path"}"#,
        )
            .into_response();
    };
    let body = match body {
        Ok(b) => b,
        Err(e) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return refuse(
                &app,
                &headers,
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "body over 5 MiB",
            );
        }
        Err(e) => {
            return refuse(
                &app,
                &headers,
                StatusCode::BAD_REQUEST,
                "malformed",
                &e.body_text(),
            );
        }
    };
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok());
    if let Err(why) = verify::verify(secret.as_bytes(), signature, &body) {
        return refuse(
            &app,
            &headers,
            StatusCode::UNAUTHORIZED,
            "rejected",
            why.reason(),
        );
    }
    // Verified: the body is GitHub's. The headers are not signed; the body hash makes a replay
    // under fresh headers a duplicate (D4).
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty() && v.len() <= 64)
            .map(str::to_owned)
    };
    let (Some(id), Some(ev)) = (header("x-github-delivery"), header("x-github-event")) else {
        return refuse(
            &app,
            &headers,
            StatusCode::BAD_REQUEST,
            "malformed",
            "missing X-GitHub-Delivery or X-GitHub-Event",
        );
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return refuse(
            &app,
            &headers,
            StatusCode::BAD_REQUEST,
            "malformed",
            "body is not JSON",
        );
    };
    let read_repos = app.read_repos().await;
    let maps: Vec<(String, String)> = app
        .config
        .workspace_entries()
        .0
        .into_iter()
        .map(|w| (w.map_repo, w.map_ref))
        .collect();
    let is_read = |r: &str| read_repos.contains(r);
    let is_map = |r: &str, b: &str| {
        maps.iter()
            .any(|(mr, mb)| mr.eq_ignore_ascii_case(r) && mb == b)
    };
    let delivery = event::read(
        &ev,
        &json,
        &Context {
            read_repos: &is_read,
            map_branch: &is_map,
        },
    );
    let span = tracing::info_span!(
        span::INBOUND_EVENT,
        rubato.github.event = event::event_label(&ev),
        rubato.repo = delivery.repo.as_deref().unwrap_or("-"),
        rubato.issue = delivery.issue_number.unwrap_or(0),
        rubato.outcome = tracing::field::Empty,
    );
    let _enter = span.enter();
    let hash: String = Sha256::digest(&body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let now = app.clock.now().timestamp();
    let (status, outcome) = match store::insert(&app.db, &id, &hash, &delivery, now).await {
        Err(e) => {
            tracing::error!(delivery = %id, event = %ev, error = %format!("{e:#}"), "webhook store failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "error")
        }
        Ok(false) => (StatusCode::OK, "duplicate"),
        Ok(true) if delivery.ignored.is_some() => (StatusCode::OK, "ignored"),
        Ok(true) => {
            app.webhook_wake.notify_one();
            (StatusCode::ACCEPTED, "accepted")
        }
    };
    span.record("rubato.outcome", outcome);
    tracing::info!(
        delivery = %id,
        event = %ev,
        action = delivery.action.as_deref().unwrap_or("-"),
        rubato.repo = delivery.repo.as_deref().unwrap_or("-"),
        rubato.issue = delivery.issue_number.unwrap_or(0),
        rubato.outcome = outcome,
        reason = delivery.ignored.unwrap_or("-"),
        "webhook delivery"
    );
    count(&app, &ev, outcome);
    answer(status, outcome)
}
