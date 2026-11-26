//! Asking the server for one source's snapshot (ADR 0086): one GET with the pairing token,
//! through the same gate as the websocket (TLS, except on loopback or for a pairing made with
//! `--insecure`, ADR 0067), and never more bytes than the cap.

use crate::kernel::config::check_server_url;
use rubato_protocol::graph::routes::GRAPH_SNAPSHOT;
use rubato_protocol::limits;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// What the server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// The gzipped snapshot body.
    Body(Vec<u8>),
    /// 401: the pairing is unknown or revoked. The copy goes.
    Unauthorized,
    /// 403: the owner has no fresh team record. The copy stays and is marked not refreshed.
    NotEntitled,
    /// Anything else: not reached, a refusal to send the token in clear text, 404, 5xx, a body
    /// over the cap.
    Failed(String),
}

/// How the sync fetches: the real client, or a test's stand-in.
pub type Fetch = Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Fetched> + Send>> + Send + Sync>;

/// The real fetch, against the paired server.
pub fn http(server_url: String, token: String, allow_plain_http: bool) -> Fetch {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_default();
    Arc::new(move |source: String| {
        let (client, server_url, token) = (client.clone(), server_url.clone(), token.clone());
        Box::pin(async move { get(&client, &server_url, &token, allow_plain_http, &source).await })
    })
}

async fn get(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    allow_plain_http: bool,
    source: &str,
) -> Fetched {
    // Before the token goes anywhere: the same rule the websocket is held to.
    if let Err(e) = check_server_url(server_url, allow_plain_http) {
        return Fetched::Failed(format!("{e:#}"));
    }
    let mut url = match url::Url::parse(&format!(
        "{}{GRAPH_SNAPSHOT}",
        server_url.trim_end_matches('/')
    )) {
        Ok(url) => url,
        Err(e) => return Fetched::Failed(format!("server URL: {e}")),
    };
    url.query_pairs_mut().append_pair("source", source);
    let mut resp = match client.get(url).bearer_auth(token).send().await {
        Ok(r) => r,
        Err(e) => return Fetched::Failed(format!("request: {e}")),
    };
    match resp.status().as_u16() {
        200 => {}
        401 => return Fetched::Unauthorized,
        403 => return Fetched::NotEntitled,
        other => return Fetched::Failed(format!("HTTP {other}")),
    }
    let mut body = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if (body.len() + chunk.len()) as u64 > limits::GRAPH_SNAPSHOT_MAX_BYTES {
                    return Fetched::Failed(format!(
                        "snapshot is over {} bytes",
                        limits::GRAPH_SNAPSHOT_MAX_BYTES
                    ));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Fetched::Body(body),
            Err(e) => return Fetched::Failed(format!("body: {e}")),
        }
    }
}
