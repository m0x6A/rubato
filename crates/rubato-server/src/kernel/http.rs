//! The one way a handler turns a store error into a status: logged with the slice that failed.
//! And the workspace a map-scoped request is for (ADR 0049): `Ws`, checked once per request.

use crate::kernel::state::App;
use axum::extract::{FromRequestParts, RawPathParams, Request, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rubato_protocol::DEFAULT_WORKSPACE;

/// The workspace a map-scoped request is for: the `{ws}` of `/api/w/{ws}/…`, else `default`.
/// Only a configured id gets here; `check_workspace` answers 404 for any other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ws(pub String);

impl Ws {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Ws {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<Ws>()
            .cloned()
            .unwrap_or_else(|| Ws(DEFAULT_WORKSPACE.to_string())))
    }
}

/// The body of the 404 answer for a workspace this server does not hold.
pub const UNKNOWN_WORKSPACE: &str = r#"{"reason":"unknown_workspace"}"#;

/// Layer on every `/api/w/{ws}/…` route: a configured `{ws}` becomes the request's `Ws`; any
/// other is 404 `unknown_workspace`, counted on `rubato.web.rejected`. Never a fallback to
/// `default`: a write must land where it was meant or nowhere.
pub async fn check_workspace(
    State(app): State<App>,
    params: RawPathParams,
    mut req: Request,
    next: Next,
) -> Response {
    let ws = params
        .iter()
        .find(|(k, _)| *k == "ws")
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();
    let (entries, _) = app.workspace_entries();
    if !entries.iter().any(|w| w.id == ws) {
        // The route template, not the path: the id in the URL is not a label.
        let route = req
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        app.reject(&route, "unknown_workspace", "-");
        tracing::warn!(rubato.route = %route, "unknown workspace");
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            UNKNOWN_WORKSPACE,
        )
            .into_response();
    }
    tracing::Span::current().record(rubato_protocol::otel::attr::WORKSPACE, ws.as_str());
    req.extensions_mut().insert(Ws(ws));
    next.run(req).await
}

/// The body of the 403 answer for a write or websocket upgrade from another origin (ADR 0066).
pub const CROSS_ORIGIN: &str = r#"{"reason":"cross_origin"}"#;

/// `scheme://host[:port]` of a URL, lower case, without a path: what a browser sends as `Origin`.
fn origin_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?;
    Some(match u.port() {
        Some(p) => format!("{}://{host}:{p}", u.scheme()),
        None => format!("{}://{host}", u.scheme()),
    })
    .map(|s| s.to_ascii_lowercase())
}

/// Whether a request comes from a browser page on another origin and must be refused (design
/// 193 D1, the rule Go ships as `CrossOriginProtection`). Only unsafe methods and websocket
/// upgrades are checked. Fetch metadata decides first; without it, a request with no `Origin`
/// is not from a browser (the companion, `curl`) and passes; an `Origin` must be the public
/// URL's or the request's own `Host`.
pub fn cross_origin(
    method: &axum::http::Method,
    headers: &axum::http::HeaderMap,
    public_url: &str,
) -> bool {
    use axum::http::{Method, header};
    let upgrade = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let safe = matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    if safe && !upgrade {
        return false;
    }
    if let Some(site) = headers.get("sec-fetch-site") {
        let site = site.to_str().unwrap_or("").to_ascii_lowercase();
        return !(site == "same-origin" || site == "none");
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return false;
    };
    let origin = origin.to_str().unwrap_or("").to_ascii_lowercase();
    if origin_of(public_url).is_some_and(|o| o == origin) {
        return false;
    }
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase);
    match host {
        Some(h) => origin != format!("http://{h}") && origin != format!("https://{h}"),
        None => true,
    }
}

/// Layer on every API and websocket route: a cross-origin write or upgrade answers 403
/// `cross_origin`, counted on `rubato.web.rejected`, before any handler or session lookup.
pub async fn same_origin(State(app): State<App>, req: Request, next: Next) -> Response {
    if !cross_origin(req.method(), req.headers(), &app.config.public_url) {
        return next.run(req).await;
    }
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    let h = |name: &str| {
        req.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-")
            .to_owned()
    };
    tracing::warn!(
        rubato.route = %route,
        method = %req.method(),
        origin = %h("origin"),
        sec_fetch_site = %h("sec-fetch-site"),
        "cross-origin request refused"
    );
    app.reject(&route, "cross_origin", "-");
    (
        StatusCode::FORBIDDEN,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        CROSS_ORIGIN,
    )
        .into_response()
}

/// The Content-Security-Policy for this server (ADR 0066): only its own origin, its websocket,
/// and the font subsets the build inlines as `data:`; never framed. With voice on (ADR 0073)
/// `connect-src` also allows the LiveKit server, as `wss:` and `https:`; with it off the policy is
/// exactly as before.
pub fn content_security_policy(public_url: &str, livekit_url: Option<&str>) -> String {
    let base = policy_without_voice(public_url);
    match livekit_url.and_then(livekit_origins) {
        Some(extra) => base.replacen("; font-src", &format!(" {extra}; font-src"), 1),
        None => base,
    }
}

