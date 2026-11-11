//! The companion queue (ADR 0020): issues a developer intends to run on one of their own
//! companions, in order. Stored intent, visible to everyone, written only by the companion's
//! paired user. Never a progress state and never an input to `derive_progress`: once a session
//! exists, the session is the truth.

use crate::ids::IssueRef;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A lane holds at most this many queued issues; a write past it is refused.
pub const QUEUE_MAX_PER_COMPANION: usize = 50;

/// One issue waiting on one companion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueuedIssue {
    pub issue: IssueRef,
    pub companion_id: Uuid,
    /// GitHub login of who queued it: the companion's own user.
    pub queued_by: String,
    pub queued_at: DateTime<Utc>,
    /// Place in the lane, from 0.
    pub position: u32,
    /// Why the latest start from the queue did not start, as the companion reported it. Absent
    /// until a start is refused or fails; the entry stays queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_start_error: Option<String>,
}

/// Body of `POST` (add) and `DELETE` (remove) on the queue route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueueRequest {
    pub issue: IssueRef,
    pub companion_id: Uuid,
}

/// Body of `PUT`: the full order of one lane. Refused when its set of issues differs from what
/// is stored, so a reorder never drops an entry added elsewhere meanwhile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueueOrder {
    pub companion_id: Uuid,
    pub issues: Vec<IssueRef>,
}

/// Route path for the queue.
pub mod routes {
    pub const QUEUE: &str = "/api/queue";
}
