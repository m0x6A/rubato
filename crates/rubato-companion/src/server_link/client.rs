//! The outbound websocket to the server (ADR 0012): one connection, JSON envelopes with
//! sequence numbers, resume on reconnect. Nothing listens for the server; the companion calls.

use super::seq::{Reservation, ReservationError};
use anyhow::{Context, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use rubato_protocol::pairing::routes;
use rubato_protocol::{
    CompanionEnvelope, CompanionMessage, PROTOCOL_VERSION, ServerEnvelope, ServerMessage,
};
use std::collections::VecDeque;
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
    /// Sequence numbers come from the reservation in `seq.json`, never from a counter that
    /// restarts with the process.
    seq: Reservation,
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
        seq: Reservation,
        last_server_seq: u64,
    ) -> Self {
        Self {
            server_url,
            identity,
            seq,
            outbox: VecDeque::new(),
            last_server_seq,
        }
    }

    fn envelope(
        &mut self,
        message: CompanionMessage,
    ) -> Result<CompanionEnvelope, ReservationError> {
        let seq = self.seq.next_seq()?;
        Ok(CompanionEnvelope {
            seq,
            sent_at: Utc::now(),
            message,
        })
    }

    /// Run until `shutdown` flips. `rx` carries outbound messages; `tx_in` receives server messages;
    /// `connected` reports connection state. A lost connection is retried; a sequence
    /// reservation that cannot be written ends the run with that error (plan 29 S4).
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
                // Not a lost connection: reconnecting cannot make `seq.json` writable, and
                // sending without a reservation would reuse numbers after the next restart.
                Err(e) if e.is::<ReservationError>() => return Err(e),
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
        })?;
        let hello_seq = hello.seq;
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
                    let env = self.envelope(msg)?;
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
                                    // A hello is never in the outbox, so an ack for it means the
                                    // server had this number on record already: a collision, and
                                    // everything sent below its record is being dropped.
                                    if acked == hello_seq {
                                        tracing::error!(
                                            start_seq = self.seq.start_seq(),
                                            reserved_to = self.seq.reserved_to(),
                                            "hello treated as a replay"
                                        );
                                    }
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
                            let ack = self.envelope(CompanionMessage::Ack { seq: env.seq })?;
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
    use super::*;
    use crate::kernel::telemetry::capture;
    use crate::server_link::seq::BLOCK;
    use std::future::Future;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::WebSocketStream;

    /// A websocket on 127.0.0.1 that hands every accepted connection to `serve`.
    async fn ws_server<F, Fut>(serve: F) -> String
    where
        F: Fn(WebSocketStream<TcpStream>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                if let Ok(ws) = tokio_tungstenite::accept_async(tcp).await {
                    tokio::spawn(serve(ws));
                }
            }
        });
        url
    }

    fn identity() -> Identity {
        Identity {
            companion_id: Uuid::nil(),
            user: "m0x6A".into(),
            version: "test".into(),
            token: "t".into(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_reservation_ends_the_client() {
        let url = ws_server(|mut ws| async move { while ws.next().await.is_some() {} }).await;
        // A reservation at the end of its block whose state dir has gone: the hello needs a
        // new block and cannot get one.
        let dir = tempfile::tempdir().unwrap();
        let mut seq = Reservation::open(dir.path(), 0).unwrap();
        for _ in 0..BLOCK {
            seq.next_seq().unwrap();
        }
        let seq_path = Reservation::path(dir.path());
        drop(dir);

        let (_out_tx, out_rx) = mpsc::unbounded_channel();
        let (in_tx, _in_rx) = mpsc::unbounded_channel();
        let (connected, _connected_rx) = watch::channel(false);
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let client = Client::new(url, identity(), seq, 0);
        let r = tokio::time::timeout(
            Duration::from_secs(5),
            client.run(out_rx, in_tx, connected, shutdown),
        )
        .await
        .expect("run returns instead of retrying as if the connection were lost");
        let e = r.expect_err("a failed reservation is an error");
        let reservation = e
            .downcast_ref::<ReservationError>()
            .unwrap_or_else(|| panic!("not a reservation error: {e:#}"));
        assert_eq!(reservation.path, seq_path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_hello_answered_with_a_bare_ack_is_logged_as_a_replay() {
        let (events, _g) = capture::start();
        // What rubato-server does with a sequence number it has already recorded.
        let url = ws_server(|mut ws| async move {
            while let Some(Ok(frame)) = ws.next().await {
                let Message::Text(t) = frame else { continue };
                let env: CompanionEnvelope = serde_json::from_str(&t).unwrap();
                if matches!(env.message, CompanionMessage::Hello { .. }) {
                    let ack = ServerEnvelope {
                        seq: 1,
                        sent_at: Utc::now(),
                        message: ServerMessage::Ack { seq: env.seq },
                    };
                    let text = serde_json::to_string(&ack).unwrap();
                    ws.send(Message::Text(text.into())).await.unwrap();
                }
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let seq = Reservation::open(dir.path(), 100).unwrap();

        let (_out_tx, out_rx) = mpsc::unbounded_channel();
        let (in_tx, mut in_rx) = mpsc::unbounded_channel();
        let (connected, _connected_rx) = watch::channel(false);
        let (shutdown_tx, shutdown) = watch::channel(false);
        let client = Client::new(url, identity(), seq, 0);
        let run = tokio::spawn(client.run(out_rx, in_tx, connected, shutdown));
        let got = tokio::time::timeout(Duration::from_secs(5), in_rx.recv())
            .await
            .expect("the ack reached the client");
        assert!(
            matches!(got, Some(ServerMessage::Ack { seq: 101 })),
            "{got:?}"
        );
        shutdown_tx.send(true).unwrap();
        run.await.unwrap().unwrap();

        let events = events.lock().unwrap();
        let replays: Vec<_> = events
            .iter()
            .filter(|e| e.message == "hello treated as a replay")
            .collect();
        assert_eq!(replays.len(), 1, "{events:?}");
        assert_eq!(replays[0].level, tracing::Level::ERROR);
        assert_eq!(
            replays[0].fields.get("start_seq").map(String::as_str),
            Some("100")
        );
        assert_eq!(
            replays[0].fields.get("reserved_to").map(String::as_str),
            Some("1100")
        );
    }

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