/// `wss://host[:port] https://host[:port]` for a LiveKit URL (`ws:` and `http:` for a plain one).
fn livekit_origins(url: &str) -> Option<String> {
    let o = origin_of(url)?;
    let rest = ["wss://", "https://"]
        .iter()
        .find_map(|p| o.strip_prefix(p))
        .map(|r| (format!("wss://{r}"), format!("https://{r}")))
        .or_else(|| {
            ["ws://", "http://"]
                .iter()
                .find_map(|p| o.strip_prefix(p))
                .map(|r| (format!("ws://{r}"), format!("http://{r}")))
        })?;
    Some(format!("{} {}", rest.0, rest.1))
}

fn policy_without_voice(public_url: &str) -> String {
    let ws = origin_of(public_url)
        .map(|o| {
            if let Some(rest) = o.strip_prefix("https://") {
                format!(" wss://{rest}")
            } else if let Some(rest) = o.strip_prefix("http://") {
                format!(" ws://{rest}")
            } else {
                String::new()
            }
        })
        .unwrap_or_default();
    format!(
        "default-src 'self'; connect-src 'self'{ws}; font-src 'self' data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'"
    )
}

/// Add `Content-Security-Policy` and `X-Content-Type-Options: nosniff` to a response.
pub fn add_security_headers(csp: &axum::http::HeaderValue, res: &mut Response) {
    use axum::http::{HeaderValue, header};
    let h = res.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, csp.clone());
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
}

/// Log `error!(slice, error, "request failed")` and answer 500.
/// The span of one HTTP request (ADR 0082). A request carrying a valid `traceparent`, which
/// the web sends on its own API calls, gets `rubato.http.request` of kind server under the
/// browser's client span, so the service graph sees both ends of the call. Any other request
/// keeps the internal span `TraceLayer` would make, so health checks and asset loads draw no
/// virtual caller. Each request is counted by what its header gave. The header is used for
/// nothing but the parent: not an identity, not an authorisation.
pub fn request_span<B>(app: &App, req: &axum::http::Request<B>) -> tracing::Span {
    use opentelemetry::KeyValue;
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
    };
    use rubato_protocol::otel::{TraceParent, attr, span};
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;
    let header = req
        .headers()
        .get("traceparent")
        .map(|v| v.to_str().ok().and_then(TraceParent::parse));
    let outcome = match header {
        None => "absent",
        Some(None) => "invalid",
        Some(Some(_)) => "linked",
    };
    app.metrics
        .http_traceparent
        .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
    let Some(Some(tp)) = header else {
        return tracing::debug_span!(
            "request",
            method = %req.method(),
            uri = %req.uri(),
            version = ?req.version()
        );
    };
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or_else(|| req.uri().path().to_owned(), |m| m.as_str().to_owned());
    let s = tracing::info_span!(
        span::HTTP_REQUEST,
        otel.kind = "server",
        http.request.method = %req.method(),
        http.route = %route,
        http.response.status_code = tracing::field::Empty,
        otel.status_code = tracing::field::Empty,
    );
    let remote = SpanContext::new(
        TraceId::from(tp.trace_id),
        SpanId::from(tp.span_id),
        TraceFlags::new(tp.flags),
        true,
        TraceState::default(),
    );
    // Without an OpenTelemetry layer (no OTLP endpoint) there is nothing to parent.
    let _ = s.set_parent(opentelemetry::Context::new().with_remote_span_context(remote));
    s
}

/// The response's status on the request's span; a server error marks it failed.
pub fn record_status<B>(res: &axum::http::Response<B>, span: &tracing::Span) {
    let status = res.status();
    span.record("http.response.status_code", status.as_u16());
    if status.is_server_error() {
        span.record("otel.status_code", "ERROR");
    }
}

