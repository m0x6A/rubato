//! Device-code pairing (ADR 0011): the companion asks the server for a code, the human enters
//! it in the web view while logged in with GitHub, the companion polls until approved and
//! stores the token with mode 0600.

use crate::kernel::config::State;
use anyhow::{Context, Result, bail};
use rubato_protocol::pairing::{
    PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse, routes,
};
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

pub struct Paired {
    pub token: String,
    pub user: String,
}

/// The whole `pair` command: keep the companion id this machine already has, pair, and write
/// `companion.json`. It writes nothing else: the sequence reservation in `seq.json` belongs to
/// the running companion, so pairing again never makes it reuse sequence numbers (plan 29 S3).
pub async fn pair_and_save(
    state_dir: &Path,
    server_url: &str,
    hostname: &str,
    version: &str,
    show: impl Fn(&PairStartResponse),
) -> Result<State> {
    let companion_id = State::load(state_dir)?
        .map(|s| s.companion_id)
        .unwrap_or_else(Uuid::new_v4);
    let paired = pair(server_url, companion_id, hostname, version, show).await?;
    let state = State {
        companion_id,
        server_url: server_url.to_owned(),
        user: paired.user,
        pairing_token: paired.token,
        last_seq: 0,
        last_server_seq: 0,
    };
    state.save(state_dir)?;
    Ok(state)
}

/// `show` is called once with the code to display. Returns when approved, denied or expired.
pub async fn pair(
    server_url: &str,
    companion_id: Uuid,
    hostname: &str,
    version: &str,
    show: impl Fn(&PairStartResponse),
) -> Result<Paired> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let base = server_url.trim_end_matches('/');
    let start: PairStartResponse = client
        .post(format!("{base}{}", routes::PAIR_START))
        .json(&PairStartRequest {
            companion_id,
            hostname: hostname.to_owned(),
            companion_version: version.to_owned(),
        })
        .send()
        .await
        .context("pair/start request")?
        .error_for_status()
        .context("pair/start status")?
        .json()
        .await
        .context("pair/start body")?;
    show(&start);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(start.expires_in_secs.max(1));
    let interval = Duration::from_secs(start.interval_secs.max(1));
    loop {
        if tokio::time::Instant::now() > deadline {
            bail!("pairing code expired before it was approved");
        }
        let poll: PairPollResponse = client
            .post(format!("{base}{}", routes::PAIR_POLL))
            .json(&PairPollRequest {
                device_code: start.device_code.clone(),
            })
            .send()
            .await
            .context("pair/poll request")?
            .error_for_status()
            .context("pair/poll status")?
            .json()
            .await
            .context("pair/poll body")?;
        match poll {
            PairPollResponse::Pending => tokio::time::sleep(interval).await,
            PairPollResponse::Approved { token, user } => return Ok(Paired { token, user }),
            PairPollResponse::Denied => bail!("pairing was denied"),
            PairPollResponse::Expired => bail!("pairing code expired"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;
    use axum::{Json, Router};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A server that approves at once and hands out a new token for every pairing.
    async fn approving_server() -> String {
        let pairings = Arc::new(AtomicU32::new(0));
        let app = Router::new()
            .route(
                routes::PAIR_START,
                post(|Json(_): Json<PairStartRequest>| async {
                    Json(PairStartResponse {
                        device_code: "dc".into(),
                        user_code: "ABCD-1234".into(),
                        verification_url: "http://test/pair".into(),
                        interval_secs: 0,
                        expires_in_secs: 30,
                    })
                }),
            )
            .route(
                routes::PAIR_POLL,
                post(move |Json(_): Json<PairPollRequest>| async move {
                    let n = pairings.fetch_add(1, Ordering::SeqCst) + 1;
                    Json(PairPollResponse::Approved {
                        token: format!("token-{n}"),
                        user: "m0x6A".into(),
                    })
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    #[tokio::test]
    async fn pairing_again_keeps_the_companion_id_and_leaves_the_reservation() {
        let url = approving_server().await;
        let dir = tempfile::tempdir().unwrap();
        let first = pair_and_save(dir.path(), &url, "host", "test", |_| {})
            .await
            .unwrap();
        assert_eq!(State::load(dir.path()).unwrap(), Some(first.clone()));
        assert!(
            !dir.path().join("seq.json").exists(),
            "pairing reserves nothing"
        );

        // The companion has run since: its reservation is on disk.
        let seq = dir.path().join("seq.json");
        std::fs::write(&seq, br#"{"reserved_to":5000}"#).unwrap();
        let second = pair_and_save(dir.path(), &url, "host", "test", |_| {})
            .await
            .unwrap();
        assert_eq!(second.companion_id, first.companion_id);
        assert_eq!(second.pairing_token, "token-2", "the new token is stored");
        assert_eq!(State::load(dir.path()).unwrap(), Some(second));
        assert_eq!(std::fs::read(&seq).unwrap(), br#"{"reserved_to":5000}"#);
    }
}
