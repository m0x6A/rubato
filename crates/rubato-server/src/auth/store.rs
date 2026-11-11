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

/// A session's login and its team record.
pub async fn web_session(
    db: &SqlitePool,
    id: &str,
) -> Result<Option<(String, Option<DateTime<Utc>>)>> {
    Ok(
        sqlx::query("SELECT login, team_checked_at FROM web_sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(db)
            .await?
            .map(|r| {
                (
                    r.get::<String, _>("login"),
                    r.get::<Option<String>, _>("team_checked_at")
                        .map(|t| parse_ts(&t)),
                )
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
