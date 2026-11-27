//! Tenants on the `web-server` seam (ADR 0087, 0088, 0089): who a person is in each tenant,
//! the members an admin manages, and the installations an operator attaches.
//!
//! A tenant owns GitHub App installations, workspaces and invited members. A person may be in
//! several; `Me.tenants` lists them. Ids use the workspace alphabet.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whether `s` is a well-formed tenant id: `[a-z0-9-]{1,32}`, as workspace ids are.
pub fn is_tenant_id(s: &str) -> bool {
    crate::workspace::is_workspace_id(s)
}

/// A member's role in one tenant (ADR 0088). An operator is not a role: it comes from the
/// server's configuration, never from a tenant.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Invites, changes roles, removes members, and creates, edits and deletes workspaces.
    Admin,
    /// Uses the tenant's workspaces.
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "admin" => Some(Role::Admin),
            "member" => Some(Role::Member),
            _ => None,
        }
    }
}

/// One tenant the person is a member of, as `Me` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TenantInfo {
    pub id: String,
    pub name: String,
    pub role: Role,
}

/// One member of a tenant, as the members page lists them. `joined` is false until the invited
/// person has logged in once (Assumption A4: invites do not expire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemberView {
    pub github_id: u64,
    pub login: String,
    pub role: Role,
    pub invited_by: String,
    /// RFC 3339.
    pub invited_at: String,
    pub joined: bool,
}

/// `POST` members: invite a GitHub login. The server resolves it to a user id at invite time;
/// the id is what counts after (ADR 0088).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InviteRequest {
    pub login: String,
    #[serde(default = "member")]
    pub role: Role,
}

fn member() -> Role {
    Role::Member
}

/// `PUT` a member: change their role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoleChange {
    pub role: Role,
}

/// Whether a GitHub account is a person or an organisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AccountType {
    User,
    Organization,
}

impl AccountType {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountType::User => "User",
            AccountType::Organization => "Organization",
        }
    }

    pub fn parse(s: &str) -> Option<AccountType> {
        match s {
            "User" => Some(AccountType::User),
            "Organization" => Some(AccountType::Organization),
            _ => None,
        }
    }
}

/// Where an installation of the App stands, as the operator page shows it (design §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallationState {
    /// Installed on GitHub, attached to no tenant: never read.
    Unattached,
    /// Attached; its repositories are read.
    Attached,
    /// GitHub suspended it; attached but not read.
    Suspended,
    /// Detached or deleted; its rows are kept, hidden, until an operator purges them.
    Detached,
}

/// One installation of the App, on the operator page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdminInstallation {
    pub installation_id: u64,
    pub account_login: String,
    pub account_type: AccountType,
    pub state: InstallationState,
    /// The tenant it is attached to, or was before it was detached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// `POST` on the operator page: a new tenant, its installations and its first admin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewTenant {
    pub id: String,
    pub name: String,
    /// A GitHub login, resolved to a user id at creation.
    pub first_admin: String,
    #[serde(default)]
    pub installations: Vec<u64>,
}

/// A tenant on the operator page: names and counts, never its data (Assumption A5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdminTenant {
    pub id: String,
    pub name: String,
    pub installations: Vec<u64>,
    pub members: u32,
    pub admins: u32,
    pub workspaces: u32,
}

/// Why a tenancy request was refused.
pub mod reason {
    /// A login that is neither an operator nor invited to any tenant (ADR 0088).
    pub const NOT_INVITED: &str = "not_invited";
    /// No such tenant, or the caller is not a member: the same answer for both (404).
    pub const UNKNOWN_TENANT: &str = "unknown_tenant";
    /// A resource or repository of another tenant named inside a tenant path (404).
    pub const OTHER_TENANT: &str = "other_tenant";
    /// A login whose GitHub user id differs from the one stored: a new person (Decision 15).
    pub const IDENTITY_CHANGED: &str = "identity_changed";
    pub const BAD_ID: &str = "bad_id";
    pub const DUPLICATE: &str = "duplicate";
    /// Rows of a deleted tenant with this id are still kept.
    pub const ID_HAS_ROWS: &str = "id_has_rows";
    /// The last admin of a tenant cannot be removed or demoted.
    pub const LAST_ADMIN: &str = "last_admin";
    pub const NOT_A_MEMBER: &str = "not_a_member";
    pub const ALREADY_MEMBER: &str = "already_member";
    /// GitHub has no account with this login.
    pub const UNKNOWN_LOGIN: &str = "unknown_login";
    /// The installation is attached to another tenant.
    pub const INSTALLATION_TAKEN: &str = "installation_taken";
    /// The installation's repositories still have rows of another tenant: purge first.
    pub const ROWS_OF_OTHER_TENANT: &str = "rows_of_other_tenant";
    /// Detach refused while a workspace of the tenant names one of its repositories.
    pub const INSTALLATION_IN_USE: &str = "installation_in_use";
    /// Delete refused while the tenant has workspaces or attached installations.
    pub const TENANT_NOT_EMPTY: &str = "tenant_not_empty";
}

