//! The port through which slice `auth` hands a login to the slices that decide who a person is
//! across tenants (ADR 0087, 0088), so `auth` names none of them and stays a leaf.
//!
//! The composition root installs the real adapter (`admin::Admission`) at start; until then,
//! and in tests that do not install it, [`Unwired`] admits nobody new and knows no tenants.

use super::state::App;
use anyhow::Result;
use async_trait::async_trait;
use rubato_protocol::TenantInfo;

/// What `auth`'s login callback and `Me` ask of the tenancy slices.
#[async_trait]
pub trait Admission: Send + Sync {
    /// Called with GitHub's answer for a login, before anything of the login is written. When
    /// the stored GitHub id for `login` differs from `github_id`, the login now belongs to
    /// someone else (a renamed and reused account): the adapter revokes the old person's web
    /// sessions, user tokens, pairings and queue rows in one transaction and answers `true`
    /// (Decision 15). Memberships are keyed by id, so none carries over.
    async fn before_login(&self, app: &App, login: &str, github_id: i64) -> Result<bool>;

    /// Called once a login has its session: memberships learn the current login and when the
    /// person first joined. A failure is logged by the caller, never fatal to the login.
    async fn after_login(&self, app: &App, login: &str, github_id: i64) -> Result<()>;

    /// The tenants `github_id` is a member of, in a stable order, for `Me`.
    async fn tenants_of(&self, app: &App, github_id: i64) -> Result<Vec<TenantInfo>>;
}

/// No tenancy wired: nobody's identity is checked and nobody is in a tenant.
pub struct Unwired;

#[async_trait]
impl Admission for Unwired {
    async fn before_login(&self, _: &App, _: &str, _: i64) -> Result<bool> {
        Ok(false)
    }

    async fn after_login(&self, _: &App, _: &str, _: i64) -> Result<()> {
        Ok(())
    }

    async fn tenants_of(&self, _: &App, _: i64) -> Result<Vec<TenantInfo>> {
        Ok(Vec::new())
    }
}
