//! Workspaces (ADRs 0044, 0049): one server holds one or more, each one map and the view state
//! kept for it. Map-scoped routes are served under `/api/w/{ws}`; unprefixed, they mean the
//! workspace with id `default`.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The id of the workspace that keeps every row from before workspaces, and that unprefixed
/// routes serve (ADR 0044).
pub const DEFAULT_WORKSPACE: &str = "default";

/// One workspace as the switcher shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceInfo {
    /// `[a-z0-9-]{1,32}`: in URLs (`?w=`, `/api/w/{id}`), rows and the `rubato.workspace` attribute.
    pub id: String,
    /// What people call it: the entry's `name`, else the map's.
    pub name: String,
    /// `owner/name` of the repo holding this workspace's map.
    pub map_repo: String,
    /// Git sha of the map as loaded. Absent until it has loaded once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded_at: Option<DateTime<Utc>>,
}

/// Whether `s` is a well-formed workspace id: `[a-z0-9-]{1,32}`.
pub fn is_workspace_id(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub mod routes {
    /// `GET`: the configured workspaces, in switcher order.
    pub const WORKSPACES: &str = "/api/workspaces";
    /// Map-scoped routes are also served under this prefix plus the workspace id.
    pub const SCOPE_PREFIX: &str = "/api/w";

    /// A map-scoped route for one workspace: `/api/overview` in `shop` is
    /// `/api/w/shop/overview`. A path outside `/api/` is returned unchanged.
    pub fn scoped(workspace: &str, route: &str) -> String {
        match route.strip_prefix("/api/") {
            Some(rest) => format!("{SCOPE_PREFIX}/{workspace}/{rest}"),
            None => route.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_ids() {
        for ok in ["default", "shop", "a", "auth-2", &"x".repeat(32)] {
            assert!(is_workspace_id(ok), "{ok}");
        }
        for bad in ["", "Shop", "shop!", "a b", "a/b", "a:b", &"x".repeat(33)] {
            assert!(!is_workspace_id(bad), "{bad}");
        }
    }

    #[test]
    fn scoped_routes() {
        assert_eq!(
            routes::scoped("shop", "/api/overview"),
            "/api/w/shop/overview"
        );
        assert_eq!(
            routes::scoped("shop", "/api/map/draft/ops"),
            "/api/w/shop/map/draft/ops"
        );
        assert_eq!(routes::scoped("shop", "/ws/web"), "/ws/web");
    }
}
