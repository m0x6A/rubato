//! Tables `tenants`, `tenant_installations`, `tenant_members` and `tenant_repos` (migration
//! 0032). Every write that has a rule (the last admin, an installation in one tenant) checks it
//! inside the caller's transaction, so two admins acting at once cannot both pass it.

use crate::kernel::db::ts;
use crate::kernel::github::{Attached, Routes};
use crate::kernel::tenancy::TenantId;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::tenant::reason;
use rubato_protocol::{MemberView, Role, TenantInfo, is_tenant_id};
use sqlx::{Row, SqliteConnection, SqlitePool};

/// Why a tenancy write was refused: one of `rubato_protocol::tenant::reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused(pub &'static str);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// A write's answer: done, or refused for a reason the caller shows.
pub type Outcome<T> = std::result::Result<T, Refused>;

pub async fn count(conn: &mut SqliteConnection) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM tenants")
        .fetch_one(&mut *conn)
        .await?)
}

/// Rows of the tenancy tables that still name `tenant`, though the tenant row may be gone.
/// Later pull requests add the tenant-owned tables as they gain the column (design §3.3).
async fn id_has_rows(conn: &mut SqliteConnection, tenant: &str) -> Result<bool> {
    for table in ["tenant_installations", "tenant_members", "tenant_repos"] {
        let n: i64 =
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE tenant_id = ?"))
                .bind(tenant)
                .fetch_one(&mut *conn)
                .await?;
        if n > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn create_tenant(
    conn: &mut SqliteConnection,
    id: &str,
    name: &str,
    by: &str,
    now: DateTime<Utc>,
) -> Result<Outcome<()>> {
    if !is_tenant_id(id) {
        return Ok(Err(Refused(reason::BAD_ID)));
    }
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tenants WHERE id = ?")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    if exists > 0 {
        return Ok(Err(Refused(reason::DUPLICATE)));
    }
    if id_has_rows(conn, id).await? {
        return Ok(Err(Refused(reason::ID_HAS_ROWS)));
    }
    sqlx::query("INSERT INTO tenants (id, name, created_by, created_at) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind(name)
        .bind(by)
        .bind(ts(now))
        .execute(&mut *conn)
        .await?;
    Ok(Ok(()))
}

pub async fn tenant_exists(db: &SqlitePool, id: &str) -> Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tenants WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await?;
    Ok(n > 0)
}

/// Attach an installation to `tenant` (ADR 0087, design §3.3). Refused while it is attached to
/// another tenant, or while it was detached from another and that tenant's tombstoned rows of
/// its repositories remain (purge first). Attaching it again to the tenant it was detached from
/// brings its repositories back.
pub async fn attach_installation(
    conn: &mut SqliteConnection,
    tenant: &str,
    installation_id: u64,
    by: &str,
    now: DateTime<Utc>,
) -> Result<Outcome<()>> {
    let id = installation_id as i64;
    let held: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT tenant_id, detached_at FROM tenant_installations WHERE installation_id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    match held {
        Some((t, None)) if t == tenant => return Ok(Ok(())),
        Some((_, None)) => return Ok(Err(Refused(reason::INSTALLATION_TAKEN))),
        Some((t, Some(_))) if t != tenant => {
            let rows: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM tenant_repos WHERE installation_id = ?")
                    .bind(id)
                    .fetch_one(&mut *conn)
                    .await?;
            if rows > 0 {
                return Ok(Err(Refused(reason::ROWS_OF_OTHER_TENANT)));
            }
        }
        _ => {}
    }
    sqlx::query(
        "INSERT INTO tenant_installations (installation_id, tenant_id, attached_by, attached_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT(installation_id) DO UPDATE SET tenant_id = excluded.tenant_id, attached_by = excluded.attached_by, attached_at = excluded.attached_at, detached_at = NULL",
    )
    .bind(id)
    .bind(tenant)
    .bind(by)
    .bind(ts(now))
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE tenant_repos SET detached_at = NULL, tenant_id = ? WHERE installation_id = ?",
    )
    .bind(tenant)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(Ok(()))
}

