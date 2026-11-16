//! The Streamable HTTP subset of MCP the slice speaks (ADR 0071, #197 design §9, plan 220 §4):
//! one JSON-RPC message per `POST /mcp`, answered with `application/json`, no stream, no session
//! id. A notification gets 202 with no body. `GET /mcp` is 405. A present `Origin` that is not a
//! loopback origin is 403. OAuth discovery (`/.well-known/*`, asked without the token after a
//! 401) is 404 and counted.
//!
//! `401` means a bad token only, and the receiver's layer answers it (`hooks::server`). A request
//! with a good token and no known launch gets a JSON-RPC error (`-32001`) or, for `tools/call`, a
//! tool error: Claude Code reads a 401 as "needs authentication" and drops the server.

use super::{AgentMcp, tools};
use crate::hooks::LaunchHeader;
use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::Instrument;

/// The protocol versions the slice answers in, newest first. A client asking for another gets the
/// newest.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

/// JSON-RPC error for a request whose launch the companion does not know.
pub const NO_LAUNCH: i64 = -32001;

fn count(st: &AgentMcp, tool: &str, outcome: &str, reason: &str) {
    st.metrics.mcp_calls.add(
        1,
        &[
            KeyValue::new(attr::MCP_TOOL, tool.to_owned()),
            KeyValue::new(attr::OUTCOME, outcome.to_owned()),
            KeyValue::new(attr::DROP_REASON, reason.to_owned()),
        ],
    );
    if outcome == "refused" {
        tracing::warn!(
            rubato.mcp.tool = tool,
            rubato.reason = reason,
            "mcp call refused"
        );
    }
}

/// A loopback origin, or none: a browser page elsewhere must not reach `/mcp` (DNS rebinding).
fn origin_ok(headers: &HeaderMap) -> bool {
    let Some(o) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(o) = o.to_str() else { return false };
    let Ok(url) = url::Url::parse(o) else {
        return false;
    };
    matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
    )
}

fn json_response(v: Value) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], v.to_string()).into_response()
}

fn result(id: &Value, r: Value) -> Response {
    json_response(json!({"jsonrpc": "2.0", "id": id, "result": r}))
}

fn error(id: &Value, code: i64, message: &str) -> Response {
    json_response(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
}

/// The tool or method name for telemetry: never a string the client chose.
fn tool_label(method: &str, params: &Value) -> &'static str {
    match method {
        "initialize" => "initialize",
        "ping" => "ping",
        "tools/list" => "tools/list",
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            tools::TOOLS
                .iter()
                .map(|t| t.name)
                .chain(super::fleet::FLEET_TOOLS.iter().map(|(n, _, _)| *n))
                .find(|n| *n == name)
                .unwrap_or("unknown")
        }
        _ => "unknown",
    }
}

pub(super) async fn not_allowed() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

pub(super) async fn discovery(State(st): State<Arc<AgentMcp>>) -> StatusCode {
    count(&st, "discovery", "refused", "discovery");
    StatusCode::NOT_FOUND
}

pub(super) async fn post(
    State(st): State<Arc<AgentMcp>>,
    Extension(LaunchHeader(launch)): Extension<LaunchHeader>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !origin_ok(&headers) {
        count(&st, "unknown", "refused", "origin");
        return StatusCode::FORBIDDEN.into_response();
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            count(&st, "unknown", "error", "parse");
            return error(&Value::Null, -32700, "parse error");
        }
    };
    if !msg.is_object() {
        count(&st, "unknown", "error", "invalid_request");
        return error(&Value::Null, -32600, "one JSON-RPC message per request");
    }
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let Some(id) = msg.get("id").cloned() else {
        // A notification (`notifications/initialized` and the like): nothing to answer.
        return StatusCode::ACCEPTED.into_response();
    };
    let tool = tool_label(&method, &params);
    let span = tracing::info_span!(
        span::MCP_CALL,
        rubato.mcp.tool = tool,
        rubato.launch_id = tracing::field::Empty,
        rubato.issue = tracing::field::Empty,
        rubato.outcome = tracing::field::Empty,
        rubato.reason = tracing::field::Empty,
    );
    async move {
        let span = tracing::Span::current();
        let view = match launch {
            Some(l) => st.registry.lock().await.assignment_of(l),
            None => None,
        };
        let Some(view) = view else {
            span.record("rubato.outcome", "refused");
            span.record("rubato.reason", "no_launch");
            count(&st, tool, "refused", "no_launch");
            return if method == "tools/call" {
                result(&id, tools::refused("no_launch", "this companion did not launch this session").result)
            } else {
                error(&id, NO_LAUNCH, "no launch: this companion did not launch this session")
            };
        };
        span.record("rubato.launch_id", view.launch_id.to_string());
        span.record("rubato.issue", view.issue.to_string());
        if st.initialized.mark(view.launch_id) {
            tracing::info!(rubato.launch_id = %view.launch_id, "mcp initialized");
        }
        let (response, outcome, reason) = match method.as_str() {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|v| **v == asked)
                    .unwrap_or(&PROTOCOL_VERSIONS[0]);
                let r = json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "rubato-companion", "version": crate::kernel::VERSION},
                    "instructions": "Rubato's companion for this session: get_assignment says what it was launched for, report_status says in one line what you are doing, plan_status says where its plan stands. None of them approves anything.",
                });
                (result(&id, r), "ok", "")
            }
            "ping" => (result(&id, json!({})), "ok", ""),
            "tools/list" => {
                let lead = super::fleet::lead_ok(&st, &view, &headers).is_ok();
                (result(&id, tools::list(lead)), "ok", "")
            }
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                if super::fleet::is_fleet_tool(name) {
                    let c = super::fleet::call(&st, &view, name, &args, &headers).await;
                    span.record("rubato.outcome", c.outcome);
                    count(&st, tool, c.outcome, c.reason);
                    return result(&id, c.result);
                }
                match tools::call(&st, &view, name, &args) {
                    Some(c) => (result(&id, c.result), c.outcome, c.reason),
                    None => (error(&id, -32602, "unknown tool"), "error", "unknown_tool"),
                }
            }
            _ => (error(&id, -32601, "method not found"), "error", "unknown_method"),
        };
        span.record("rubato.outcome", outcome);
        if !reason.is_empty() {
            span.record("rubato.reason", reason);
        }
        count(&st, tool, outcome, reason);
        response
    }
    .instrument(span)
    .await
}
