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
    /// One resource changed that the overview does not carry (plan 122): only a view showing it
    /// refetches it. The overview is not refetched.
    Updated {
        at: DateTime<Utc>,
        resource: WebResource,
    },
}

/// A resource a view can refetch on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebResource {
    /// `GET` `session_message::routes::LAUNCH_MESSAGES` for this launch.
    LaunchMessages { launch_id: uuid::Uuid },
}

/// Route path for the browser websocket.
pub mod routes {
    pub const WEB_WS: &str = "/ws/web";
}