pub fn internal(slice: &'static str, e: impl std::fmt::Display) -> StatusCode {
    tracing::error!(slice, error = %e, "request failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;

    /// Design 193 D1, case by case. Each row is (method, headers, public URL, refused).
    #[test]
    fn cross_origin_table() {
        use axum::http::{HeaderMap, Method};
        const PUB: &str = "https://rubato.example";
        const LAN: &str = "http://192.168.1.20:8081";
        type Case<'a> = (Method, &'a [(&'a str, &'a str)], &'a str, bool);
        let cases: &[Case] = &[
            (
                Method::GET,
                &[("origin", "https://evil.example")],
                PUB,
                false,
            ),
            (
                Method::HEAD,
                &[("sec-fetch-site", "cross-site")],
                PUB,
                false,
            ),
            (
                Method::OPTIONS,
                &[("origin", "https://evil.example")],
                PUB,
                false,
            ),
            (Method::POST, &[], PUB, false),
            (
                Method::POST,
                &[("sec-fetch-site", "same-origin")],
                PUB,
                false,
            ),
            (Method::POST, &[("sec-fetch-site", "none")], PUB, false),
            (Method::POST, &[("sec-fetch-site", "same-site")], PUB, true),
            (
                Method::DELETE,
                &[("sec-fetch-site", "cross-site")],
                PUB,
                true,
            ),
            // Fetch metadata decides before Origin: the dev proxy keeps localhost:5173.
            (
                Method::PUT,
                &[
                    ("sec-fetch-site", "same-origin"),
                    ("origin", "http://localhost:5173"),
                    ("host", "127.0.0.1:8080"),
                ],
                PUB,
                false,
            ),
            (
                Method::POST,
                &[("origin", "https://rubato.example")],
                PUB,
                false,
            ),
            (
                Method::POST,
                &[("origin", "HTTPS://Rubato.Example")],
                PUB,
                false,
            ),
            (
                Method::POST,
                &[
                    ("origin", "https://evil.example"),
                    ("host", "rubato.example"),
                ],
                PUB,
                true,
            ),
            (
                Method::POST,
                &[("origin", "null"), ("host", "rubato.example")],
                PUB,
                true,
            ),
            // The team's LAN deployment: plain http on a LAN address and port.
            (
                Method::POST,
                &[
                    ("origin", "http://192.168.1.20:8081"),
                    ("host", "192.168.1.20:8081"),
                ],
                LAN,
                false,
            ),
            (
                Method::POST,
                &[("origin", "http://192.168.1.20:8081")],
                LAN,
                false,
            ),
            (
                Method::POST,
                &[
                    ("origin", "http://192.168.1.20:8082"),
                    ("host", "192.168.1.20:8081"),
                ],
                LAN,
                true,
            ),
            // Origin matches Host though the public URL is stale.
            (
                Method::POST,
                &[
                    ("origin", "http://10.0.0.5:8081"),
                    ("host", "10.0.0.5:8081"),
                ],
                LAN,
                false,
            ),
            // A websocket upgrade is a GET but is checked.
            (
                Method::GET,
                &[
                    ("upgrade", "websocket"),
                    ("origin", "https://evil.example"),
                    ("host", "rubato.example"),
                ],
                PUB,
                true,
            ),
            (
                Method::GET,
                &[("upgrade", "websocket"), ("host", "rubato.example")],
                PUB,
                false,
            ),
            (
                Method::GET,
                &[("upgrade", "WebSocket"), ("sec-fetch-site", "same-origin")],
                PUB,
                false,
            ),
        ];
        for (method, headers, public_url, refused) in cases {
            let mut map = HeaderMap::new();
            for (k, v) in *headers {
                map.insert(*k, v.parse().unwrap());
            }
            assert_eq!(
                cross_origin(method, &map, public_url),
                *refused,
                "{method} {headers:?} against {public_url}"
            );
        }
        assert!(cases.iter().any(|c| c.3) && cases.iter().any(|c| !c.3));
        println!("checked {} cross-origin cases", cases.len());
    }

    #[test]
    fn the_policy_names_the_public_websocket_origin() {
        let csp = content_security_policy("http://192.168.1.20:8081/", None);
        assert!(
            csp.contains("connect-src 'self' ws://192.168.1.20:8081;"),
            "{csp}"
        );
        assert!(csp.contains("frame-ancestors 'none'"));
        let csp = content_security_policy("https://rubato.example", None);
        assert!(
            csp.contains("connect-src 'self' wss://rubato.example;"),
            "{csp}"
        );
    }

    /// ADR 0073: voice on adds the LiveKit origin to `connect-src` only; voice off is byte-equal
    /// to the policy before voice.
    #[test]
    fn voice_adds_only_the_livekit_origin() {
        let off = content_security_policy("https://rubato.example", None);
        assert_eq!(
            off,
            "default-src 'self'; connect-src 'self' wss://rubato.example; font-src 'self' data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'"
        );
        let on = content_security_policy(
            "https://rubato.example",
            Some("wss://livekit.example:7880/"),
        );
        assert!(
            on.contains(
                "connect-src 'self' wss://rubato.example wss://livekit.example:7880 https://livekit.example:7880; font-src"
            ),
            "{on}"
        );
        assert_eq!(
            on.replace(
                " wss://livekit.example:7880 https://livekit.example:7880",
                ""
            ),
            off
        );
        // A URL that does not read adds nothing.
        assert_eq!(
            content_security_policy("https://rubato.example", Some("nope")),
            off
        );
    }

    #[test]
    fn internal_logs_slice_and_error() {
        let (events, _guard) = capture::start();
        assert_eq!(
            internal("pairing", "disk full"),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        let e = &events[0];
        assert_eq!(e.level, tracing::Level::ERROR);
        assert_eq!(e.message, "request failed");
        assert_eq!(e.fields.get("slice").map(String::as_str), Some("pairing"));
        assert_eq!(e.fields.get("error").map(String::as_str), Some("disk full"));
    }
}
