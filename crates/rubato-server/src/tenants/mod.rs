//! Slice `tenants`: who belongs to which tenant, the installations each one owns and the
//! repositories those cover (ADR 0087, 0088). Owns tables `tenants`, `tenant_installations`,
//! `tenant_members` and `tenant_repos`.
//!
//! Every slice that serves a tenant's data will name this one for its scope (design §4.2), so it
//! names nothing but `auth` and the kernel. Work that spans slices (the identity guard, removing
//! a person) lives in `admin`, which may name them all.

mod scope;
mod seed;
mod store;

pub use scope::{check_tenant, member_logins, scope_for};
pub use seed::{DEFAULT_TENANT, Seeded, start};
pub use store::{
    Outcome, Refused, Removed, Synced, add_member, attach_installation, attached, create_tenant,
    detach_installation, installations_of, members, note_login, remove_member, role_of, set_role,
    set_suspended, sync_repos, tenant_exists, tenants_of,
};

use crate::kernel::state::App;

/// Give the GitHub registry the tenancy rows as they now are (design §5.1): at start, and after
/// every change to an installation or its repositories. With one GitHub context there is
/// nothing to route by.
pub async fn load_routes(app: &App) -> anyhow::Result<()> {
    if !app.github.routes_by_installation() {
        return Ok(());
    }
    let routes = store::live_routes(&app.db).await?;
    tracing::debug!(
        installations = routes.installations.len(),
        repos = routes.repos.len(),
        "tenancy routes loaded"
    );
    app.github.set_routes(routes);
    Ok(())
}

#[cfg(test)]
mod tests;
