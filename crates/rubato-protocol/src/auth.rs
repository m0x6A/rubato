//! Who is logged in, on the `web-server` seam: GitHub OAuth routes and `Me`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Me {
    pub login: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Rubato can post as this person, so *Accept plan* works (ADR 0030).
    #[serde(default)]
    pub can_act_on_github: bool,
    /// Why not, when it cannot: `no_token_key`, `oauth_only`, `tokens_do_not_expire` or
    /// `log_in_again`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cannot_act_reason: Option<String>,
}

/// Route paths for login (`/auth/*` is outside `/api`).
pub mod routes {
    pub const ME: &str = "/api/me";
    pub const LOGIN: &str = "/auth/github/login";
    pub const CALLBACK: &str = "/auth/github/callback";
    pub const LOGOUT: &str = "/auth/logout";
}
