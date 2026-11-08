//! `rubato-protocol` is the contract between `rubato-companion`, `rubato-server` and `web`.
//!
//! Serde types only: no I/O, no async runtime. Both binaries depend on it and the web
//! frontend consumes the JSON Schema exported from it (`schema/protocol/`). A breaking
//! change here is a seam change (`companion-server`, `web-server`) and blocks.

pub mod github;
pub mod hook;
pub mod ids;
pub mod launch;
pub mod limits;
pub mod map;
pub mod message;
pub mod otel;
pub mod pairing;
pub mod plan;
pub mod progress;
pub mod schema;
pub mod session;

pub use github::{ChecksStatus, DecisionBody, DecisionRecord, PullRequestState};
pub use hook::{HookEvent, HookEventKind};
pub use ids::{IssueRef, MapRef, ParseIssueRefError};
pub use launch::{CapHit, CapKind, LaunchCommand, LaunchResult, RefusalReason};
pub use limits::Limits;
pub use map::{MapNode, NodeKind, RepoRef, Seam, SystemMap};
pub use message::{
    CompanionEnvelope, CompanionMessage, PROTOCOL_VERSION, ServerEnvelope, ServerMessage,
};
pub use pairing::{PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse};
pub use plan::{AffectedPart, Assumption, AssumptionScope, ORow, PartStatus, Plan};
pub use progress::{Progress, ProgressInputs, ProgressState, derive_progress};
pub use session::{
    Heartbeat, IssuePin, SessionState, SessionStatus, StaleSession, StateSource, TmuxLocation,
};