/// The attached, not detached installations of `tenant`.
pub async fn installations_of(db: &SqlitePool, tenant: &str) -> Result<Vec<u64>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT installation_id FROM tenant_installations WHERE tenant_id = ? AND detached_at IS NULL ORDER BY installation_id",
    )
    .bind(tenant)
    .fetch_all(db)
    .await?;
    Ok(ids.into_iter().map(|i| i as u64).collect())
}

async fn admins(conn: &mut SqliteConnection, tenant: &str) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM tenant_members WHERE tenant_id = ? AND role = 'admin'",
    )
    .bind(tenant)
    .fetch_one(&mut *conn)
    .await?)
}

async fn role_in(
    conn: &mut SqliteConnection,
    tenant: &str,
    github_id: i64,
) -> Result<Option<Role>> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM tenant_members WHERE tenant_id = ? AND github_id = ?")
            .bind(tenant)
            .bind(github_id)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(role.as_deref().and_then(Role::parse))
}

/// Invite a person by GitHub user id (ADR 0088). `login` is for display.
pub async fn add_member(
    conn: &mut SqliteConnection,
    tenant: &str,
    github_id: i64,
    login: &str,
    role: Role,
    by: &str,
    now: DateTime<Utc>,
) -> Result<Outcome<()>> {
    if role_in(conn, tenant, github_id).await?.is_some() {
        return Ok(Err(Refused(reason::ALREADY_MEMBER)));
    }
    sqlx::query(
        "INSERT INTO tenant_members (tenant_id, github_id, login, role, invited_by, invited_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(tenant)
    .bind(github_id)
    .bind(login)
    .bind(role.as_str())
    .bind(by)
    .bind(ts(now))
    .execute(&mut *conn)
    .await?;
    Ok(Ok(()))
}

/// Change a member's role; the last admin cannot be demoted.
pub async fn set_role(
    conn: &mut SqliteConnection,
    tenant: &str,
    github_id: i64,
    role: Role,
) -> Result<Outcome<()>> {
    let Some(now) = role_in(conn, tenant, github_id).await? else {
        return Ok(Err(Refused(reason::NOT_A_MEMBER)));
    };
    if now == Role::Admin && role != Role::Admin && admins(conn, tenant).await? <= 1 {
        return Ok(Err(Refused(reason::LAST_ADMIN)));
    }
    sqlx::query("UPDATE tenant_members SET role = ? WHERE tenant_id = ? AND github_id = ?")
        .bind(role.as_str())
        .bind(tenant)
        .bind(github_id)
        .execute(&mut *conn)
        .await?;
    Ok(Ok(()))
}

/// What a removal left: whether the person is now in no tenant at all, so the caller also ends
/// their sessions and pairings (design §4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removed {
    pub last_membership: bool,
}

/// Remove a member; the last admin cannot be removed.
pub async fn remove_member(
    conn: &mut SqliteConnection,
    tenant: &str,
    github_id: i64,
) -> Result<Outcome<Removed>> {
    let Some(role) = role_in(conn, tenant, github_id).await? else {
        return Ok(Err(Refused(reason::NOT_A_MEMBER)));
    };
    if role == Role::Admin && admins(conn, tenant).await? <= 1 {
        return Ok(Err(Refused(reason::LAST_ADMIN)));
    }
    sqlx::query("DELETE FROM tenant_members WHERE tenant_id = ? AND github_id = ?")
        .bind(tenant)
        .bind(github_id)
        .execute(&mut *conn)
        .await?;
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tenant_members WHERE github_id = ?")
        .bind(github_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(Ok(Removed {
        last_membership: left == 0,
    }))
}

/// The person's role in `tenant`, read on every tenant request (Decision 11).
pub async fn role_of(db: &SqlitePool, tenant: &str, github_id: i64) -> Result<Option<Role>> {
    let mut conn = db.acquire().await?;
    role_in(&mut conn, tenant, github_id).await
}

/// The tenants a person is in, by name then id.
pub async fn tenants_of(db: &SqlitePool, github_id: i64) -> Result<Vec<TenantInfo>> {
    let rows = sqlx::query(
        "SELECT t.id, t.name, m.role FROM tenant_members m JOIN tenants t ON t.id = m.tenant_id \
         WHERE m.github_id = ? ORDER BY t.name, t.id",
    )
    .bind(github_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(TenantInfo {
                id: r.get("id"),
                name: r.get("name"),
                role: Role::parse(&r.get::<String, _>("role"))?,
            })
        })
        .collect())
}

