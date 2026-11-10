//! Fan-out to browsers: `WebEvent::Changed` whenever a signal lands; clients refetch the overview.

use crate::auth::CurrentUser;
use crate::kernel::state::App;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use opentelemetry::KeyValue;
use rubato_protocol::otel::span;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::Instrument;

static WEB_CLIENTS: AtomicU64 = AtomicU64::new(0);

pub async fn upgrade(State(app): State<App>, _user: CurrentUser, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| handle(app, socket))
}

async fn handle(app: App, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = app.web_tx.subscribe();
    let n = WEB_CLIENTS.fetch_add(1, Ordering::SeqCst) + 1;
    app.metrics
        .ws_clients
        .record(n, &[KeyValue::new("kind", "web")]);
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    let span = tracing::debug_span!(span::WS_FANOUT, rubato.subscribers = app.web_tx.receiver_count());
                    let text = serde_json::to_string(&ev).unwrap_or_default();
                    if sink.send(Message::Text(text.into())).instrument(span).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
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
