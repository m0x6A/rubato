//! An installation's life after it is attached (design §3.3, §5.3, §5.4): its repositories as
//! GitHub lists them, suspension, and deletion. A repository that leaves the tenant is
//! tombstoned, and in the same transaction its room agents leave, its pending call-in requests
//! are withdrawn and its queued issues are dropped, so nothing of it runs on.

use crate::kernel::state::App;
use crate::kernel::tenancy::TenantId;
use crate::{queue, rooms, tenants};
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqliteConnection;

/// What GitHub said about an attached installation (a webhook, ADR 0074).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallationEvent {
    Suspended,
    Unsuspended,
    /// The App was uninstalled from the account: the installation is detached.
    Deleted,
    /// Repositories were added to it or removed from it: list them again.
    Repositories,
}

/// One refresh over every live installation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refreshed {
    pub installations: usize,
    pub failed: usize,
    pub added: usize,
    pub removed: usize,
    pub conflicts: usize,
}

/// Everything of these repositories that would go on running for the tenant they left.
async fn leave(conn: &mut SqliteConnection, repos: &[String], now: DateTime<Utc>) -> Result<()> {
    if repos.is_empty() {
        return Ok(());
    }
    let (agents, requests) = rooms::leave_repos(conn, repos, now).await?;
    let queued = queue::drop_repos(conn, repos).await?;
    tracing::info!(
        repos = repos.len(),
        agents,
        requests,
        queued,
        "repositories left their tenant: agents dismissed, requests withdrawn, queue dropped"
    );
    Ok(())
}

/// Apply one listing of `installation`'s repositories.
async fn apply(
    app: &App,
    tenant: &TenantId,
    installation: u64,
    listed: &[String],
) -> Result<tenants::Synced> {
    let now = app.clock.now();
    let mut tx = app.db.begin().await?;
    let s = tenants::sync_repos(&mut tx, tenant.as_str(), installation, listed, now).await?;
    leave(&mut tx, &s.removed, now).await?;
    tx.commit().await?;
    for repo in &s.conflicts {
        tracing::warn!(
            rubato.github.installation = installation,
            rubato.tenant = %tenant,
            rubato.repo = %repo,
            "repository held by another tenant; not taken"
        );
    }
    if !s.added.is_empty() || !s.removed.is_empty() {
        tracing::info!(
            rubato.github.installation = installation,
            rubato.tenant = %tenant,
            added = s.added.len(),
            removed = s.removed.len(),
            "installation repositories changed"
        );
    }
    Ok(s)
}

/// List one installation's repositories as it and apply the listing.
async fn refresh_one(app: &App, installation: u64) -> Result<Option<tenants::Synced>> {
    let Some(live) = app
        .github
        .live()
        .into_iter()
        .find(|l| l.installation == Some(installation))
    else {
        return Ok(None);
    };
    let listed = live.client.installation_repos().await?;
    Ok(Some(apply(app, &live.tenant, installation, &listed).await?))
}

/// On the full lane (design §5.3): every live installation's repositories, listed as it, become
/// the tenant's. One installation failing ends only its own refresh. A server with one GitHub
/// context has none to list.
pub async fn refresh_installations(app: &App) -> Refreshed {
    let mut out = Refreshed::default();
    if !app.github.routes_by_installation() {
        return out;
    }
    for live in app.github.live() {
        let Some(id) = live.installation else {
            continue;
        };
        out.installations += 1;
        let synced = match live.client.installation_repos().await {
            Ok(listed) => apply(app, &live.tenant, id, &listed).await,
            Err(e) => Err(e),
        };
        match synced {
            Ok(s) => {
                out.added += s.added.len();
                out.removed += s.removed.len();
                out.conflicts += s.conflicts.len();
            }
            Err(e) => {
                out.failed += 1;
                tracing::warn!(
                    rubato.github.installation = id,
                    rubato.tenant = %live.tenant,
                    error = %format!("{e:#}"),
                    "listing the installation's repositories failed"
                );
            }
        }
    }
    if let Err(e) = tenants::load_routes(app).await {
        tracing::error!(error = %format!("{e:#}"), "loading the tenancy routes failed");
    }
    out
}

/// Apply what GitHub said about an attached installation. Nothing happens for one that is not
/// attached: the webhook handler admits none, and one detached since is left as it is.
pub async fn installation_changed(
    app: &App,
    installation: u64,
    event: InstallationEvent,
) -> Result<()> {
    let now = app.clock.now();
    let mut tx = app.db.begin().await?;
    let Some((tenant, _)) = tenants::attached(&mut tx, installation).await? else {
        return Ok(());
    };
    match event {
        InstallationEvent::Suspended | InstallationEvent::Unsuspended => {
            let at = (event == InstallationEvent::Suspended).then_some(now);
            tenants::set_suspended(&mut tx, installation, at).await?;
            tx.commit().await?;
            tracing::warn!(
                rubato.github.installation = installation,
                rubato.tenant = %tenant,
                suspended = at.is_some(),
                "installation suspension changed"
            );
        }
        InstallationEvent::Deleted => {
            let repos = tenants::detach_installation(&mut tx, installation, now).await?;
            leave(&mut tx, &repos, now).await?;
            tx.commit().await?;
            tracing::warn!(
                rubato.github.installation = installation,
                rubato.tenant = %tenant,
                repos = repos.len(),
                "installation deleted on GitHub: detached, its rows kept until purged"
            );
        }
        InstallationEvent::Repositories => tx.commit().await?,
    }
    tenants::load_routes(app).await?;
    if matches!(
        event,
        InstallationEvent::Unsuspended | InstallationEvent::Repositories
    ) {
        refresh_one(app, installation).await?;
        tenants::load_routes(app).await?;
    }
    Ok(())
}
