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
        /// The tenant the change belongs to (ADR 0087). A socket is opened for one tenant and is
        /// sent only that tenant's events, and the few that concern every tenant (absent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant: Option<String>,
        /// The workspace the change belongs to, for changes of one map only (layout, draft, map
        /// load; ADR 0049). Absent: every workspace may have changed, so every client refetches.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
    },
    /// One resource changed that the overview does not carry (plan 122): only a view showing it
    /// refetches it. The overview is not refetched.
    Updated {
        at: DateTime<Utc>,
        /// The tenant the resource belongs to (ADR 0087): never sent to another tenant's socket.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant: Option<String>,
        resource: WebResource,
    },
}

impl WebEvent {
    /// The tenant an event belongs to; `None` for one that concerns every tenant.
    pub fn tenant(&self) -> Option<&str> {
        match self {
            WebEvent::Changed { tenant, .. } | WebEvent::Updated { tenant, .. } => {
                tenant.as_deref()
            }
        }
    }
}

/// A resource a view can refetch on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebResource {
    /// `GET` `session_message::routes::LAUNCH_MESSAGES` for this launch.
    LaunchMessages { launch_id: uuid::Uuid },
    /// `GET` `session_log::routes::LAUNCH_LOG` for this launch.
    LaunchLog { launch_id: uuid::Uuid },
    /// `GET` `plan_draft::routes::DRAFT` (and its revisions) for this draft.
    Draft { draft_id: uuid::Uuid },
    /// `GET` `room::routes::ROOM` for this plan's room (ADR 0057).
    Room {
        repo: String,
        number: u64,
        branch: String,
    },
}

/// Reasons of a `Changed` event that a reader acts on by name.
pub mod changed {
    /// Who is in the tenant changed (ADR 0088): the members page refetches, and each socket of
    /// the tenant reads its own membership again.
    pub const MEMBERS: &str = "members";
}

/// Route path for the browser websocket.
pub mod routes {
    /// With `?t=<tenant>`: one socket per tenant the page shows (ADR 0089).
    pub const WEB_WS: &str = "/ws/web";
    /// The websocket close code when the person is no longer a member of the socket's tenant.
    pub const CLOSE_NOT_A_MEMBER: u16 = 4403;
}
