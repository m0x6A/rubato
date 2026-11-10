//! Tables `users` and `web_sessions`.

use crate::kernel::db::ts;
use anyhow::Result;
use chrono::Utc;
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

pub async fn create_web_session(db: &SqlitePool, login: &str) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO web_sessions (id, login, created_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(login)
        .bind(ts(Utc::now()))
        .execute(db)
        .await?;
    Ok(id)
}

pub async fn web_session_login(db: &SqlitePool, id: &str) -> Result<Option<String>> {
    Ok(sqlx::query("SELECT login FROM web_sessions WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?
        .map(|r| r.get::<String, _>("login")))
}

pub async fn delete_web_session(db: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM web_sessions WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}
