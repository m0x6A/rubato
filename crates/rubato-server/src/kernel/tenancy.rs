//! Who a request or a poll pass acts for (ADR 0087, 0088, 0089; design §4.2).
//!
//! - [`TenantId`]: a well-formed tenant id. Work that acts for no person (the poll, webhooks,
//!   housekeeping) carries one.
//! - [`WsKey`]: one workspace of one tenant. Workspace ids are unique only inside a tenant, so
//!   every map, registry entry and workspace row is keyed by this.
//! - [`TenantScope`]: a person acting in a tenant they are a member of. Only slice `tenants`
//!   makes one, after reading `tenant_members`: its request layer (`tenants::check_tenant`) on
//!   every `/api/t/{tenant}` route, and `tenants::scope_for` outside a request. A test pins
//!   that no other file calls [`TenantScope::granted`].
//! - [`AdminScope`]: a `TenantScope` whose role is admin.

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use rubato_protocol::tenant::reason;
use rubato_protocol::{Role, is_tenant_id};

/// The tenant the seed makes from a server as it was before tenants (design §8). A server
/// reading with a personal token or local repositories has one GitHub context and so one
/// tenant: this one.
pub const DEFAULT_TENANT: &str = "default";

/// A well-formed tenant id. Holding one says nothing about who may act for it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TenantId(String);

impl TenantId {
    /// `None` unless `s` is in the tenant alphabet (`[a-z0-9-]{1,32}`).
    pub fn parse(s: &str) -> Option<TenantId> {
        is_tenant_id(s).then(|| TenantId(s.to_owned()))
    }

    /// A tenant id read back from a row (`tenants.id`, or a `tenant_id` column): it was checked
    /// when it was written.
    pub fn stored(s: String) -> TenantId {
        TenantId(s)
    }

    pub fn default_tenant() -> TenantId {
        TenantId(DEFAULT_TENANT.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One workspace of one tenant (ADR 0089).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WsKey {
    pub tenant: TenantId,
    pub ws: String,
}

impl WsKey {
    pub fn new(tenant: TenantId, ws: impl Into<String>) -> WsKey {
        WsKey {
            tenant,
            ws: ws.into(),
        }
    }

    /// A key read back from a row's `tenant_id` and `workspace_id`.
    pub fn stored(tenant: String, ws: String) -> WsKey {
        WsKey::new(TenantId::stored(tenant), ws)
    }

    /// A workspace of tenant `default`: the seed's, and every one of a server with one GitHub
    /// context.
    pub fn of_default(ws: impl Into<String>) -> WsKey {
        WsKey::new(TenantId::default_tenant(), ws)
    }

    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    /// The workspace's id inside its tenant.
    pub fn ws(&self) -> &str {
        &self.ws
    }
}

impl std::fmt::Display for WsKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.tenant, self.ws)
    }
}

/// A person acting in a tenant they are a member of, as their membership stood when the request
/// (or the intake) began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantScope {
    tenant: TenantId,
    github_id: i64,
    login: String,
    role: Role,
}

impl TenantScope {
    /// Only slice `tenants` calls this, after reading the membership (design §4.2).
    pub fn granted(tenant: TenantId, github_id: i64, login: String, role: Role) -> TenantScope {
        TenantScope {
            tenant,
            github_id,
            login,
            role,
        }
    }

    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    pub fn github_id(&self) -> i64 {
        self.github_id
    }

    pub fn login(&self) -> &str {
        &self.login
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    /// One of this tenant's workspaces, by id. Whether it exists is the registry's to say.
    pub fn ws(&self, ws: impl Into<String>) -> WsKey {
        WsKey::new(self.tenant.clone(), ws)
    }
}

/// The 404 for a tenant that does not exist or that the person is not in: one answer for both,
/// so a tenant's existence does not leak (ADR 0089).
pub fn unknown_tenant() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"reason":"{}"}}"#, reason::UNKNOWN_TENANT),
    )
        .into_response()
}

/// The 404 for a resource or repository of another tenant named inside a tenant path.
pub fn other_tenant() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"reason":"{}"}}"#, reason::OTHER_TENANT),
    )
        .into_response()
}

impl<S: Send + Sync> FromRequestParts<S> for TenantScope {
    type Rejection = Response;

    /// The scope `tenants::check_tenant` put on the request. A route mounted without that
    /// layer has none and answers 404: never a default tenant.
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<TenantScope>()
            .cloned()
            .ok_or_else(unknown_tenant)
    }
}

/// A member whose role is admin (ADR 0088): invites, roles, removals, and creating, editing and
/// deleting workspaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminScope(pub TenantScope);

/// The body of the 403 for a member who is not an admin of the tenant.
pub const NOT_ADMIN: &str = r#"{"reason":"not_admin"}"#;

impl<S: Send + Sync> FromRequestParts<S> for AdminScope {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, s: &S) -> Result<Self, Self::Rejection> {
        let scope = TenantScope::from_request_parts(parts, s).await?;
        if !scope.is_admin() {
            return Err((
                StatusCode::FORBIDDEN,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                NOT_ADMIN,
            )
                .into_response());
        }
        Ok(AdminScope(scope))
    }
}

/// For a slice's handler tests, which mount its routes without the composition root's layers:
/// every request acts as an admin of tenant `default` in workspace `default`. A
/// `check_workspace` layer inside it still decides the workspace from the path.
#[cfg(test)]
pub async fn test_scope(mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let login = req
        .headers()
        .get("x-rubato-dev-user")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("m0x6A")
        .to_owned();
    req.extensions_mut().insert(TenantScope::granted(
        TenantId::default_tenant(),
        0,
        login,
        Role::Admin,
    ));
    req.extensions_mut()
        .insert(crate::kernel::http::Ws(WsKey::of_default(
            rubato_protocol::DEFAULT_WORKSPACE,
        )));
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_key_names_its_tenant() {
        let k = WsKey::new(TenantId::parse("acme").unwrap(), "shop");
        assert_eq!((k.tenant().as_str(), k.ws()), ("acme", "shop"));
        assert_eq!(k.to_string(), "acme/shop");
        assert_ne!(
            k,
            WsKey::of_default("shop"),
            "the same id in another tenant"
        );
    }

    /// Design §4.2: a scope is made in slice `tenants` only.
    #[test]
    fn only_the_tenants_slice_grants_a_scope() {
        fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    let rel = p
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    let text = std::fs::read_to_string(&p).unwrap();
                    let n = text.matches("TenantScope::granted(").count();
                    if n > 0 && rel != "kernel/tenancy.rs" {
                        out.push(rel);
                    }
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut callers = Vec::new();
        walk(&root, &root, &mut callers);
        callers.sort();
        let outside: Vec<&String> = callers.iter().filter(|f| !granted_here(f)).collect();
        assert!(
            outside.is_empty(),
            "TenantScope::granted outside slice tenants: {outside:?}"
        );
        assert!(!callers.is_empty(), "the tenants slice grants scopes");
        assert!(
            !granted_here("pins/handlers.rs"),
            "must fail: any other slice is reported"
        );
        println!("checked {} files that grant a scope", callers.len());
    }

    fn granted_here(file: &str) -> bool {
        file.starts_with("tenants/")
    }

    #[test]
    fn only_the_tenant_alphabet_parses() {
        assert_eq!(TenantId::parse("acme").unwrap().as_str(), "acme");
        assert!(TenantId::parse("Acme").is_none());
        assert!(TenantId::parse("").is_none());
        assert_eq!(TenantId::default_tenant().as_str(), DEFAULT_TENANT);
    }
}
