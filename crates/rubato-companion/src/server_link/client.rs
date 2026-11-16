//! The outbound websocket to the server (ADR 0012): one connection, JSON envelopes with
//! sequence numbers, resume on reconnect. Nothing listens for the server; the companion calls.

use super::seq::{Reservation, ReservationError};
use crate::kernel::config::{Transport, check_server_url, warn_plain_http};
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

/// The server answered 401 to this companion's token: its pairing was revoked (ADR 0062).
#[derive(Debug, thiserror::Error)]
#[error("pairing revoked: the server refused this companion's token")]
pub struct Revoked;

#[derive(Clone)]
pub struct Identity {
    pub companion_id: Uuid,
    pub user: String,
    pub version: String,
    pub token: String,
    /// What this companion announces in Hello (`rubato_protocol::capability`).
    pub capabilities: Vec<String>,
    /// Interactive or headless (ADR 0062), sent in Hello.
    pub client: rubato_protocol::ClientKind,
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
    /// `pair --insecure` was used: plain http/ws off loopback is allowed (ADR 0067).
    allow_plain_http: bool,
}

/// `http(s)://host` → `ws(s)://host/ws/companion`. Refuses plain text off loopback unless
/// `allow_plain_http` (ADR 0067).
pub fn ws_url(server_url: &str, allow_plain_http: bool) -> Result<String> {
    check_server_url(server_url, allow_plain_http)?;
    let base = server_url.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_owned()
    };
    Ok(format!("{ws}{}", routes::COMPANION_WS))
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
            allow_plain_http: false,
        }
    }

    /// Accept an http/ws server URL off loopback: `allow_plain_http` from `companion.json`.
    pub fn allow_plain_http(mut self, allow: bool) -> Self {
        self.allow_plain_http = allow;
        self
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
        // Not a lost connection either: retrying cannot make a plain-text URL safe. Checked once
        // per start, so the opt-in warning is logged once per start, not per reconnect.
        if check_server_url(&self.server_url, self.allow_plain_http)? == Transport::PlainOffLoopback
        {
            warn_plain_http(&self.server_url);
        }
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
                // Reconnecting with a revoked token can never succeed: the companion stops, and
                // with it every turn it started (plan 122 §D14).
                Err(e) if e.is::<Revoked>() => {
                    tracing::error!(
                        "pairing revoked: the server refuses this companion's token; re-pair with `rubato-companion pair`"
                    );
                    return Err(e);
                }
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
        let mut req = ws_url(&self.server_url, self.allow_plain_http)?
            .into_client_request()
            .context("bad server url")?;
        req.headers_mut().insert(
            "authorization",
            format!("Bearer {}", self.identity.token).parse()?,
        );
        let (ws, _) = match tokio_tungstenite::connect_async(req).await {
            Ok(ok) => ok,
            // The server refuses a revoked pairing's token (ADR 0062): not a lost connection.
            Err(tokio_tungstenite::tungstenite::Error::Http(r)) if r.status() == 401 => {
                return Err(Revoked.into());
            }
            Err(e) => return Err(anyhow::Error::from(e).context("websocket connect")),
        };
        let (mut sink, mut stream) = ws.split();
        let hello = self.envelope(CompanionMessage::Hello {
            companion_id: self.identity.companion_id,
            user: self.identity.user.clone(),
            companion_version: self.identity.version.clone(),
            protocol_version: PROTOCOL_VERSION,
            last_seen_server_seq: self.last_server_seq,
            capabilities: self.identity.capabilities.clone(),
            client: self.identity.client,
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
                    // A fleet request is never replayed late (ADR 0072): sent now or not at all.
                    let keep = kept_in_outbox(&msg);
                    let env = self.envelope(msg)?;
                    let text = serde_json::to_string(&env)?;
                    if keep {
                        self.outbox.push_back(env);
                        if self.outbox.len() > 10_000 {
                            self.outbox.pop_front();
                        }
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
                                    drop_acked(&mut self.outbox, acked);
                                }
                                ServerMessage::Welcome { last_seen_companion_seq, .. } => {
                                    let acked = *last_seen_companion_seq;
                                    drop_acked(&mut self.outbox, acked);
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

/// Whether a message waits in the outbox for a replay after reconnect. A fleet request never
/// does (ADR 0072): a lead's request acted on minutes later would surprise everyone.
fn kept_in_outbox(m: &CompanionMessage) -> bool {
    !matches!(m, CompanionMessage::FleetRequest(_))
}

/// Drop what an ack covers. The outbox is in seq order (each envelope takes the next number as
/// it is pushed), so this pops from the front instead of scanning up to 10,000 entries.
fn drop_acked(outbox: &mut VecDeque<CompanionEnvelope>, acked: u64) {
    while outbox.front().is_some_and(|e| e.seq <= acked) {
        outbox.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;

    #[test]
    fn a_fleet_request_is_never_kept_for_a_replay() {
        use rubato_protocol::{FleetAction, FleetRequest};
        let fleet = CompanionMessage::FleetRequest(FleetRequest {
            request_id: uuid::Uuid::nil(),
            lead_launch_id: uuid::Uuid::nil(),
            action: FleetAction::List,
        });
        assert!(!kept_in_outbox(&fleet));
        assert!(kept_in_outbox(&CompanionMessage::Ack { seq: 1 }));
    }

    #[test]
    fn an_ack_pops_only_what_it_covers() {
        let outbox = |seqs: std::ops::RangeInclusive<u64>| -> VecDeque<CompanionEnvelope> {
            seqs.map(|seq| CompanionEnvelope {
                seq,
                sent_at: Utc::now(),
                message: CompanionMessage::Ack { seq },
            })
            .collect()
        };
        let left = |o: &VecDeque<CompanionEnvelope>| o.iter().map(|e| e.seq).collect::<Vec<_>>();
        let mut o = outbox(5..=9);
        drop_acked(&mut o, 7);
        assert_eq!(left(&o), vec![8, 9]);
        let mut o = outbox(5..=9);
        drop_acked(&mut o, 4);
        assert_eq!(
            left(&o),
            vec![5, 6, 7, 8, 9],
            "an ack below the front drops nothing"
        );
        let mut o = outbox(5..=9);
        drop_acked(&mut o, 100);
        assert!(o.is_empty(), "an ack past the back drops everything");
    }
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
            capabilities: vec![],
            client: rubato_protocol::ClientKind::Interactive,
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

    /// ADR 0062: a 401 at connect means the pairing was revoked. The client ends with `Revoked`
    /// instead of retrying forever, so the companion stops and its turns with it.
    #[tokio::test(flavor = "current_thread")]
    async fn a_401_at_connect_ends_the_client_as_revoked() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n")
                    .await;
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let seq = Reservation::open(dir.path(), 0).unwrap();
        let (_out_tx, out_rx) = mpsc::unbounded_channel();
        let (in_tx, _in_rx) = mpsc::unbounded_channel();
        let (connected, _connected_rx) = watch::channel(false);
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let client = Client::new(format!("http://{addr}"), identity(), seq, 0);
        let r = tokio::time::timeout(
            Duration::from_secs(5),
            client.run(out_rx, in_tx, connected, shutdown),
        )
        .await
        .expect("run returns instead of retrying a revoked token");
        let e = r.expect_err("a revoked pairing is an error");
        assert!(e.is::<Revoked>(), "not Revoked: {e:#}");
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
            super::ws_url("https://rubato.example/", false).unwrap(),
            "wss://rubato.example/ws/companion"
        );
        assert_eq!(
            super::ws_url("http://127.0.0.1:8080", false).unwrap(),
            "ws://127.0.0.1:8080/ws/companion"
        );
        assert_eq!(
            super::ws_url("http://192.168.1.20:8080", true).unwrap(),
            "ws://192.168.1.20:8080/ws/companion"
        );
        let refused = super::ws_url("http://rubato.example", false).unwrap_err();
        assert!(format!("{refused:#}").contains("https"), "{refused:#}");
    }
}
