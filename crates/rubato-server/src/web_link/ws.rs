//! Fan-out to browsers: `WebEvent::Changed` whenever a signal lands; clients refetch the overview.

use crate::auth::CurrentUser;
use crate::kernel::state::App;
use crate::kernel::tenancy::TenantId;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use opentelemetry::KeyValue;
use rubato_protocol::WebEvent;
use rubato_protocol::otel::{attr, span};
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::Instrument;

static WEB_CLIENTS: AtomicU64 = AtomicU64::new(0);

/// The reason of the catch-up event a lagged client is sent.
pub const LAGGED: &str = "lagged";

/// `?t=<tenant>`: the tenant the socket is for.
#[derive(serde::Deserialize)]
pub struct Tenant {
    #[serde(default)]
    t: Option<String>,
}

/// `GET /ws/web?t=<tenant>` (ADR 0089, design §6.1): a socket for one tenant, opened only for a
/// member of it. A tenant that does not exist, one the person is not in, and no `t` at all
/// answer the same 404.
pub async fn upgrade(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    axum::extract::Query(q): axum::extract::Query<Tenant>,
    ws: WebSocketUpgrade,
) -> Response {
    let tenant = q.t.as_deref().and_then(TenantId::parse);
    let scope = match &tenant {
        Some(t) => match crate::tenants::scope_for(&app, t, &user).await {
            Ok(s) => s,
            Err(e) => {
                return axum::response::IntoResponse::into_response(crate::kernel::http::internal(
                    "web_link", e,
                ));
            }
        },
        None => None,
    };
    let Some(scope) = scope else {
        app.reject(
            rubato_protocol::web_link::routes::WEB_WS,
            rubato_protocol::tenant::reason::UNKNOWN_TENANT,
            &user,
        );
        return crate::kernel::tenancy::unknown_tenant();
    };
    ws.on_upgrade(move |socket| handle(app, socket, scope))
}

async fn handle(app: App, socket: WebSocket, scope: crate::kernel::tenancy::TenantScope) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = app.web_tx.subscribe();
    let mine = scope.tenant().clone();
    // Membership is read again this often: a person removed from the tenant stops hearing of
    // it within that long (design §6.1).
    let mut recheck = tokio::time::interval(app.config.web_membership_check);
    recheck.tick().await;
    let n = WEB_CLIENTS.fetch_add(1, Ordering::SeqCst) + 1;
    app.metrics
        .ws_clients
        .record(n, &[KeyValue::new("kind", "web")]);
    loop {
        tokio::select! {
            // Membership is read again on a timer: a person removed from the tenant is told so
            // with close code 4403, and hears nothing more of it.
            _ = recheck.tick() => {
                let still = crate::tenants::scope_for(&app, &mine, scope.login())
                    .await
                    .ok()
                    .flatten()
                    .is_some();
                if !still {
                    tracing::info!(rubato.tenant = %mine, rubato.user = %scope.login(), "web socket closed: no longer a member");
                    let _ = sink
                        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: rubato_protocol::web_link::routes::CLOSE_NOT_A_MEMBER,
                            reason: "not a member".into(),
                        })))
                        .await;
                    break;
                }
            }
            ev = rx.recv() => match ev {
                // Another tenant's event is dropped here, before it is sent (design §6.1): the
                // browser never sees it, whatever its own filter does.
                Ok(ev) if ev.tenant().is_some_and(|t| t != mine.as_str()) => {}
                Ok(ev) => {
                    let span = tracing::debug_span!(span::WS_FANOUT, rubato.subscribers = app.web_tx.receiver_count());
                    let text = serde_json::to_string(&ev).unwrap_or_default();
                    if sink.send(Message::Text(text.into())).instrument(span).await.is_err() {
                        break;
                    }
                }
                // This client fell behind and events were skipped for it. Counted, and it is
                // sent one catch-up event so it refetches instead of staying behind unseen.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    app.metrics.events_dropped.add(
                        skipped,
                        &[KeyValue::new(attr::DROP_REASON, "web_lagged")],
                    );
                    tracing::warn!(skipped, "web client lagged");
                    let catch_up = WebEvent::Changed {
                        at: chrono::Utc::now(),
                        reason: LAGGED.into(),
                        tenant: Some(mine.to_string()),
                        workspace: None,
                    };
                    let text = serde_json::to_string(&catch_up).unwrap_or_default();
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            inbound = stream.next() => match inbound {
                Some(Ok(Message::Ping(p))) => { let _ = sink.send(Message::Pong(p)).await; }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
    let n = WEB_CLIENTS.fetch_sub(1, Ordering::SeqCst) - 1;
    app.metrics
        .ws_clients
        .record(n, &[KeyValue::new("kind", "web")]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::db::connect;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use crate::kernel::telemetry::metric_reader;
    use axum::Router;
    use axum::routing::get;
    use rubato_protocol::otel::metric;
    use std::sync::Arc;
    use std::time::Duration;

    /// A browser that falls behind is not left behind unseen: the skipped events are counted
    /// and it gets exactly one catch-up event, so it refetches.
    #[tokio::test(flavor = "current_thread")]
    async fn a_lagged_client_gets_one_changed_event() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(
            pool,
            Config::for_tests(url, "m0x6A"),
            Arc::new(FakeGitHub::new()),
        );
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/ws/web", get(upgrade))
            .with_state(app.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/web?t=default"))
            .await
            .unwrap();
        // Wait for the server side to subscribe.
        for _ in 0..200 {
            if app.web_tx.receiver_count() > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(app.web_tx.receiver_count(), 1);

        // More events than the channel holds, sent without yielding: the handler cannot keep up.
        let sent = 300usize;
        for i in 0..sent {
            app.notify(&format!("burst-{i}"));
        }
        let mut reasons = Vec::new();
        while let Ok(Some(Ok(msg))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
        {
            if let tokio_tungstenite::tungstenite::Message::Text(t) = msg
                && let WebEvent::Changed { reason, .. } = serde_json::from_str(&t).unwrap()
            {
                reasons.push(reason);
            }
        }
        let lagged = reasons.iter().filter(|r| *r == LAGGED).count();
        assert_eq!(lagged, 1, "one catch-up event: {} received", reasons.len());
        assert_eq!(
            reasons[0], LAGGED,
            "it comes before the events that were kept"
        );
        let dropped: Vec<_> = reader
            .points(metric::EVENTS_DROPPED)
            .into_iter()
            .filter(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some("web_lagged"))
            .collect();
        assert_eq!(dropped.len(), 1, "{dropped:?}");
        assert_eq!(
            dropped[0].value as usize + reasons.len() - 1,
            sent,
            "skipped plus delivered is everything sent"
        );
        println!(
            "sent {sent}, delivered {}, skipped {}",
            reasons.len() - 1,
            dropped[0].value
        );
    }
}
