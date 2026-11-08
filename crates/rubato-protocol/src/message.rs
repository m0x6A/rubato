//! Envelopes on the `companion-server` seam (ADR 0012): JSON over one outbound websocket.

use crate::hook::HookEvent;
use crate::launch::{CapHit, LaunchCommand, LaunchResult};
use crate::limits::Limits;
use crate::session::{Heartbeat, SessionState, StaleSession};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Bumped on any breaking change to the envelopes. Both sides refuse a mismatch.
pub const PROTOCOL_VERSION: u32 = 1;

/// Companion → server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompanionMessage {
    /// First message after connect. Carries the pairing identity and where to resume from.
    Hello {
        companion_id: Uuid,
        user: String,
        companion_version: String,
        protocol_version: u32,
        /// Last server sequence the companion has seen; the server replays newer commands.
        last_seen_server_seq: u64,
    },
    Heartbeat(Heartbeat),
    Hook(HookEvent),
    /// Full snapshot of the sessions this companion sees. Replaces the previous snapshot.
    Sessions {
        sessions: Vec<SessionState>,
    },
    Launch(LaunchResult),
    CapHit(CapHit),
    Stale(StaleSession),
    /// Acknowledges a server envelope by sequence.
    Ack {
        seq: u64,
    },
}

/// Server → companion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Answer to `Hello`.
    Welcome {
        server_version: String,
        protocol_version: u32,
        /// Last companion sequence the server has; the companion replays newer events.
        last_seen_companion_seq: u64,
        limits: Limits,
    },
    Launch(LaunchCommand),
    Ack {
        seq: u64,
    },
    /// The connection will be closed. `code` is stable; `message` is for humans.
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompanionEnvelope {
    /// Monotonic per companion, starting at 1, persisted across restarts.
    pub seq: u64,
    pub sent_at: DateTime<Utc>,
    pub message: CompanionMessage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServerEnvelope {
    /// Monotonic per pairing.
    pub seq: u64,
    pub sent_at: DateTime<Utc>,
    pub message: ServerMessage,
}
