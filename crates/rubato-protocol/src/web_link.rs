//! The browser websocket: `WebEvent` fan-out on `/ws/web`.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Pushed to browsers over `/ws/web`. Clients refetch the overview on `Changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebEvent {
    Changed {
        at: DateTime<Utc>,
        reason: String,
        /// The workspace the change belongs to, for changes of one map only (layout, draft, map
        /// load; ADR 0049). Absent: every workspace may have changed, so every client refetches.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
    },
}

/// Route path for the browser websocket.
pub mod routes {
    pub const WEB_WS: &str = "/ws/web";
}
