//! The pin that ties an issue to the map, and the request that sets it.

use crate::ids::{IssueRef, MapRef};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An issue pinned to a node or seam. The one piece of state GitHub does not hold (ADR 0008).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IssuePin {
    pub issue: IssueRef,
    pub target: MapRef,
    /// GitHub login of who pinned it.
    pub pinned_by: String,
    pub pinned_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PinRequest {
    pub issue: IssueRef,
    pub target: MapRef,
}

/// Route path for pins.
pub mod routes {
    pub const PINS: &str = "/api/pins";
}
