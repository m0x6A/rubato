//! Envelopes on the `companion-server` seam (ADR 0012): JSON over one outbound websocket.

use crate::hook::HookEvent;
use crate::launch::{CapHit, LaunchCommand, LaunchResult};
use crate::limits::Limits;
use crate::plan_draft::DraftBundle;
use crate::session::{Heartbeat, SessionState, StaleSession};
use crate::session_message::{SessionMessage, SessionMessageResult};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Bumped on any breaking change to the envelopes. Both sides refuse a mismatch.
pub const PROTOCOL_VERSION: u32 = 1;

/// What each side can take (ADR 0051). Each announces its own in Hello or Welcome, and neither
/// sends a kind the other did not announce. Unknown names are ignored, so either side can add one.
pub mod capability {
    /// The companion types session messages into the sessions it launched.
    pub const SESSION_INPUT: &str = "session_input";
    /// The companion forwards only the allowlisted hook payload (ADR 0052).
    pub const HOOK_ALLOWLIST: &str = "hook_allowlist";
    /// The server takes `SessionMessageResult` and sends `SessionMessage`.
    pub const SESSION_MESSAGES: &str = "session_messages";
    /// The companion mirrors launched sessions' plan bundles at settle points (ADR 0053).
    pub const DRAFT_MIRROR: &str = "draft_mirror";
    /// The server takes `CompanionMessage::Draft`.
    pub const DRAFTS: &str = "drafts";
}

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
        /// What this companion can take (`capability`). Absent from an older companion.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capabilities: Vec<String>,
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
    /// The answer to one `SessionMessage` (ADR 0051).
    SessionMessageResult(SessionMessageResult),
    /// A launched session's plan bundle at a settle point (ADR 0053).
    Draft(DraftBundle),
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
        /// What this server can take (`capability`). Absent from an older server.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capabilities: Vec<String>,
    },
    Launch(LaunchCommand),
    /// Text for one session the paired user launched (ADR 0051).
    SessionMessage(SessionMessage),
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