/// Route paths. Tenant routes live under `/api/t/{tenant}` (ADR 0089); operator routes under
/// `/api/admin`.
pub mod routes {
    pub const TENANT_PREFIX: &str = "/api/t";

    /// A tenant route for one tenant (ADR 0089): `/api/queue` in `acme` is `/api/t/acme/queue`.
    /// A path outside `/api/` is returned unchanged.
    pub fn in_tenant(tenant: &str, route: &str) -> String {
        match route.strip_prefix("/api/") {
            Some(rest) => format!("{TENANT_PREFIX}/{tenant}/{rest}"),
            None => route.to_string(),
        }
    }
    /// `GET` lists, `POST` invites.
    pub const MEMBERS: &str = "/api/t/{tenant}/members";
    /// `PUT` changes a role, `DELETE` removes.
    pub const MEMBER: &str = "/api/t/{tenant}/members/{github_id}";
    /// `GET`: the App's installations, attached or not.
    pub const ADMIN_INSTALLATIONS: &str = "/api/admin/installations";
    /// `GET` lists, `POST` creates.
    pub const ADMIN_TENANTS: &str = "/api/admin/tenants";
    /// `DELETE`: an empty tenant.
    pub const ADMIN_TENANT: &str = "/api/admin/tenants/{tenant}";
    /// `POST` attaches, `DELETE` detaches.
    pub const ADMIN_ATTACH: &str = "/api/admin/tenants/{tenant}/installations/{installation_id}";
    /// `POST`: delete a detached installation's rows.
    pub const ADMIN_PURGE: &str = "/api/admin/installations/{installation_id}/purge";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_and_workspace_paths() {
        assert_eq!(routes::in_tenant("acme", "/api/queue"), "/api/t/acme/queue");
        assert_eq!(routes::in_tenant("acme", "/ws/web"), "/ws/web");
        assert_eq!(
            crate::workspace::routes::scoped("acme", "shop", "/api/overview"),
            "/api/t/acme/w/shop/overview"
        );
    }

    #[test]
    fn tenant_ids_use_the_workspace_alphabet() {
        assert!(is_tenant_id("acme"));
        assert!(is_tenant_id("team-2"));
        assert!(!is_tenant_id(""));
        assert!(!is_tenant_id("Acme"));
        assert!(!is_tenant_id("a/b"));
        assert!(!is_tenant_id(&"a".repeat(33)));
    }

    #[test]
    fn roles_round_trip_as_snake_case() {
        for r in [Role::Admin, Role::Member] {
            assert_eq!(Role::parse(r.as_str()), Some(r));
            let json = serde_json::to_string(&r).unwrap();
            assert_eq!(json, format!("\"{}\"", r.as_str()));
        }
        assert_eq!(Role::parse("operator"), None);
    }

    #[test]
    fn an_invite_without_a_role_is_a_member() {
        let r: InviteRequest = serde_json::from_str(r#"{"login":"ada"}"#).unwrap();
        assert_eq!(r.role, Role::Member);
    }

    #[test]
    fn account_types_are_githubs_spelling() {
        assert_eq!(
            AccountType::parse("Organization"),
            Some(AccountType::Organization)
        );
        assert_eq!(
            serde_json::to_string(&AccountType::User).unwrap(),
            "\"User\""
        );
    }
}
