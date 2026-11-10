//! The outbound websocket to the server (ADR 0012): one connection, JSON envelopes with
//! sequence numbers, resume on reconnect. Nothing listens for the server; the companion calls.

use anyhow::{Context, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use rubato_protocol::pairing::routes;
use rubato_protocol::{
    CompanionEnvelope, CompanionMessage, PROTOCOL_VERSION, ServerEnvelope, ServerMessage,
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

#[derive(Clone)]
pub struct Identity {
    pub companion_id: Uuid,
    pub user: String,
    pub version: String,
    pub token: String,
}

pub struct Client {
    pub server_url: String,
    pub identity: Identity,
    pub seq: Arc<AtomicU64>,
    /// Envelopes sent and not yet acknowledged; replayed after a reconnect.
    outbox: VecDeque<CompanionEnvelope>,
    last_server_seq: u64,
}

/// `http(s)://host` → `ws(s)://host/ws/companion`.
pub fn ws_url(server_url: &str) -> String {
    let base = server_url.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_owned()
    };
    format!("{ws}{}", routes::COMPANION_WS)
}

impl Client {
    pub fn new(
        server_url: String,
        identity: Identity,
        start_seq: u64,
        last_server_seq: u64,
    ) -> Self {
        Self {
            server_url,
            identity,
            seq: Arc::new(AtomicU64::new(start_seq)),
            outbox: VecDeque::new(),
            last_server_seq,
        }
    }

    fn envelope(&mut self, message: CompanionMessage) -> CompanionEnvelope {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        CompanionEnvelope {
            seq,
            sent_at: Utc::now(),
            message,
        }
    }

    /// Run until `shutdown` flips. `rx` carries outbound messages; `tx_in` receives server messages;
    /// `connected` reports connection state.
    pub async fn run(
        mut self,
        mut rx: mpsc::UnboundedReceiver<CompanionMessage>,
        tx_in: mpsc::UnboundedSender<ServerMessage>,
        connected: watch::Sender<bool>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<()> {
        let mut backoff = Duration::from_secs(1);
        loop {
            if *shutdown.borrow() {
                return Ok(());
            }
            match self
                .session(&mut rx, &tx_in, &connected, &mut shutdown)
                .await
            {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let _ = connected.send(false);
                    tracing::warn!(error = %e, retry_in_secs = backoff.as_secs(), "server connection lost");
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = shutdown.changed() => return Ok(()),
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }

    async fn session(
        &mut self,
        rx: &mut mpsc::UnboundedReceiver<CompanionMessage>,
        tx_in: &mpsc::UnboundedSender<ServerMessage>,
        connected: &watch::Sender<bool>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<()> {
        let mut req = ws_url(&self.server_url)
            .into_client_request()
            .context("bad server url")?;
        req.headers_mut().insert(
            "authorization",
            format!("Bearer {}", self.identity.token).parse()?,
        );
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .context("websocket connect")?;
        let (mut sink, mut stream) = ws.split();
        let hello = self.envelope(CompanionMessage::Hello {
            companion_id: self.identity.companion_id,
            user: self.identity.user.clone(),
            companion_version: self.identity.version.clone(),
            protocol_version: PROTOCOL_VERSION,
            last_seen_server_seq: self.last_server_seq,
        });
        sink.send(Message::Text(serde_json::to_string(&hello)?.into()))
            .await?;
        // Replay what the server has not acknowledged.
        for env in &self.outbox {
            sink.send(Message::Text(serde_json::to_string(env)?.into()))
                .await?;
        }
        let _ = connected.send(true);
        tracing::info!(server = %self.server_url, "connected");
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    let _ = sink.close().await;
                    return Ok(());
                }
                out = rx.recv() => {
                    let Some(msg) = out else { return Ok(()); };
                    let env = self.envelope(msg);
                    let text = serde_json::to_string(&env)?;
                    self.outbox.push_back(env);
                    if self.outbox.len() > 10_000 {
                        self.outbox.pop_front();
                    }
                    sink.send(Message::Text(text.into())).await.context("send")?;
                }
                inbound = stream.next() => {
                    let Some(frame) = inbound else { anyhow::bail!("server closed the connection") };
                    match frame.context("receive")? {
                        Message::Text(t) => {
                            let env: ServerEnvelope = match serde_json::from_str(&t) {
                                Ok(e) => e,
                                Err(e) => {
                                    tracing::warn!(error = %e, "undecodable server envelope dropped");
                                    continue;
                                }
                            };
                            self.last_server_seq = self.last_server_seq.max(env.seq);
                            match &env.message {
                                ServerMessage::Ack { seq } => {
                                    let acked = *seq;
                                    self.outbox.retain(|e| e.seq > acked);
                                }
                                ServerMessage::Welcome { last_seen_companion_seq, .. } => {
                                    let acked = *last_seen_companion_seq;
                                    self.outbox.retain(|e| e.seq > acked);
                                }
                                ServerMessage::Error { code, message } => {
                                    tracing::error!(code, message, "server error");
                                }
                                _ => {}
                            }
                            let ack = self.envelope(CompanionMessage::Ack { seq: env.seq });
                            sink.send(Message::Text(serde_json::to_string(&ack)?.into())).await?;
                            if tx_in.send(env.message).is_err() {
                                return Ok(());
                            }
                        }
                        Message::Ping(p) => sink.send(Message::Pong(p)).await?,
                        Message::Close(_) => anyhow::bail!("server closed the connection"),
                        _ => {}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ws_urls() {
        assert_eq!(
            super::ws_url("https://rubato.example/"),
            "wss://rubato.example/ws/companion"
        );
        assert_eq!(
            super::ws_url("http://127.0.0.1:8080"),
            "ws://127.0.0.1:8080/ws/companion"
        );
    }
}
