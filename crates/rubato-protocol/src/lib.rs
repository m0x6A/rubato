//! `rubato-protocol` is the contract between `rubato-companion`, `rubato-server` and `web`.
//!
//! Serde types only: no I/O, no async runtime. Both binaries depend on it and the web
//! frontend consumes the JSON Schema exported from it (`schema/protocol/`). A breaking
//! change here is a seam change (`companion-server`, `web-server`) and blocks.

pub mod agent;
pub mod approve;
pub mod auth;
pub mod github;
pub mod handoff;
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
pub mod plan_draft;
pub mod plan_hash;
pub mod progress;
pub mod pulls;
pub mod queue;
pub mod room;
pub mod schema;
pub mod session;
pub mod session_log;
pub mod session_message;
pub mod traffic;
pub mod voice;
pub mod web_link;
pub mod workspace;

pub use agent::{AgentReport, BranchStatus, DraftStatus, PlanStatus};
pub use approve::{
    ApproveRequest, ApproveResponse, ApproveState, DraftApproveRequest, DraftApproveResponse,
};
pub use auth::Me;
pub use github::{
    ApprovalBody, ApprovalRecord, ApprovalSource, ApprovalState, ApprovalVersion, ChecksStatus,
    DecisionBody, DecisionRecord, PullRequestState, ReviewBody, ReviewRecord, comment_counts,
    trusted_author,
};
pub use handoff::{
    HandoffLine, HandoffOutcome, HandoffReport, PlanningMode, SKILL_HANDOFF_VERSION,
    skill_has_handoff,
};
pub use hook::{HookEvent, HookEventKind};
pub use ids::{IssueRef, MapRef, ParseIssueRefError, is_branch_slug};
pub use issue_body::{
    MapPlacement, PinSuggestion, map_line, map_line_in, parse_map_line, parse_map_placement,
};
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
    Flow, FlowKind, MAP_ID_PATTERN, MapNode, MapResponse, NodeKind, RepoRef, Scenario,
    ScenarioStep, Seam, ServiceId, SystemMap, is_map_id,
};
pub use map_draft::{DraftOp, DraftOpRequest, MapDraft, MapOp, Proposal, ProposalState};
pub use message::{
    CompanionEnvelope, CompanionMessage, PROTOCOL_VERSION, ServerEnvelope, ServerMessage,
    capability,
};
pub use pairing::{
    PairApproveRequest, PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse,
    PairingInfo,
};
pub use pins::{IssuePin, PinRequest};
pub use plan::{
    AffectedPart, Assumption, AssumptionScope, DataModel, DataModelKind, FitReason, ModelChange,
    ORow, PartStatus, Plan, PlanApprovalRecord, PlanDecision, PlanFit, PlanFlow, plan_fit,
    seam_groups,
};
pub use plan_draft::{
    DraftBundle, DraftFile, DraftRefusal, DraftRestore, DraftRevision, DraftRevisionInfo,
    DraftSummary, DraftView, ImportRequest, ImportResponse, RestoreSkipped,
};
pub use plan_hash::{
    APPROVAL_KEYS, AdrPin, adr_pins, approval_hash, approval_names, bundle_hash, bundle_paths,
    clear_checkboxes, git_blob_sha, normalise_plan, strip_approval_record,
};
pub use progress::{
    Backlog, BacklogRepo, ConnectedRepo, Finding, IssueSummary, OpenIssue, Overview, Progress,
    ProgressInputs, ProgressState, SpendPoint, blocked_from, derive_progress,
};
pub use pulls::{
    MergeMethod, MergeOffer, MergeRequest, MergeResponse, MergeState, OpenPull, PlanGate,
    PullDetail, PullState, RepoPulls,
};
pub use queue::{QueueOrder, QueueRequest, QueuedIssue};
pub use room::{
    CallInRequest, CallInResponse, DismissAgentRequest, LeftReason, RequestAnswer,
    RequestAnswerBody, RequestState, RoomAgent, RoomAuthorKind, RoomKey, RoomMessage,
    RoomPostRequest, RoomReply, RoomRequest, RoomView, SeamNotice, addressed_handles, agent_handle,
};
pub use session::{
    BlockCause, ClientKind, Heartbeat, SessionState, SessionStatus, StaleSession, StateSource,
    TmuxLocation,
};
pub use session_log::{LaunchLog, LogKind, LogRole, SessionLogEntry};
pub use session_message::{
    LaunchMessages, MessageRefusal, ROOM_TURN_PREAMBLE, RoomTurnMode, SessionMessage,
    SessionMessageKind, SessionMessageOutcome, SessionMessageRequest, SessionMessageResult,
    SessionMessageState, SessionMessageView, room_turn_text,
};
pub use traffic::{
    DiscrepancyKey, DismissRequest, GhostEdge, GhostPart, ObservedTraffic, SeamTraffic,
    SourceStatus, TrafficDismissal, TrafficEdge, TrafficEnd, TrafficReport, TrafficState,
    UndeclaredEdge, derive_traffic,
};
pub use voice::{
    UtteranceQuery, VoiceAvailability, VoiceJoinFailed, VoiceToken, VoiceTokenRequest,
};
pub use web_link::{WebEvent, WebResource};
pub use workspace::{DEFAULT_WORKSPACE, WorkspaceInfo, is_workspace_id};
