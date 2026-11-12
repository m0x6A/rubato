//! `rubato-protocol` is the contract between `rubato-companion`, `rubato-server` and `web`.
//!
//! Serde types only: no I/O, no async runtime. Both binaries depend on it and the web
//! frontend consumes the JSON Schema exported from it (`schema/protocol/`). A breaking
//! change here is a seam change (`companion-server`, `web-server`) and blocks.

pub mod auth;
pub mod github;
pub mod hook;
pub mod ids;
pub mod launch;
pub mod layout;
pub mod limits;
pub mod map;
pub mod map_draft;
pub mod message;
pub mod otel;
pub mod pairing;
pub mod pins;
pub mod plan;
pub mod progress;
pub mod queue;
pub mod schema;
pub mod session;
pub mod web_link;

pub use auth::Me;
pub use github::{
    ApprovalBody, ApprovalRecord, ApprovalState, ApprovalVersion, ChecksStatus, DecisionBody,
    DecisionRecord, PullRequestState, ReviewBody, ReviewRecord, trusted_author,
};
pub use hook::{HookEvent, HookEventKind};
pub use ids::{IssueRef, MapRef, ParseIssueRefError};
pub use launch::{
    CapHit, CapKind, LaunchCommand, LaunchRequest, LaunchResponse, LaunchResult, RefusalReason,
};
pub use layout::{MapLayout, Position};
pub use limits::Limits;
pub use map::{Flow, FlowKind, MapNode, MapResponse, NodeKind, RepoRef, Seam, SystemMap};
pub use map_draft::{DraftOp, DraftOpRequest, MapDraft, MapOp, Proposal, ProposalState};
pub use message::{
    CompanionEnvelope, CompanionMessage, PROTOCOL_VERSION, ServerEnvelope, ServerMessage,
};
pub use pairing::{
    PairApproveRequest, PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse,
    PairingInfo,
};
pub use pins::{IssuePin, PinRequest};
pub use plan::{AffectedPart, Assumption, AssumptionScope, ORow, PartStatus, Plan, PlanFlow};
pub use progress::{
    Backlog, BacklogRepo, Finding, IssueSummary, OpenIssue, Overview, Progress, ProgressInputs,
    ProgressState, derive_progress,
};
pub use queue::{QueueOrder, QueueRequest, QueuedIssue};
pub use session::{
    Heartbeat, SessionState, SessionStatus, StaleSession, StateSource, TmuxLocation,
};
pub use web_link::WebEvent;
