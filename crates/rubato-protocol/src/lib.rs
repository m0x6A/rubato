//! `rubato-protocol` is the contract between `rubato-companion`, `rubato-server` and `web`.
//!
//! Serde types only: no I/O, no async runtime. Both binaries depend on it and the web
//! frontend consumes the JSON Schema exported from it (`schema/protocol/`). A breaking
//! change here is a seam change (`companion-server`, `web-server`) and blocks.

pub mod approve;
pub mod auth;
pub mod github;
pub mod hook;
pub mod ids;
pub mod issue_body;
pub mod issue_new;
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
pub mod pulls;
pub mod queue;
pub mod schema;
pub mod session;
pub mod traffic;
pub mod web_link;

pub use approve::{ApproveRequest, ApproveResponse, ApproveState};
pub use auth::Me;
pub use github::{
    ApprovalBody, ApprovalRecord, ApprovalState, ApprovalVersion, ChecksStatus, DecisionBody,
    DecisionRecord, PullRequestState, ReviewBody, ReviewRecord, trusted_author,
};
pub use hook::{HookEvent, HookEventKind};
pub use ids::{IssueRef, MapRef, ParseIssueRefError};
pub use issue_body::{PinSuggestion, map_line, parse_map_line};
pub use issue_new::{
    CreationState, NewIssueRequest, NewIssueResponse, parse_request_line, render_issue,
    request_line,
};
pub use launch::{
    CapHit, CapKind, LaunchCommand, LaunchRequest, LaunchResponse, LaunchResult, RefusalReason,
};
pub use layout::{MapLayout, Position};
pub use limits::Limits;
pub use map::{
    Flow, FlowKind, MapNode, MapResponse, NodeKind, RepoRef, Scenario, ScenarioStep, Seam,
    SystemMap,
};
pub use map_draft::{DraftOp, DraftOpRequest, MapDraft, MapOp, Proposal, ProposalState};
pub use message::{
    CompanionEnvelope, CompanionMessage, PROTOCOL_VERSION, ServerEnvelope, ServerMessage,
};
pub use pairing::{
    PairApproveRequest, PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse,
    PairingInfo,
};
pub use pins::{IssuePin, PinRequest};
pub use plan::{
    AffectedPart, Assumption, AssumptionScope, DataModel, DataModelKind, FitReason, ModelChange,
    ORow, PartStatus, Plan, PlanFit, PlanFlow, plan_fit,
};
pub use progress::{
    Backlog, BacklogRepo, ConnectedRepo, Finding, IssueSummary, OpenIssue, Overview, Progress,
    ProgressInputs, ProgressState, derive_progress,
};
pub use pulls::{OpenPull, RepoPulls};
pub use queue::{QueueOrder, QueueRequest, QueuedIssue};
pub use session::{
    Heartbeat, SessionState, SessionStatus, StaleSession, StateSource, TmuxLocation,
};
pub use traffic::{
    ObservedTraffic, SeamTraffic, SourceStatus, TrafficEdge, TrafficReport, TrafficState,
    UndeclaredEdge, derive_traffic,
};
pub use web_link::WebEvent;
