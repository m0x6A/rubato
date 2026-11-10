//! The browser websocket: `WebEvent` fan-out on `/ws/web`.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Pushed to browsers over `/ws/web`. Clients refetch the overview on `Changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebEvent {
    Changed { at: DateTime<Utc>, reason: String },
}

/// Route path for the browser websocket.
pub mod routes {
    pub const WEB_WS: &str = "/ws/web";
}
