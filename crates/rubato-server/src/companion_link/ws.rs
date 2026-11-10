//! The companion websocket (ADR 0012): bearer token, envelopes with sequence numbers, acks,
//! launch commands out. A span per inbound event with the session and issue as attributes.

use super::store;
use crate::kernel::state::{App, CompanionConn};
use crate::{launch, pairing};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{
    CompanionEnvelope, CompanionMessage, PROTOCOL_VERSION, ServerEnvelope, ServerMessage,
};
use tokio::sync::mpsc;
use tracing::Instrument;
use uuid::Uuid;

pub async fn upgrade(State(app): State<App>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    let pairing = match pairing::authenticate(&app.db, token).await {
        Ok(Some(p)) if p.revoked_at.is_none() => p,
        Ok(_) => {
            app.metrics
                .events_dropped
                .add(1, &[KeyValue::new(attr::DROP_REASON, "auth")]);
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "pairing lookup");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    ws.on_upgrade(move |socket| handle(app, pairing, socket))
}

async fn send_env(
    app: &App,
    companion_id: Uuid,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: ServerMessage,
) -> bool {
    let seq = pairing::next_server_seq(&app.db, companion_id)
        .await
        .unwrap_or(0);
    let env = ServerEnvelope {
        seq,
        sent_at: Utc::now(),
        message,
    };
    match serde_json::to_string(&env) {
        Ok(text) => sink.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => false,
    }
}

async fn handle(app: App, pairing: pairing::PairingRow, socket: WebSocket) {
    let companion_id = pairing.companion_id;
    let user = pairing.user.clone();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();
    app.companions
        .lock()
        .unwrap()
        .insert(companion_id, CompanionConn { tx });
    app.metrics.ws_clients.record(
        app.companions.lock().unwrap().len() as u64,
        &[KeyValue::new("kind", "companion")],
    );
    tracing::info!(%companion_id, %user, "companion connected");
    app.notify("companion connected");

    loop {
        tokio::select! {
            Some(msg) = rx.recv() => {
                if !send_env(&app, companion_id, &mut sink, msg).await {
                    break;
                }
            }
            inbound = stream.next() => {
                let Some(Ok(frame)) = inbound else { break };
                match frame {
                    Message::Text(text) => {
                        let env: CompanionEnvelope = match serde_json::from_str(&text) {
                            Ok(e) => e,
                            Err(e) => {
                                app.metrics.events_dropped.add(1, &[KeyValue::new(attr::DROP_REASON, "schema")]);
                                tracing::warn!(%companion_id, error = %e, "undecodable companion envelope dropped");
                                continue;
                            }
                        };
                        if let Some(reply) = on_envelope(&app, &pairing, env).await
                            && !send_env(&app, companion_id, &mut sink, reply).await {
                            break;
                        }
                    }
                    Message::Ping(p) => {
                        let _ = sink.send(Message::Pong(p)).await;
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
    app.companions.lock().unwrap().remove(&companion_id);
    app.metrics.ws_clients.record(
        app.companions.lock().unwrap().len() as u64,
        &[KeyValue::new("kind", "companion")],
    );
    tracing::info!(%companion_id, "companion disconnected");
    app.notify("companion disconnected");
}

/// Record the envelope, update rows, and answer. Returns a reply when one is due.
async fn on_envelope(
    app: &App,
    pairing: &pairing::PairingRow,
    env: CompanionEnvelope,
) -> Option<ServerMessage> {
    let companion_id = pairing.companion_id;
    let kind = match &env.message {
        CompanionMessage::Hello { .. } => "hello",
        CompanionMessage::Heartbeat(_) => "heartbeat",
        CompanionMessage::Hook(_) => "hook",
        CompanionMessage::Sessions { .. } => "sessions",
        CompanionMessage::Launch(_) => "launch",
        CompanionMessage::CapHit(_) => "cap_hit",
        CompanionMessage::Stale(_) => "stale",
        CompanionMessage::Ack { .. } => "ack",
    };
    let span = tracing::info_span!(span::INBOUND_EVENT, rubato.companion_id = %companion_id, rubato.user = %pairing.user, kind, seq = env.seq);
    async move {
        let now = Utc::now();
        if !matches!(env.message, CompanionMessage::Ack { .. }) {
            let payload = serde_json::to_value(&env.message).unwrap_or_default();
            match store::record_event(&app.db, companion_id, env.seq, kind, &payload, now).await {
                Ok(false) => {
                    tracing::debug!(seq = env.seq, "replayed envelope already recorded");
                    return Some(ServerMessage::Ack { seq: env.seq });
                }
                Ok(true) => {}
                Err(e) => tracing::error!(error = %e, "recording event"),
            }
            let _ = pairing::set_last_companion_seq(&app.db, companion_id, env.seq).await;
        }
        let _ = pairing::touch(&app.db, companion_id, now).await;
        match env.message {
            CompanionMessage::Hello {
                companion_id: claimed,
                protocol_version,
                user,
                ..
            } => {
                if claimed != companion_id || user != pairing.user {
                    app.metrics
                        .events_dropped
                        .add(1, &[KeyValue::new(attr::DROP_REASON, "auth")]);
                    return Some(ServerMessage::Error {
                        code: "auth".into(),
                        message: "hello does not match the pairing".into(),
                    });
                }
                if protocol_version != PROTOCOL_VERSION {
                    return Some(ServerMessage::Error {
                        code: "protocol".into(),
                        message: format!(
                            "server speaks protocol {PROTOCOL_VERSION}, companion {protocol_version}"
                        ),
                    });
                }
                Some(ServerMessage::Welcome {
                    server_version: crate::kernel::VERSION.to_owned(),
                    protocol_version: PROTOCOL_VERSION,
                    last_seen_companion_seq: pairing.last_companion_seq,
                    limits: app.config.limits,
                })
            }
            CompanionMessage::Heartbeat(h) => {
                app.metrics.companion_heartbeat.record(
                    h.at.timestamp() as u64,
                    &[
                        KeyValue::new(attr::COMPANION_ID, companion_id.to_string()),
                        KeyValue::new(attr::USER, pairing.user.clone()),
                    ],
                );
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Hook(h) => {
                tracing::info!(rubato.session_id = %h.session_id, rubato.hook_event = ?h.hook_event_name, "hook");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Sessions { sessions } => {
                if let Err(e) = store::store_snapshot(&app.db, companion_id, &sessions, now).await {
                    tracing::error!(error = %e, "storing snapshot");
                }
                let mut counts = std::collections::BTreeMap::new();
                for s in &sessions {
                    *counts
                        .entry(format!("{:?}", s.status).to_lowercase())
                        .or_insert(0u64) += 1;
                }
                for (state, n) in counts {
                    app.metrics.sessions_by_state.record(
                        n,
                        &[
                            KeyValue::new(attr::SESSION_STATE, state),
                            KeyValue::new(attr::COMPANION_ID, companion_id.to_string()),
                        ],
                    );
                }
                app.notify("sessions");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Launch(result) => {
                match launch::record_result(&app.db, &result).await {
                    Ok(Some(issue)) => {
                        tracing::info!(rubato.issue = %issue, outcome = ?std::mem::discriminant(&result), "launch result")
                    }
                    Ok(None) => tracing::warn!("launch result for an unknown command"),
                    Err(e) => tracing::error!(error = %e, "storing launch result"),
                }
                app.notify("launch");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::CapHit(hit) => {
                tracing::warn!(rubato.issue = %hit.issue, kind = ?hit.kind, value = hit.value, limit = hit.limit, "{}", rubato_protocol::otel::event::CAP_HIT);
                if let Err(e) = launch::record_cap_hit(&app.db, &hit).await {
                    tracing::error!(error = %e, "storing cap hit");
                }
                app.notify("cap hit");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Stale(s) => {
                tracing::warn!(rubato.session_id = ?s.session_id, silent_for_secs = s.silent_for_secs, "{}", rubato_protocol::otel::event::SESSION_STALE);
                let _ = store::store_stale(&app.db, &s).await;
                app.notify("stale");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Ack { .. } => None,
        }
    }
    .instrument(span)
    .await
}
