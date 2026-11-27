//! Tables `tenants`, `tenant_installations`, `tenant_members` and `tenant_repos` (migration
//! 0032). Every write that has a rule (the last admin, an installation in one tenant) checks it
//! inside the caller's transaction, so two admins acting at once cannot both pass it.

use crate::kernel::db::ts;
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
