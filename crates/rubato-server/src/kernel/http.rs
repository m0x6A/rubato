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
    let (entries, _) = app.config.workspace_entries();
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

/// Log `error!(slice, error, "request failed")` and answer 500.
pub fn internal(slice: &'static str, e: impl std::fmt::Display) -> StatusCode {
    tracing::error!(slice, error = %e, "request failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;

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
