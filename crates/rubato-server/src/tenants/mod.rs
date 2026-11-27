//! Slice `tenants`: who belongs to which tenant, the installations each one owns and the
//! repositories those cover (ADR 0087, 0088). Owns tables `tenants`, `tenant_installations`,
//! `tenant_members` and `tenant_repos`.
//!
//! Every slice that serves a tenant's data will name this one for its scope (design §4.2), so it
//! names nothing but `auth` and the kernel. Work that spans slices (the identity guard, removing
//! a person) lives in `admin`, which may name them all.

mod seed;
mod store;

pub use seed::{DEFAULT_TENANT, Seeded, start};
pub use store::{
    Outcome, Refused, Removed, add_member, attach_installation, create_tenant, installations_of,
    members, note_login, remove_member, role_of, set_role, tenant_exists, tenants_of,
};

#[cfg(test)]
mod tests;
