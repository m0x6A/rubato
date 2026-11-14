//! The loopback hook receiver (ADR 0013): `POST /hook` on 127.0.0.1 with a bearer token.
//! Claude Code's hook command is `curl ... --data-binary @-`, so the body is the hook's stdin
//! JSON as is. A launched session's command also sends `X-Rubato-Launch` (plan 122 §D1). The
//! port and token are kept across restarts when the port can be bound again, so sessions
//! launched earlier keep posting to a live port.

use crate::kernel::config::HookEndpoint;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use chrono::Utc;
use rand::RngCore;
use rubato_protocol::HookEvent;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use uuid::Uuid;

/// The header a launched session's hook command sends with its launch id.
pub const LAUNCH_HEADER: &str = "x-rubato-launch";

#[derive(Clone)]
struct AppState {
    token: Arc<String>,
    tx: mpsc::UnboundedSender<HookEvent>,
    dropped: mpsc::UnboundedSender<&'static str>,
}

pub struct HookServer {
    pub endpoint: HookEndpoint,
    listener: TcpListener,
    state: AppState,
}

pub fn new_token() -> String {
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    // URL-safe base64 without padding, hand-rolled to avoid a dependency.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(32);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8 | chunk[2] as u32;
        for i in (0..4).rev() {
            out.push(ALPHABET[((n >> (6 * i)) & 63) as usize] as char);
        }
    }
    out
}

impl HookServer {
    /// Bind an ephemeral loopback port with a new token. `dropped` receives a reason per
    /// rejected request, for the `rubato.events.dropped` counter.
    pub async fn bind(
        tx: mpsc::UnboundedSender<HookEvent>,
        dropped: mpsc::UnboundedSender<&'static str>,
    ) -> anyhow::Result<Self> {
        Self::bind_reusing(None, tx, dropped).await
    }

    /// Bind `previous`'s port and keep its token when the port is free, so the hooks files of
    /// sessions launched before a restart still reach this receiver. Otherwise an ephemeral
    /// port with a new token: compare the result's endpoint with `previous` to see which.
    pub async fn bind_reusing(
        previous: Option<&HookEndpoint>,
        tx: mpsc::UnboundedSender<HookEvent>,
        dropped: mpsc::UnboundedSender<&'static str>,
    ) -> anyhow::Result<Self> {
        let kept = match previous {
            Some(ep) if ep.port != 0 && !ep.token.is_empty() => {
                match TcpListener::bind(("127.0.0.1", ep.port)).await {
                    Ok(l) => Some((l, ep.token.clone())),
                    Err(e) => {
                        tracing::debug!(port = ep.port, error = %e, "previous hook port not free");
                        None
                    }
                }
            }
            _ => None,
        };
        let (listener, token) = match kept {
            Some(k) => k,
            None => (TcpListener::bind(("127.0.0.1", 0)).await?, new_token()),
        };
        let port = listener.local_addr()?.port();
        Ok(Self {
            endpoint: HookEndpoint {
                port,
                token: token.clone(),
            },
            listener,
            state: AppState {
                token: Arc::new(token),
                tx,
                dropped,
            },
        })
    }

    pub async fn serve(self) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/hook", post(receive))
            .route("/healthz", get(|| async { "ok" }))
            .with_state(self.state);
        axum::serve(self.listener, app).await?;
        Ok(())
    }
}

/// The launch id a launched session's hook command sends. Empty (a session not launched from
/// Rubato expands the variable to nothing) or not a UUID reads as none.
fn launch_id(headers: &HeaderMap) -> Option<Uuid> {
    let v = headers.get(LAUNCH_HEADER)?.to_str().ok()?.trim();
    if v.is_empty() {
        return None;
    }
    Uuid::parse_str(v).ok()
}

async fn receive(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth.strip_prefix("Bearer ").map(str::trim) != Some(st.token.as_str()) {
        let _ = st.dropped.send("auth");
        return StatusCode::UNAUTHORIZED;
    }
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            let _ = st.dropped.send("schema");
            return StatusCode::BAD_REQUEST;
        }
    };
    match HookEvent::from_payload(payload, Utc::now()) {
        Some(mut ev) => {
            ev.launch_id = launch_id(&headers);
            if st.tx.send(ev).is_err() {
                let _ = st.dropped.send("overflow");
                return StatusCode::SERVICE_UNAVAILABLE;
            }
            StatusCode::ACCEPTED
        }
        None => {
            let _ = st.dropped.send("schema");
            StatusCode::BAD_REQUEST
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_url_safe_and_long() {
        let t = new_token();
        assert_eq!(t.len(), 32);
        assert!(
            t.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(t, new_token());
    }

    #[test]
    fn the_launch_header_is_read_and_an_empty_or_bad_one_ignored() {
        let id = Uuid::from_u128(7);
        let with = |v: &str| {
            let mut h = HeaderMap::new();
            h.insert(LAUNCH_HEADER, v.parse().unwrap());
            launch_id(&h)
        };
        assert_eq!(with(&id.to_string()), Some(id));
        assert_eq!(with(&format!(" {id} ")), Some(id));
        assert_eq!(with(""), None);
        assert_eq!(with("not-a-uuid"), None);
        assert_eq!(launch_id(&HeaderMap::new()), None);
    }

    /// The port and token come back when the port is free; a taken port gets a new port and a
    /// new token, never the old token on another port.
    #[tokio::test]
    async fn a_restart_keeps_the_port_and_token_when_it_can() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (dtx, _drx) = mpsc::unbounded_channel();
        // Between the drop and the rebind another test may take the freed ephemeral port, which
        // is the taken-port case below, not a failure of this one: try a few fresh ports.
        let mut kept = None;
        for _ in 0..5 {
            let first = HookServer::bind(tx.clone(), dtx.clone()).await.unwrap();
            let previous = first.endpoint.clone();
            drop(first);
            let again = HookServer::bind_reusing(Some(&previous), tx.clone(), dtx.clone())
                .await
                .unwrap();
            if again.endpoint.port == previous.port {
                assert_eq!(again.endpoint, previous, "a kept port keeps its token");
                kept = Some((again, previous));
                break;
            }
        }
        let (_again, previous) = kept.expect("the freed port was taken five times in a row");

        // `_again` still holds the port.
        let moved = HookServer::bind_reusing(Some(&previous), tx.clone(), dtx.clone())
            .await
            .unwrap();
        assert_ne!(moved.endpoint.port, previous.port);
        assert_ne!(moved.endpoint.token, previous.token);

        let fresh = HookServer::bind_reusing(None, tx, dtx).await.unwrap();
        assert_ne!(fresh.endpoint.token, previous.token);
    }
}
