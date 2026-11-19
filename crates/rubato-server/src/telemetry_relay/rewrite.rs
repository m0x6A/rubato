//! The relay's rewrite (ADR 0082): what the browser sent becomes spans of `web` with only what
//! the server lets through. Pure: no I/O, no clock of its own.

use rubato_protocol::otel::semconv::SERVICE_NAMESPACE;
use rubato_protocol::telemetry_relay::{MAX_CLOCK_SKEW_SECS, MAX_SPANS, SERVICE_NAME};
use serde_json::{Value, json};

/// Why a post is refused whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalid {
    /// Not an OTLP/JSON `ExportTraceServiceRequest`.
    NotOtlp,
    NoSpans,
    TooMany,
}

/// The rewritten request, and how many of the browser's spans it kept and dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct Rewritten {
    pub body: Value,
    pub kept: usize,
    pub dropped: usize,
}

/// The methods the web uses.
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "DELETE", "PATCH"];
/// The longest route template, and the most segments in one.
const MAX_ROUTE_BYTES: usize = 128;
const MAX_ROUTE_SEGMENTS: usize = 10;

/// Whether `route` is a route template as the web makes them: under `/api/`, its literal
/// segments lowercase words, its ids replaced by named placeholders. No digits, so no issue or
/// pull number, and no free text; not the relay's own route.
pub fn is_route(route: &str) -> bool {
    if route.len() > MAX_ROUTE_BYTES
        || !route.starts_with("/api/")
        || route == rubato_protocol::telemetry_relay::routes::TELEMETRY_TRACES
    {
        return false;
    }
    let segments: Vec<&str> = route[1..].split('/').collect();
    segments.len() <= MAX_ROUTE_SEGMENTS
        && segments.iter().all(|s| {
            let word = |w: &str| {
                !w.is_empty()
                    && w.bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'_' || b == b'-')
            };
            word(s)
                || (s.len() > 2
                    && s.starts_with('{')
                    && s.ends_with('}')
                    && word(&s[1..s.len() - 1]))
        })
}

fn hex_id(v: Option<&Value>, len: usize) -> Option<String> {
    let s = v?.as_str()?;
    (s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && s.bytes().any(|b| b != b'0'))
    .then(|| s.to_owned())
}

/// A nanosecond time, as OTLP/JSON writes it (a string) or a number.
fn nanos(v: Option<&Value>) -> Option<u128> {
    match v? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64().map(u128::from),
        _ => None,
    }
}

/// An attribute's value by key, as a string or an integer.
fn attribute<'a>(span: &'a Value, key: &str) -> Option<&'a Value> {
    span.get("attributes")?
        .as_array()?
        .iter()
        .find(|a| a.get("key").and_then(Value::as_str) == Some(key))?
        .get("value")
}

fn int_value(v: Option<&Value>) -> Option<i64> {
    match v?.get("intValue")? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// One span kept, or `None` when it is dropped.
fn keep(span: &Value, now_nanos: u128) -> Option<Value> {
    let trace_id = hex_id(span.get("traceId"), 32)?;
    let span_id = hex_id(span.get("spanId"), 16)?;
    let parent = match span.get("parentSpanId") {
        None => None,
        Some(Value::String(s)) if s.is_empty() => None,
        p => Some(hex_id(p, 16)?),
    };
    // Client spans only: the browser's side of a call.
    let client = matches!(span.get("kind"), Some(Value::Number(n)) if n.as_u64() == Some(3))
        || span.get("kind").and_then(Value::as_str) == Some("SPAN_KIND_CLIENT");
    if !client {
        return None;
    }
    let (start, end) = (
        nanos(span.get("startTimeUnixNano"))?,
        nanos(span.get("endTimeUnixNano"))?,
    );
    let skew = u128::try_from(MAX_CLOCK_SKEW_SECS).ok()? * 1_000_000_000;
    if end < start || start.abs_diff(now_nanos) > skew || end.abs_diff(now_nanos) > skew {
        return None;
    }
    let method = attribute(span, "http.request.method")?
        .get("stringValue")?
        .as_str()?;
    let method = METHODS.into_iter().find(|m| *m == method)?;
    let route = attribute(span, "url.path")?.get("stringValue")?.as_str()?;
    if !is_route(route) {
        return None;
    }
    let status = int_value(attribute(span, "http.response.status_code"))
        .filter(|s| (0..=599).contains(s))?;
    let code = span
        .get("status")
        .and_then(|s| s.get("code"))
        .and_then(Value::as_u64)
        .filter(|c| *c <= 2)
        .unwrap_or(0);
    let mut out = json!({
        "traceId": trace_id,
        "spanId": span_id,
        "name": format!("{method} {route}"),
        "kind": 3,
        "startTimeUnixNano": start.to_string(),
        "endTimeUnixNano": end.to_string(),
        "attributes": [
            {"key": "http.request.method", "value": {"stringValue": method}},
            {"key": "url.path", "value": {"stringValue": route}},
            {"key": "http.response.status_code", "value": {"intValue": status.to_string()}},
        ],
        "status": {"code": code},
    });
    if let Some(p) = parent {
        out["parentSpanId"] = Value::String(p);
    }
    Some(out)
}

/// The browser's request rewritten (ADR 0082): one resource, `service.name=web` in the server's
/// `namespace`; each span renamed from its method and route template, with only its ids, kind,
/// times, status code and three attributes. Anything else the browser wrote stays behind.
pub fn rewrite(request: &Value, namespace: &str, now_nanos: u128) -> Result<Rewritten, Invalid> {
    let resources = request
        .get("resourceSpans")
        .and_then(Value::as_array)
        .ok_or(Invalid::NotOtlp)?;
    let mut spans: Vec<&Value> = Vec::new();
    for r in resources {
        for scope in r
            .get("scopeSpans")
            .and_then(Value::as_array)
            .ok_or(Invalid::NotOtlp)?
        {
            spans.extend(
                scope
                    .get("spans")
                    .and_then(Value::as_array)
                    .ok_or(Invalid::NotOtlp)?,
            );
            if spans.len() > MAX_SPANS {
                return Err(Invalid::TooMany);
            }
        }
    }
    if spans.is_empty() {
        return Err(Invalid::NoSpans);
    }
    let kept: Vec<Value> = spans.iter().filter_map(|s| keep(s, now_nanos)).collect();
    let dropped = spans.len() - kept.len();
    let body = json!({
        "resourceSpans": [{
            "resource": {"attributes": [
                {"key": "service.name", "value": {"stringValue": SERVICE_NAME}},
                {"key": SERVICE_NAMESPACE, "value": {"stringValue": namespace}},
            ]},
            "scopeSpans": [{"scope": {"name": "rubato-web-relay"}, "spans": kept}],
        }],
    });
    Ok(Rewritten {
        kept: kept_len(&body),
        body,
        dropped,
    })
}

fn kept_len(body: &Value) -> usize {
    body["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .map_or(0, Vec::len)
}