/// A tenant's members: admins first, then by login.
pub async fn members(db: &SqlitePool, tenant: &str) -> Result<Vec<MemberView>> {
    let rows = sqlx::query(
        "SELECT github_id, login, role, invited_by, invited_at, joined_at FROM tenant_members \
         WHERE tenant_id = ? ORDER BY role = 'admin' DESC, lower(login)",
    )
    .bind(tenant)
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(MemberView {
                github_id: u64::try_from(r.get::<i64, _>("github_id")).ok()?,
                login: r.get("login"),
                role: Role::parse(&r.get::<String, _>("role"))?,
                invited_by: r.get("invited_by"),
                invited_at: r.get("invited_at"),
                joined: r.get::<Option<String>, _>("joined_at").is_some(),
            })
        })
        .collect())
}

/// At a login: the person's current login on every membership, and when they first joined.
pub async fn note_login(
    db: &SqlitePool,
    github_id: i64,
    login: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "UPDATE tenant_members SET login = ?, joined_at = COALESCE(joined_at, ?) WHERE github_id = ?",
    )
    .bind(login)
    .bind(ts(now))
    .bind(github_id)
    .execute(db)
    .await?;
    Ok(())
}

/// The live routes (design §5.1): every attached installation (not detached), with whether it
/// is suspended, and every live repository of those.
pub async fn live_routes(db: &SqlitePool) -> Result<Routes> {
    let mut routes = Routes::default();
    let rows = sqlx::query(
        "SELECT installation_id, tenant_id, suspended_at FROM tenant_installations WHERE detached_at IS NULL",
    )
    .fetch_all(db)
    .await?;
    for r in rows {
        let Some(tenant) = TenantId::parse(&r.get::<String, _>("tenant_id")) else {
            continue;
        };
        routes.installations.insert(
            r.get::<i64, _>("installation_id") as u64,
            Attached {
                tenant,
                suspended: r.get::<Option<String>, _>("suspended_at").is_some(),
            },
        );
    }
    let repos: Vec<(String, i64)> = sqlx::query_as(
        "SELECT r.repo, r.installation_id FROM tenant_repos r \
         JOIN tenant_installations i ON i.installation_id = r.installation_id AND i.tenant_id = r.tenant_id \
         WHERE r.detached_at IS NULL AND i.detached_at IS NULL",
    )
    .fetch_all(db)
    .await?;
    routes.repos = repos
        .into_iter()
        .map(|(repo, id)| (repo.to_ascii_lowercase(), id as u64))
        .collect();
    Ok(routes)
}

/// The tenant an installation is attached to, and whether it is suspended; `None` when it is
/// not attached or was detached.
pub async fn attached(
    conn: &mut SqliteConnection,
    installation_id: u64,
) -> Result<Option<(String, bool)>> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT tenant_id, suspended_at FROM tenant_installations WHERE installation_id = ? AND detached_at IS NULL",
    )
    .bind(installation_id as i64)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|(t, s)| (t, s.is_some())))
}

/// What one listing of an installation's repositories changed (design §5.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Synced {
    pub added: Vec<String>,
    /// Live before, absent from the listing now: tombstoned.
    pub removed: Vec<String>,
    /// Named by the listing but held by another tenant, live or tombstoned: left as they are.
    pub conflicts: Vec<String>,
}

