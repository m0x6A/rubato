//! Envelopes on the `companion-server` seam (ADR 0012): JSON over one outbound websocket.

use crate::agent::{AgentReport, PlanStatus};
use crate::handoff::HandoffReport;
use crate::hook::HookEvent;
use crate::launch::{CapHit, LaunchCommand, LaunchResult};
use crate::limits::Limits;
use crate::plan_draft::DraftBundle;
use crate::session::{ClientKind, Heartbeat, SessionState, StaleSession};
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
    /// The companion types `plan_approved` and forwards the skill's handoff report (ADR 0055).
    pub const PLAN_HANDOFF: &str = "plan_handoff";
    /// The server takes `CompanionMessage::HandoffReport`.
    pub const HANDOFF_REPORTS: &str = "handoff_reports";
    /// The companion writes `LaunchCommand.draft` into a worktree the launch creates (ADR 0053).
    pub const DRAFT_RESTORE: &str = "draft_restore";
    /// The companion runs in headless mode (ADR 0062) and may send `source: headless` snapshots.
    pub const HEADLESS: &str = "headless";
    /// Companion: it takes `SessionMessageKind::Stop`. Server: it takes `source: headless`
    /// snapshots and offers Stop. Neither side sends a stop or a headless snapshot otherwise.
    pub const SESSION_STOP: &str = "session_stop";
    /// The companion types room turns and runs read-only fork turns (ADR 0057).
    pub const ROOM_TURNS: &str = "room_turns";
    /// The server takes `CompanionMessage::RoomReply`.
    pub const ROOM_REPLIES: &str = "room_replies";
    /// Companion: it serves MCP to its launched sessions and takes `ServerMessage::PlanStatus`.
    /// Server: it takes `CompanionMessage::AgentReport` (ADR 0071). One name both ways.
    pub const AGENT_MCP: &str = "agent_mcp";
    /// Companion: its orchestrator flag is on and it serves the fleet tools to a lead launch.
    /// Server: `RUBATO_ORCHESTRATOR=1`; it takes `FleetRequest` and sends `FleetResult` (ADR 0072).
    pub const FLEET: &str = "fleet";
    /// Companion: it keeps a copy of the context graph and takes `ServerMessage::GraphSources`.
    /// Server: its context graph flag is on and it serves snapshots (ADR 0084, ADR 0086).
    pub const CONTEXT_GRAPH: &str = "context_graph";
    /// Prefix of how the companion's Claude Code is authenticated, `claude_auth:api_key`: the kind
    /// only, never a value (ADR 0076).
    pub const CLAUDE_AUTH_PREFIX: &str = "claude_auth:";
    /// Prefix of the installed skill's version, `skill:0.2.0`.
    pub const SKILL_PREFIX: &str = "skill:";
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
        /// Interactive (tmux) or headless (ADR 0062). An older companion is interactive.
        #[serde(default, skip_serializing_if = "ClientKind::is_interactive")]
        client: ClientKind,
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
    /// What `plan.ts handoff` reported to the loopback endpoint (ADR 0055).
    HandoffReport(HandoffReport),
    /// An agent's answer to a room turn (ADR 0057).
    RoomReply(crate::room::RoomReply),
    /// What a launched session's agent said it is doing, through MCP (ADR 0071). Sent only to a
    /// server that announced `agent_mcp`.
    AgentReport(AgentReport),
    /// A lead launch's fleet tool call (ADR 0072). Sent only to a server that announced `fleet`,
    /// only while the link is up, and never kept in the outbox.
    FleetRequest(crate::fleet::FleetRequest),
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
    /// Where the plan of one session this companion launched stands (ADR 0071). Sent only to a
    /// companion that announced `agent_mcp`.
    PlanStatus(Box<PlanStatus>),
    /// The answer to one `FleetRequest` (ADR 0072). Sent only to a companion that announced `fleet`.
    FleetResult(Box<crate::fleet::FleetResult>),
    /// Every context graph source the companion may hold (ADR 0086). Sent only to a companion
    /// that announced `context_graph` and whose owner may read the graph.
    GraphSources(crate::graph::GraphSources),
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
    /// The W3C trace context of the companion's send span (ADR 0081), read with
    /// `otel::TraceParent::parse`. The server uses it only as the parent of its inbound span;
    /// one that does not parse is handled as absent. Left out by an older companion and when
    /// the companion exports no traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
    pub message: CompanionMessage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServerEnvelope {
    /// Monotonic per pairing.
    pub seq: u64,
    pub sent_at: DateTime<Utc>,
    pub message: ServerMessage,
}
