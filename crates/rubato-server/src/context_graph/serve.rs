//! Getting the graph to companions (ADR 0086): the list of sources on the websocket, and one
//! source's snapshot on a route the companion asks with its pairing token.
//!
//! **Who may read.** A pairing never expires, and the graph holds the file and symbol names of
//! every repository on the maps. So a pairing reads it only while its owner has a team record
//! that still counts: a web session whose access to the App's installation was checked at most
//! seven days ago, the rule every web route is held to (ADR 0022). A person who leaves the
//! team stops receiving the graph when they stop seeing the web. A server without the App
//! starts with the graph only when it is local (`kernel::config`), where there is no team to
//! check against.
//!
//! Every source goes to every such companion (plan 322, Q1, decided by Mattias 2026-11-27).

use super::metrics;
use crate::kernel::state::App;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rubato_protocol::graph::{GraphSourceInfo, GraphSources, reason};
use rubato_protocol::otel::span;
use rubato_protocol::{ServerMessage, capability, limits};
use serde_json::json;
use tracing::Instrument;
use uuid::Uuid;

/// Whether `user`'s companions may read the graph now.
pub(super) async fn may_read(app: &App, user: &str) -> bool {
    if app.config.github_app.is_none() {
        return true;
    }
    crate::auth::has_fresh_team_record(&app.db, user, app.clock.now())
        .await
        .unwrap_or(false)
}

/// What the server announces: every source that has been built, at most the cap.
pub(super) async fn sources(app: &App) -> Option<GraphSources> {
    let store = super::store(app)?;
    let rows = store.sources().await.ok()?;
    let sources: Vec<GraphSourceInfo> = rows
        .into_iter()
        .filter_map(|r| {
            Some(GraphSourceInfo {
                content_hash: r.content_hash?,
                generated_at: r.generated_at?,
                source: r.source,
                kind: r.kind,
                head_sha: r.head_sha,
                map_version: r.map_version,
                nodes: r.nodes,
                edges: r.edges,
                truncated: r.truncated,
            })
        })
        .take(limits::GRAPH_SOURCES_MAX)
        .collect();
    Some(GraphSources { sources })
}

/// The owner of a companion that is paired and not revoked.
async fn owner_of(app: &App, companion_id: Uuid) -> Option<String> {
    crate::pairing::list_pairings(&app.db, None)
        .await
        .ok()?
        .into_iter()
        .find(|p| p.companion_id == companion_id && p.revoked_at.is_none())
        .map(|p| p.user)
}

/// Tell one companion what it should hold, if it keeps a copy and its owner may read. Returns
/// whether the list was sent.
pub async fn announce_to(app: &App, companion_id: Uuid) -> bool {
    if app.config.context_graph.is_none()
        || !app
            .capabilities(companion_id)
            .iter()
            .any(|c| c == capability::CONTEXT_GRAPH)
    {
        return false;
    }
    let Some(user) = owner_of(app, companion_id).await else {
        return false;
    };
    if !may_read(app, &user).await {
        return false;
    }
    let Some(list) = sources(app).await else {
        return false;
    };
    app.send_to_companion(companion_id, ServerMessage::GraphSources(list))
}

/// Tell every connected companion: after a source was built or removed.
pub async fn announce_all(app: &App) -> usize {
    let mut sent = 0;
    for companion in app.connected_companions() {
        if announce_to(app, companion).await {
            sent += 1;
        }
    }
    sent
}

fn refuse(status: StatusCode, why: &'static str) -> Response {
    (status, Json(json!({ "reason": why }))).into_response()
}

/// `GET /api/graph/snapshot?source=<source key>`, `Authorization: Bearer <pairing token>`: one
/// source as gzipped NDJSON. The token opens this route and no other.
pub async fn snapshot(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    // `?source=`, as `SnapshotQuery` names it. Without one there is no such source.
    let source = query.get("source").cloned().unwrap_or_default();
    let span = tracing::info_span!(
        span::GRAPH_SNAPSHOT_SERVE,
        rubato.graph.source = %source,
        rubato.companion_id = tracing::field::Empty,
        rubato.outcome = tracing::field::Empty,
    );
    async move {
        let done = |outcome: &'static str, response: Response| {
            tracing::Span::current().record("rubato.outcome", outcome);
            metrics::snapshot(outcome);
            response
        };
        let Some(store) = super::store(&app) else {
            return done("off", refuse(StatusCode::NOT_FOUND, reason::CONTEXT_GRAPH_OFF));
        };
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let pairing = match token {
            Some(token) => crate::pairing::authenticate(&app.db, token).await.ok().flatten(),
            None => None,
        };
        let Some(pairing) = pairing.filter(|p| p.revoked_at.is_none()) else {
            return done("unauthorized", StatusCode::UNAUTHORIZED.into_response());
        };
        tracing::Span::current().record("rubato.companion_id", pairing.companion_id.to_string());
        if !may_read(&app, &pairing.user).await {
            return done("stale", refuse(StatusCode::FORBIDDEN, reason::TEAM_RECORD_STALE));
        }
        match store.export(&source).await {
            Ok(Some(body)) if body.len() as u64 <= limits::GRAPH_SNAPSHOT_MAX_BYTES => done(
                "ok",
                ([(header::CONTENT_TYPE, "application/gzip")], body).into_response(),
            ),
            Ok(Some(body)) => {
                tracing::error!(rubato.graph.source = %source, bytes = body.len(), "context graph snapshot over the cap");
                done("error", StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
            Ok(None) => done("unknown_source", refuse(StatusCode::NOT_FOUND, reason::UNKNOWN_SOURCE)),
            Err(e) => {
                tracing::error!(rubato.graph.source = %source, error = %e, "context graph snapshot not served");
                done("error", StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
        }
    }
    .instrument(span)
    .await
}