/// Make `tenant_repos` follow one listing of `installation`'s repositories, inside the caller's
/// transaction. A repository another tenant holds is not taken (a repository belongs to one
/// tenant, ADR 0087); one this tenant held under an old installation moves to this one, live.
pub async fn sync_repos(
    conn: &mut SqliteConnection,
    tenant: &str,
    installation_id: u64,
    listed: &[String],
    now: DateTime<Utc>,
) -> Result<Synced> {
    let id = installation_id as i64;
    let mut out = Synced::default();
    for repo in listed {
        let held: Option<(String, i64, Option<String>)> = sqlx::query_as(
            "SELECT tenant_id, installation_id, detached_at FROM tenant_repos WHERE repo = ?",
        )
        .bind(repo)
        .fetch_optional(&mut *conn)
        .await?;
        match held {
            Some((t, _, _)) if t != tenant => out.conflicts.push(repo.clone()),
            Some((_, was, detached)) => {
                if was != id || detached.is_some() {
                    out.added.push(repo.clone());
                }
                sqlx::query(
                    "UPDATE tenant_repos SET installation_id = ?, seen_at = ?, detached_at = NULL WHERE repo = ?",
                )
                .bind(id)
                .bind(ts(now))
                .bind(repo)
                .execute(&mut *conn)
                .await?;
            }
            None => {
                sqlx::query(
                    "INSERT INTO tenant_repos (repo, tenant_id, installation_id, seen_at) VALUES (?, ?, ?, ?)",
                )
                .bind(repo)
                .bind(tenant)
                .bind(id)
                .bind(ts(now))
                .execute(&mut *conn)
                .await?;
                out.added.push(repo.clone());
            }
        }
    }
    let live: Vec<String> = sqlx::query_scalar(
        "SELECT repo FROM tenant_repos WHERE installation_id = ? AND detached_at IS NULL ORDER BY repo",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    for repo in live {
        if listed.iter().any(|l| l.eq_ignore_ascii_case(&repo)) {
            continue;
        }
        sqlx::query("UPDATE tenant_repos SET detached_at = ? WHERE repo = ?")
            .bind(ts(now))
            .bind(&repo)
            .execute(&mut *conn)
            .await?;
        out.removed.push(repo);
    }
    Ok(out)
}

/// GitHub suspended (`Some`) or unsuspended (`None`) the installation.
pub async fn set_suspended(
    conn: &mut SqliteConnection,
    installation_id: u64,
    at: Option<DateTime<Utc>>,
) -> Result<bool> {
    let n = sqlx::query(
        "UPDATE tenant_installations SET suspended_at = ? WHERE installation_id = ? AND detached_at IS NULL",
    )
    .bind(at.map(ts))
    .bind(installation_id as i64)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Detach an installation (design §3.3): it and its live repositories are tombstoned, inside
/// the caller's transaction. Their rows stay, hidden, until an operator purges them. Returns the
/// repositories tombstoned now.
pub async fn detach_installation(
    conn: &mut SqliteConnection,
    installation_id: u64,
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let id = installation_id as i64;
    sqlx::query(
        "UPDATE tenant_installations SET detached_at = ? WHERE installation_id = ? AND detached_at IS NULL",
    )
    .bind(ts(now))
    .bind(id)
    .execute(&mut *conn)
    .await?;
    let repos: Vec<String> = sqlx::query_scalar(
        "SELECT repo FROM tenant_repos WHERE installation_id = ? AND detached_at IS NULL ORDER BY repo",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE tenant_repos SET detached_at = ? WHERE installation_id = ? AND detached_at IS NULL",
    )
    .bind(ts(now))
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(repos)
}

/// One row of `tenant_installations`, for the operator page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationRow {
    pub installation_id: u64,
    pub tenant: String,
    pub account_login: Option<String>,
    pub account_type: Option<String>,
    pub suspended: bool,
    pub detached: bool,
}

/// Every installation this server has a row for, attached or detached.
pub async fn installation_rows(db: &SqlitePool) -> Result<Vec<InstallationRow>> {
    let rows = sqlx::query(
        "SELECT installation_id, tenant_id, account_login, account_type, suspended_at, detached_at FROM tenant_installations ORDER BY installation_id",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .map(|r| InstallationRow {
            installation_id: r.get::<i64, _>("installation_id") as u64,
            tenant: r.get("tenant_id"),
            account_login: r.get("account_login"),
            account_type: r.get("account_type"),
            suspended: r.get::<Option<String>, _>("suspended_at").is_some(),
            detached: r.get::<Option<String>, _>("detached_at").is_some(),
        })
        .collect())
}

/// What GitHub says the installation's account is, kept for the operator page.
pub async fn set_account(
    conn: &mut SqliteConnection,
    installation_id: u64,
    login: &str,
    account_type: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE tenant_installations SET account_login = ?, account_type = ? WHERE installation_id = ?",
    )
    .bind(login)
    .bind(account_type)
    .bind(installation_id as i64)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// One tenant as the operator sees it: its name and how many people, never its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantRow {
    pub id: String,
    pub name: String,
    pub members: u32,
    pub admins: u32,
}

