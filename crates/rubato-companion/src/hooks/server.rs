//! The loopback hook receiver (ADR 0013): `POST /hook` on 127.0.0.1 with a per-start bearer
//! token. Claude Code's hook command is `curl ... --data-binary @-`, so the body is the hook's
//! stdin JSON as is.

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
    /// Bind an ephemeral loopback port. `dropped` receives a reason per rejected request, for the
    /// `rubato.events.dropped` counter.
    pub async fn bind(
        tx: mpsc::UnboundedSender<HookEvent>,
        dropped: mpsc::UnboundedSender<&'static str>,
    ) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let token = new_token();
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
        Some(ev) => {
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
}
