//! Device-code pairing of a companion with a GitHub login (ADR 0011), and the HTTP routes on
//! the `companion-server` seam.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Route paths both sides agree on.
pub mod routes {
    /// `POST` [`super::PairStartRequest`] → [`super::PairStartResponse`].
    pub const PAIR_START: &str = "/api/pair/start";
    /// `POST` [`super::PairPollRequest`] → [`super::PairPollResponse`].
    pub const PAIR_POLL: &str = "/api/pair/poll";
    /// Websocket upgrade with `Authorization: Bearer <pairing token>`.
    pub const COMPANION_WS: &str = "/ws/companion";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PairStartRequest {
    pub companion_id: Uuid,
    pub hostname: String,
    pub companion_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PairStartResponse {
    /// Opaque; the companion polls with it.
    pub device_code: String,
    /// Shown to the human, who enters it in the web view while logged in.
    pub user_code: String,
    pub verification_url: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PairPollRequest {
    pub device_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PairPollResponse {
    Pending,
    /// The human approved. `token` is shown once; the server keeps only its hash.
    Approved {
        token: String,
        user: String,
    },
    Denied,
    Expired,
}