pub async fn tenant_rows(db: &SqlitePool) -> Result<Vec<TenantRow>> {
    let rows = sqlx::query(
        "SELECT t.id, t.name,          (SELECT COUNT(*) FROM tenant_members m WHERE m.tenant_id = t.id) AS members,          (SELECT COUNT(*) FROM tenant_members m WHERE m.tenant_id = t.id AND m.role = 'admin') AS admins          FROM tenants t ORDER BY t.id",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .map(|r| TenantRow {
            id: r.get("id"),
            name: r.get("name"),
            members: r.get::<i64, _>("members") as u32,
            admins: r.get::<i64, _>("admins") as u32,
        })
        .collect())
}

/// Delete a tenant (design §3.3): refused while an installation is attached to it. The caller
/// has refused already while it has workspaces. Its members go with it; the tombstones of
/// detached installations stay until they are purged, and keep the id from being reused.
/// Returns the members removed, for the caller to end what was their last membership.
pub async fn delete_tenant(
    conn: &mut SqliteConnection,
    tenant: &str,
) -> Result<Outcome<Vec<(i64, String)>>> {
    let attached: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tenant_installations WHERE tenant_id = ? AND detached_at IS NULL",
    )
    .bind(tenant)
    .fetch_one(&mut *conn)
    .await?;
    if attached > 0 {
        return Ok(Err(Refused(reason::TENANT_NOT_EMPTY)));
    }
    let members: Vec<(i64, String)> =
        sqlx::query_as("SELECT github_id, login FROM tenant_members WHERE tenant_id = ?")
            .bind(tenant)
            .fetch_all(&mut *conn)
            .await?;
    sqlx::query("DELETE FROM tenant_members WHERE tenant_id = ?")
        .bind(tenant)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM tenants WHERE id = ?")
        .bind(tenant)
        .execute(&mut *conn)
        .await?;
    Ok(Ok(members))
}

/// Whether a person is in any tenant at all.
pub async fn in_any_tenant(conn: &mut SqliteConnection, github_id: i64) -> Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tenant_members WHERE github_id = ?")
        .bind(github_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(n > 0)
}

/// A member's login as last seen, for what a removal ends by login.
pub async fn member_login(
    conn: &mut SqliteConnection,
    tenant: &str,
    github_id: i64,
) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar(
            "SELECT login FROM tenant_members WHERE tenant_id = ? AND github_id = ?",
        )
        .bind(tenant)
        .bind(github_id)
        .fetch_optional(&mut *conn)
        .await?,
    )
}

/// The live repositories of an attached installation.
pub async fn live_repos_of(
    conn: &mut SqliteConnection,
    installation_id: u64,
) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT repo FROM tenant_repos WHERE installation_id = ? AND detached_at IS NULL ORDER BY repo",
    )
    .bind(installation_id as i64)
    .fetch_all(&mut *conn)
    .await?)
}

/// A detached installation: its tenant and the repositories whose rows are still kept. `None`
/// for an installation with no row; `Err(NOT_DETACHED)` for one that is attached.
pub async fn detached(
    conn: &mut SqliteConnection,
    installation_id: u64,
) -> Result<Option<Outcome<(String, Vec<String>)>>> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT tenant_id, detached_at FROM tenant_installations WHERE installation_id = ?",
    )
    .bind(installation_id as i64)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((tenant, detached_at)) = row else {
        return Ok(None);
    };
    if detached_at.is_none() {
        return Ok(Some(Err(Refused(reason::NOT_DETACHED))));
    }
    let repos: Vec<String> =
        sqlx::query_scalar("SELECT repo FROM tenant_repos WHERE installation_id = ? ORDER BY repo")
            .bind(installation_id as i64)
            .fetch_all(&mut *conn)
            .await?;
    Ok(Some(Ok((tenant, repos))))
}

/// After a purge deleted the rows: the tombstones themselves.
pub async fn drop_tombstones(conn: &mut SqliteConnection, installation_id: u64) -> Result<()> {
    sqlx::query("DELETE FROM tenant_repos WHERE installation_id = ? AND detached_at IS NOT NULL")
        .bind(installation_id as i64)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "DELETE FROM tenant_installations WHERE installation_id = ? AND detached_at IS NOT NULL",
    )
    .bind(installation_id as i64)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
