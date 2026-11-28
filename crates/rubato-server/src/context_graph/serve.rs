//! Getting the graph to companions (ADR 0086): the list of sources on the websocket, and one
//! source's snapshot on a route the companion asks with its pairing token.
//!
//! **Who may read.** A pairing never expires, and the graph holds the file and symbol names of
//! every repository on the maps. So a pairing reads only the sources of the tenants its owner
//! is a member of now (ADR 0087, design §6.4): a repository's source belongs to the tenant
//! the repository is live in, a workspace's and a team's to the tenant their key names. A person
//! who leaves a tenant stops receiving its sources when they stop seeing it on the web, and a
//! person in no tenant receives none.

use super::metrics;
use crate::kernel::state::App;
use crate::kernel::tenancy::TenantId;
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

/// The tenants whose sources `user`'s companions may read now: the ones `user` is a member
/// of. Empty when the memberships cannot be read: nothing is sent on a guess.
pub(super) async fn readable(app: &App, user: &str) -> Vec<TenantId> {
    match app.admission().member_of(app, user).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(rubato.user = %user, error = %format!("{e:#}"), "context graph: memberships not read");
            Vec::new()
        }
    }
}

/// What a companion whose owner is in `tenants` is told to hold: every built source of those
/// tenants. Filtered first, then cut at the cap (design §6.4): another tenant's sources never
/// take a place in the list. A cut list is logged and counted.
pub(super) async fn sources(app: &App, tenants: &[TenantId]) -> Option<GraphSources> {
    let store = super::store(app)?;
    let rows = store.sources().await.ok()?;
    let mut sources: Vec<GraphSourceInfo> = rows
        .into_iter()
        .filter(|r| super::source_tenant(app, &r.source).is_some_and(|t| tenants.contains(&t)))
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
        .collect();
    if sources.len() > limits::GRAPH_SOURCES_MAX {
        let dropped = sources.len() - limits::GRAPH_SOURCES_MAX;
        tracing::warn!(
            kept = limits::GRAPH_SOURCES_MAX,
            dropped,
            "graph sources truncated"
        );
        metrics::sources_truncated();
        sources.truncate(limits::GRAPH_SOURCES_MAX);
    }
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
    let tenants = readable(app, &user).await;
    if tenants.is_empty() {
        return false;
    }
    let Some(list) = sources(app, &tenants).await else {
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
        // In no tenant: the answer a companion already knows how to wait on.
        let tenants = readable(&app, &pairing.user).await;
        if tenants.is_empty() {
            return done("stale", refuse(StatusCode::FORBIDDEN, reason::TEAM_RECORD_STALE));
        }
        // Another tenant's source, or one of no tenant, is answered as one that does not exist.
        if !super::source_tenant(&app, &source).is_some_and(|t| tenants.contains(&t)) {
            tracing::warn!(rubato.graph.source = %source, rubato.user = %pairing.user, "context graph snapshot refused: not a source of the owner's tenants");
            return done("refused", refuse(StatusCode::NOT_FOUND, reason::UNKNOWN_SOURCE));
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
