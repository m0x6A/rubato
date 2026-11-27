//! The first start with tenants (design §8): while `tenants` is empty, make tenant `default`
//! from what the server already is. Once it holds a row the database owns tenancy, and
//! `RUBATO_GITHUB_APP_INSTALLATION_ID` is only warned about.

use super::store;
use crate::kernel::state::App;
use anyhow::Result;
use chrono::Duration;
use rubato_protocol::Role;

/// The tenant every row from before tenants belongs to (Assumption A2). The operator may
/// rename it; its id stays.
pub const DEFAULT_TENANT: &str = "default";

/// How far back a login still counts as on the team: a session's whole life (ADR 0064).
const RECENT_DAYS: i64 = 30;

/// What the start did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Seeded {
    /// The tenant made, when this start made one.
    pub tenant: Option<String>,
    pub installation: Option<u64>,
    pub members: usize,
    pub admins: usize,
    /// Users the seed did not make members: no session in the last 30 days, or no GitHub id
    /// stored. The operator page lists them to invite again.
    pub left_out: Vec<String>,
}

pub async fn start(app: &App) -> Result<Seeded> {
    let seeded = seed(app).await?;
    if seeded.tenant.is_none()
        && let Some(gh) = &app.config.github_app
    {
        tracing::warn!(
            installation_id = gh.installation_id,
            "RUBATO_GITHUB_APP_INSTALLATION_ID is still set; tenants hold installations now and the database owns them (ADR 0087)"
        );
    }
    Ok(seeded)
}

async fn seed(app: &App) -> Result<Seeded> {
    let now = app.clock.now();
    let people = crate::auth::people(&app.db, now - Duration::days(RECENT_DAYS)).await?;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    if store::count(&mut tx).await? > 0 {
        return Ok(Seeded::default());
    }
    let created = store::create_tenant(&mut tx, DEFAULT_TENANT, "Default", "seed", now).await?;
    anyhow::ensure!(created.is_ok(), "seeding tenant default: {:?}", created);
    let installation = app.config.github_app.as_ref().map(|a| a.installation_id);
    if let Some(id) = installation {
        let attached = store::attach_installation(&mut tx, DEFAULT_TENANT, id, "seed", now).await?;
        anyhow::ensure!(attached.is_ok(), "seeding installation {id}: {attached:?}");
    }
    let mut out = Seeded {
        tenant: Some(DEFAULT_TENANT.into()),
        installation,
        ..Seeded::default()
    };
    for p in &people {
        let operator = app.config.is_operator(p.github_id);
        let Some(id) = p.github_id.filter(|_| p.recent || operator) else {
            out.left_out.push(p.login.clone());
            continue;
        };
        let role = if operator { Role::Admin } else { Role::Member };
        store::add_member(&mut tx, DEFAULT_TENANT, id, &p.login, role, "seed", now)
            .await?
            .map_err(|r| anyhow::anyhow!("seeding member {}: {r}", p.login))?;
        out.members += 1;
        if role == Role::Admin {
            out.admins += 1;
        }
    }
    tx.commit().await?;
    for login in &out.left_out {
        tracing::warn!(login = %login, "seed left out login: no session in the last 30 days or no GitHub id; invite them again");
    }
    tracing::info!(
        tenant = DEFAULT_TENANT,
        installation = ?out.installation,
        members = out.members,
        admins = out.admins,
        left_out = out.left_out.len(),
        "tenant seeded from the server as it was (ADR 0087)"
    );
    if out.admins == 0 {
        tracing::warn!(
            "tenant default has no admin: no operator in RUBATO_OPERATORS has logged in yet; an operator names one on the operator page"
        );
    }
    Ok(out)
}
