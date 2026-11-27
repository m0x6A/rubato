//! Tables `users` and `web_sessions`.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

pub async fn upsert_user(
    db: &SqlitePool,
    login: &str,
    github_id: Option<i64>,
    avatar_url: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO users (login, github_id, avatar_url, created_at) VALUES (?, ?, ?, ?) ON CONFLICT(login) DO UPDATE SET github_id = excluded.github_id, avatar_url = excluded.avatar_url")
        .bind(login).bind(github_id).bind(avatar_url).bind(ts(Utc::now())).execute(db).await?;
    Ok(())
}

/// The GitHub user id stored for `login`, if any. Logins are compared without case, as GitHub
/// does.
pub async fn github_id_of(db: &SqlitePool, login: &str) -> Result<Option<i64>> {
    let id: Option<Option<i64>> =
        sqlx::query_scalar("SELECT github_id FROM users WHERE lower(login) = lower(?)")
            .bind(login)
            .fetch_optional(db)
            .await?;
    Ok(id.flatten())
}

/// End every web session of `login` and drop its user tokens, inside the caller's transaction:
/// the login now belongs to someone else (Decision 15), or the person left their last tenant.
/// Returns how many sessions ended.
pub async fn revoke_person(conn: &mut sqlx::SqliteConnection, login: &str) -> Result<u64> {
    let sessions = sqlx::query("DELETE FROM web_sessions WHERE lower(login) = lower(?)")
        .bind(login)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM user_tokens WHERE lower(login) = lower(?)")
        .bind(login)
        .execute(&mut *conn)
        .await?;
    Ok(sessions)
}

/// A new session. `team_checked_at` is when the login was seen to have access to the App's
/// installation; None when the server is not an App (ADR 0022).
pub async fn create_web_session(
    db: &SqlitePool,
    login: &str,
    team_checked_at: Option<DateTime<Utc>>,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO web_sessions (id, login, created_at, team_checked_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(login)
    .bind(ts(Utc::now()))
    .bind(team_checked_at.map(ts))
    .execute(db)
    .await?;
    Ok(id)
}

/// Whether `login` has a team record that still counts at `now`: some web session of theirs
/// whose access to the App's installation was checked recently enough (ADR 0022). What a
/// credential that never expires, a pairing, is held to before it reads the context graph
/// (ADR 0086).
pub async fn has_fresh_team_record(
    db: &SqlitePool,
    login: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let newest: Option<String> = sqlx::query_scalar(
        "SELECT MAX(team_checked_at) FROM web_sessions WHERE lower(login) = lower(?) AND team_checked_at IS NOT NULL",
    )
    .bind(login)
    .fetch_one(db)
    .await?;
    Ok(super::team::record_is_fresh(
        newest.map(|t| parse_ts(&t)),
        now,
    ))
}

/// A stored web session.
#[derive(Debug, Clone)]
pub struct WebSession {
    pub login: String,
    pub created_at: DateTime<Utc>,
    /// When the login was seen on the team; None when the server is not an App (ADR 0022).
    pub team_checked_at: Option<DateTime<Utc>>,
}

/// A session's login, when it was created, and its team record.
pub async fn web_session(db: &SqlitePool, id: &str) -> Result<Option<WebSession>> {
    Ok(
        sqlx::query("SELECT login, created_at, team_checked_at FROM web_sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(db)
            .await?
            .map(|r| WebSession {
                login: r.get::<String, _>("login"),
                created_at: parse_ts(&r.get::<String, _>("created_at")),
                team_checked_at: r
                    .get::<Option<String>, _>("team_checked_at")
                    .map(|t| parse_ts(&t)),
            }),
    )
}

pub async fn delete_web_session(db: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM web_sessions WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// A person who has logged in, as the tenant seed sees them (design §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub login: String,
    pub github_id: Option<i64>,
    /// Their newest session's team record (or, without the App, the session itself) is at or
    /// after the time asked about.
    pub recent: bool,
}

/// Every user, with whether they were on the team since `since`: a web session whose team
/// record (`team_checked_at`, ADR 0022) or, where there is none, whose creation is that recent.
pub async fn people(db: &SqlitePool, since: DateTime<Utc>) -> Result<Vec<Person>> {
    let rows = sqlx::query(
        "SELECT u.login, u.github_id, MAX(COALESCE(s.team_checked_at, s.created_at)) AS newest \
         FROM users u LEFT JOIN web_sessions s ON lower(s.login) = lower(u.login) \
         GROUP BY u.login ORDER BY lower(u.login)",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .map(|r| Person {
            login: r.get("login"),
            github_id: r.get("github_id"),
            recent: r
                .get::<Option<String>, _>("newest")
                .is_some_and(|t| parse_ts(&t) >= since),
        })
        .collect())
}
