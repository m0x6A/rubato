//! Thresholds both sides must agree on (A10). Overridable in config; these are the defaults.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Limits {
    /// Companion heartbeat interval.
    pub heartbeat_interval_secs: u64,
    /// A *working* session with no hook event for longer than this renders *unknown*.
    pub session_stale_secs: u64,
    /// A companion silent for longer than this fires `CompanionSilent`.
    pub companion_silent_secs: u64,
    /// Reconcile interval: `tmux list-panes` and identity, every this many seconds. The name is
    /// kept from the retired `tmux-ide team --json` poll (ADR 0070).
    pub team_poll_interval_secs: u64,
    /// How often the server looks for a plan file on a live branch.
    pub plan_poll_interval_secs: u64,
    /// How often the server polls GitHub for PRs, checks and decision comments per live issue.
    pub github_poll_interval_secs: u64,
    /// The longest session message the companion types (ADR 0051); longer ones are refused.
    pub session_message_max_chars: u64,
    /// The longest prompt or reply the companion forwards for the session log (ADR 0052); longer
    /// text is cut and marked truncated.
    pub session_log_max_chars: u64,
    /// The largest plan bundle the companion mirrors (ADR 0053); a larger one is sent without
    /// its files, refused as too large.
    pub draft_max_bytes: u64,
    /// The longest room message a person may post, in characters (ADR 0057).
    pub room_message_max_chars: u64,
    /// The room context a turn carries, in characters (A9).
    pub room_context_max_chars: u64,
    /// An agent not addressed for this long leaves the room (ADR 0057).
    pub room_agent_idle_secs: u64,
    /// Turns made by agents' messages in a row, with no person posting, before Rubato stops.
    pub room_agent_turn_limit: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            heartbeat_interval_secs: 30,
            session_stale_secs: 120,
            companion_silent_secs: 600,
            team_poll_interval_secs: 30,
            plan_poll_interval_secs: 60,
            github_poll_interval_secs: 60,
            session_message_max_chars: 8000,
            session_log_max_chars: 16000,
            draft_max_bytes: 1_048_576,
            room_message_max_chars: 8000,
            room_context_max_chars: 16000,
            room_agent_idle_secs: 1800,
            room_agent_turn_limit: 3,
        }
    }
}

/// The longest `doing` line of an `AgentReport` (ADR 0071); the companion cuts longer ones.
pub const AGENT_DOING_MAX_CHARS: usize = 200;
/// The most map ids an `AgentReport` names (ADR 0071); the companion keeps the first ones.
pub const AGENT_PARTS_MAX: usize = 8;

/// The most context graph sources one `GraphSources` names (ADR 0086).
pub const GRAPH_SOURCES_MAX: usize = 64;
/// The largest repository tarball the server downloads for a graph source, compressed
/// (ADR 0084). Over a cap a source is marked `truncated`; it is never a failed build.
pub const GRAPH_TARBALL_MAX_BYTES: u64 = 256 * 1024 * 1024;
/// The most uncompressed bytes the server reads from one tarball.
pub const GRAPH_TARBALL_READ_MAX_BYTES: u64 = 1024 * 1024 * 1024;
/// The most files of one repository that become nodes.
pub const GRAPH_FILES_MAX: usize = 50_000;
/// The most nodes, and edges, of one source.
pub const GRAPH_NODES_MAX: usize = 250_000;
pub const GRAPH_EDGES_MAX: usize = 1_000_000;
/// The largest file the server parses for symbols; a larger one stays a `file` node.
pub const GRAPH_PARSE_FILE_MAX_BYTES: usize = 512 * 1024;
/// The longest signature line and doc line a `symbol` node carries.
pub const GRAPH_SIGNATURE_MAX_CHARS: usize = 200;
/// The most of a decision document's Decision section an `adr` node carries.
pub const GRAPH_DECISION_MAX_BYTES: usize = 4096;
/// The largest snapshot body a companion accepts, compressed and uncompressed (ADR 0086).
pub const GRAPH_SNAPSHOT_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const GRAPH_SNAPSHOT_READ_MAX_BYTES: u64 = 512 * 1024 * 1024;
/// What a graph tool answers at most (ADR 0086): the text of one answer, the nodes of a search,
/// the nodes and the depth of a traversal.
pub const GRAPH_ANSWER_MAX_BYTES: usize = 64 * 1024;
pub const GRAPH_SEARCH_LIMIT_MAX: usize = 50;
pub const GRAPH_RELATED_LIMIT_MAX: usize = 200;
pub const GRAPH_RELATED_DEPTH_MAX: u8 = 3;
