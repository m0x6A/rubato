//! Who is logged in, on the `web-server` seam: GitHub OAuth routes and `Me`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Me {
    pub login: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// Route paths for login (`/auth/*` is outside `/api`).
pub mod routes {
    pub const ME: &str = "/api/me";
    pub const LOGIN: &str = "/auth/github/login";
    pub const CALLBACK: &str = "/auth/github/callback";
    pub const LOGOUT: &str = "/auth/logout";
}
