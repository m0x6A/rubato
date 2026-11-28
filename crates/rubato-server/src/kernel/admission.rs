//! The port through which slice `auth` hands a login to the slices that decide who a person is
//! across tenants (ADR 0087, 0088), so `auth` names none of them and stays a leaf.
//!
//! The composition root installs the real adapter (`admin::Admission`) at start; until then,
//! and in tests that do not install it, [`Unwired`] admits nobody new and knows no tenants.

use super::state::App;
use crate::kernel::tenancy::TenantId;
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

    /// Whether `login` is a member of `tenant` now, for work that acts in a tenant for a person
    /// without a request of theirs: what a companion sends for a launch, and what is pushed to
    /// one (design §6.3).
    async fn is_member(&self, app: &App, tenant: &TenantId, login: &str) -> Result<bool>;

    /// The tenants `login` is a member of now, for what follows a person without a request of
    /// theirs: the context graph sources their companions are sent (design §6.4).
    async fn member_of(&self, app: &App, login: &str) -> Result<Vec<TenantId>>;
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

    /// No tenancy wired: no membership is read, so none is found missing.
    async fn is_member(&self, _: &App, _: &TenantId, _: &str) -> Result<bool> {
        Ok(true)
    }

    /// No tenancy wired: every tenant the server reads for.
    async fn member_of(&self, app: &App, _: &str) -> Result<Vec<TenantId>> {
        Ok(app.tenants())
    }
}
