//! Workspaces (ADRs 0044, 0049, 0089): a tenant holds one or more, each one map and the view
//! state kept for it. Map-scoped routes are served under `/api/t/{tenant}/w/{ws}`; a workspace id
//! is unique inside its tenant.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The id of the workspace that keeps every row from before workspaces (ADR 0044). No tenant
/// needs one (plan 345, Decision 12).
pub const DEFAULT_WORKSPACE: &str = "default";

/// One workspace as the switcher shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceInfo {
    /// `[a-z0-9-]{1,32}`: in URLs (`?w=`, `/api/t/{tenant}/w/{id}`), rows and the `rubato.workspace` attribute.
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
    /// `GET`: the tenant's workspaces, in switcher order. A tenant route: served under
    /// [`crate::tenant::routes::in_tenant`].
    pub const WORKSPACES: &str = "/api/workspaces";

    /// A map-scoped route for one workspace of one tenant (ADR 0089): `/api/overview` in
    /// workspace `shop` of tenant `acme` is `/api/t/acme/w/shop/overview`. A path outside
    /// `/api/` is returned unchanged.
    pub fn scoped(tenant: &str, workspace: &str, route: &str) -> String {
        match route.strip_prefix("/api/") {
            Some(rest) => format!(
                "{}/{tenant}/w/{workspace}/{rest}",
                crate::tenant::routes::TENANT_PREFIX
            ),
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
            routes::scoped("acme", "shop", "/api/overview"),
            "/api/t/acme/w/shop/overview"
        );
        assert_eq!(
            routes::scoped("acme", "shop", "/api/map/draft/ops"),
            "/api/t/acme/w/shop/map/draft/ops"
        );
        assert_eq!(routes::scoped("acme", "shop", "/ws/web"), "/ws/web");
    }
}
