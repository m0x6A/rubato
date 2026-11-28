//! The admission adapter (`kernel::admission`): what a login means across tenants. It may name
//! `auth`, `pairing`, `queue` and `tenants`, which `auth` itself may not.

use crate::kernel::state::App;
use anyhow::Result;
use async_trait::async_trait;
use rubato_protocol::TenantInfo;

pub struct Admission;

#[async_trait]
impl crate::kernel::admission::Admission for Admission {
    async fn before_login(&self, app: &App, login: &str, github_id: i64) -> Result<bool> {
        identity_guard(app, login, github_id).await
    }

    async fn after_login(&self, app: &App, login: &str, github_id: i64) -> Result<()> {
        crate::tenants::note_login(&app.db, github_id, login, app.clock.now()).await
    }

    async fn tenants_of(&self, app: &App, github_id: i64) -> Result<Vec<TenantInfo>> {
        crate::tenants::tenants_of(&app.db, github_id).await
    }

    async fn is_member(
        &self,
        app: &App,
        tenant: &crate::kernel::tenancy::TenantId,
        login: &str,
    ) -> Result<bool> {
        Ok(crate::tenants::scope_for(app, tenant, login)
            .await?
            .is_some())
    }

    async fn member_of(
        &self,
        app: &App,
        login: &str,
    ) -> Result<Vec<crate::kernel::tenancy::TenantId>> {
        use crate::kernel::tenancy::{DEFAULT_TENANT, TenantId};
        let mut out: Vec<TenantId> = match crate::auth::github_id_of(&app.db, login).await? {
            Some(id) => crate::tenants::tenants_of(&app.db, id)
                .await?
                .into_iter()
                .map(|t| TenantId::stored(t.id))
                .collect(),
            None => Vec::new(),
        };
        // Local development: every dev login is in tenant `default`, as `scope_for` grants it.
        if app.config.dev_user.is_some() && !out.iter().any(|t| t.as_str() == DEFAULT_TENANT) {
            out.push(TenantId::default_tenant());
        }
        Ok(out)
    }
}

/// Decision 15: a login stored with one GitHub id and now answered with another belongs to
/// someone else (a renamed account whose login was taken). Before the new person gets anything,
/// in one transaction: the old person's web sessions and user tokens end, their pairings are
/// revoked and their companions' queues dropped. Memberships are keyed by id, so the new person
/// inherits none; the old person keeps theirs under their id. A stored id of NULL (a user from
/// before ADR 0022) is not a change.
pub async fn identity_guard(app: &App, login: &str, github_id: i64) -> Result<bool> {
    let Some(stored) = crate::auth::github_id_of(&app.db, login).await? else {
        return Ok(false);
    };
    if stored == github_id {
        return Ok(false);
    }
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let sessions = crate::auth::revoke_person(&mut tx, login).await?;
    let companions = crate::pairing::revoke_all_of(&mut tx, login).await?;
    let queued = crate::queue::drop_companions(&mut tx, &companions).await?;
    tx.commit().await?;
    tracing::warn!(
        login,
        old_github_id = stored,
        new_github_id = github_id,
        sessions,
        pairings = companions.len(),
        queued,
        "identity changed login={login}: revoked what the old person held"
    );
    Ok(true)
}
