//! Device-code pairing (ADR 0011): the companion asks the server for a code, the human enters
//! it in the web view while logged in with GitHub, the companion polls until approved and
//! stores the token with mode 0600.

use anyhow::{Context, Result, bail};
use rubato_protocol::pairing::{
    PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse, routes,
};
use std::time::Duration;
use uuid::Uuid;

pub struct Paired {
    pub token: String,
    pub user: String,
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
