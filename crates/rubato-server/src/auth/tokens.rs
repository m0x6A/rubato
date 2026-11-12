//! Table `user_tokens` (ADR 0030): a person's GitHub App user-to-server tokens, so Rubato can
//! post as them. Encrypted at rest; refreshed single-flight per login because App refresh
//! tokens rotate; never sent anywhere but GitHub.

use crate::kernel::config::TokenKey;
use crate::kernel::db::{parse_ts, ts};
use crate::kernel::state::App;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Duration, Utc};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// A token this close to expiry is refreshed before use.
const REFRESH_MARGIN_SECS: i64 = 300;
/// The token endpoint's timeout (the login callback had none).
pub const TOKEN_ENDPOINT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Why Rubato cannot act as a person: `Me.cannot_act_reason`.
pub mod reason {
    pub const NO_TOKEN_KEY: &str = "no_token_key";
    pub const OAUTH_ONLY: &str = "oauth_only";
    pub const TOKENS_DO_NOT_EXPIRE: &str = "tokens_do_not_expire";
    pub const LOG_IN_AGAIN: &str = "log_in_again";
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// Nothing stored for this login, or the server keeps no tokens.
    #[error("no token for this person")]
    NotStored,
    /// GitHub rejected the refresh token; the row was dropped.
    #[error("GitHub rejected the refresh token")]
    Rejected,
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

/// What a login answered with, as the token endpoint says it.
pub struct Granted {
    pub access: String,
    pub expires_in: Option<Duration>,
    pub refresh: Option<String>,
    pub refresh_expires_in: Option<Duration>,
}

/// Why this server keeps no tokens at all, or None when it does.
pub fn server_cannot_act(app: &App) -> Option<&'static str> {
    if app.config.github_app.is_none() {
        Some(reason::OAUTH_ONLY)
    } else if app.config.token_key.is_none() {
        Some(reason::NO_TOKEN_KEY)
    } else {
        None
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(text.len().is_multiple_of(2), "odd hex");
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).context("hex"))
        .collect()
}

pub fn seal(key: &TokenKey, plain: &str) -> Result<String> {
    let cipher = Aes256Gcm::new(&key.0.into());
    let nonce: [u8; 12] = rand::random();
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .map_err(|_| anyhow!("encrypting a token failed"))?;
    Ok(hex(&[nonce.as_slice(), &ct].concat()))
}

pub fn open(key: &TokenKey, sealed: &str) -> Result<String> {
    let bytes = unhex(sealed)?;
    anyhow::ensure!(bytes.len() > 12, "sealed token too short");
    let cipher = Aes256Gcm::new(&key.0.into());
    let plain = cipher
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .map_err(|_| anyhow!("a stored token does not open with RUBATO_TOKEN_KEY"))?;
    Ok(String::from_utf8(plain)?)
}

/// Keep what a login granted, when this server keeps tokens and the App issues expiring ones.
/// Otherwise drop any old row. Returns why nothing was kept, if it was not.
pub async fn keep(app: &App, login: &str, g: &Granted) -> Result<Option<&'static str>> {
    if let Some(why) = server_cannot_act(app) {
        delete(&app.db, login).await?;
        return Ok(Some(why));
    }
    let (Some(expires_in), Some(refresh)) = (g.expires_in, g.refresh.as_deref()) else {
        delete(&app.db, login).await?;
        tracing::warn!(
            login,
            "the GitHub App issues user tokens that do not expire; none kept"
        );
        return Ok(Some(reason::TOKENS_DO_NOT_EXPIRE));
    };
    let key = app.config.token_key.as_ref().expect("checked above");
    let now = Utc::now();
    sqlx::query("INSERT INTO user_tokens (login, access_ct, refresh_ct, expires_at, refresh_expires_at, updated_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(login) DO UPDATE SET access_ct = excluded.access_ct, refresh_ct = excluded.refresh_ct, expires_at = excluded.expires_at, refresh_expires_at = excluded.refresh_expires_at, updated_at = excluded.updated_at")
        .bind(login)
        .bind(seal(key, &g.access)?)
        .bind(seal(key, refresh)?)
        .bind(ts(now + expires_in))
        .bind(g.refresh_expires_in.map(|d| ts(now + d)))
        .bind(ts(now))
        .execute(&app.db)
        .await?;
    Ok(None)
}

pub async fn delete(db: &SqlitePool, login: &str) -> Result<()> {
    sqlx::query("DELETE FROM user_tokens WHERE login = ?")
        .bind(login)
        .execute(db)
        .await?;
    Ok(())
}

struct Row_ {
    access: String,
    refresh: String,
    expires_at: DateTime<Utc>,
    refresh_expires_at: Option<DateTime<Utc>>,
}

async fn load(app: &App, login: &str) -> Result<Option<Row_>> {
    let Some(key) = &app.config.token_key else {
        return Ok(None);
    };
    let Some(r) = sqlx::query("SELECT * FROM user_tokens WHERE login = ?")
        .bind(login)
        .fetch_optional(&app.db)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(Row_ {
        access: open(key, &r.get::<String, _>("access_ct"))?,
        refresh: open(key, &r.get::<String, _>("refresh_ct"))?,
        expires_at: parse_ts(&r.get::<String, _>("expires_at")),
        refresh_expires_at: r
            .get::<Option<String>, _>("refresh_expires_at")
            .map(|t| parse_ts(&t)),
    }))
}

/// Whether Rubato can act as this person now, or why not.
pub async fn can_act(app: &App, login: &str) -> Result<(), &'static str> {
    if let Some(why) = server_cannot_act(app) {
        return Err(why);
    }
    match load(app, login).await {
        Ok(Some(r)) if r.refresh_expires_at.is_none_or(|t| t > Utc::now()) => Ok(()),
        _ => Err(reason::LOG_IN_AGAIN),
    }
}

/// One refresh at a time per login: App refresh tokens rotate, so a second concurrent
/// refresh would spend a token the first already replaced.
fn refresh_lock(login: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(login.to_owned())
        .or_default()
        .clone()
}

/// A usable access token for the person, refreshed when it is within five minutes of expiry.
pub async fn access_token(app: &App, login: &str) -> Result<String, TokenError> {
    let fresh = |r: &Row_| r.expires_at - Utc::now() > Duration::seconds(REFRESH_MARGIN_SECS);
    let row = load(app, login).await?.ok_or(TokenError::NotStored)?;
    if fresh(&row) {
        return Ok(row.access);
    }
    let lock = refresh_lock(login);
    let _held = lock.lock().await;
    // Another task may have refreshed while this one waited.
    let row = load(app, login).await?.ok_or(TokenError::NotStored)?;
    if fresh(&row) {
        return Ok(row.access);
    }
    match refresh(app, &row.refresh).await {
        Ok(g) => {
            keep(app, login, &g).await?;
            Ok(g.access)
        }
        Err(Refresh::Rejected) => {
            // Compare-and-delete: only the row that still holds the refresh token that was
            // sent goes, never one a concurrent winner wrote.
            if load(app, login)
                .await?
                .is_some_and(|now| now.refresh == row.refresh)
            {
                delete(&app.db, login).await?;
            }
            tracing::warn!(
                login,
                "token refresh failed: GitHub rejected the refresh token"
            );
            Err(TokenError::Rejected)
        }
        Err(Refresh::Failed(e)) => {
            tracing::warn!(login, error = %format!("{e:#}"), "token refresh failed");
            Err(TokenError::Failed(e))
        }
    }
}

enum Refresh {
    Rejected,
    Failed(anyhow::Error),
}

async fn refresh(app: &App, refresh_token: &str) -> Result<Granted, Refresh> {
    let (Some(id), Some(secret)) = (
        app.config.github_client_id.as_deref(),
        app.config.github_client_secret.as_deref(),
    ) else {
        return Err(Refresh::Failed(anyhow!("GitHub OAuth is not configured")));
    };
    let http = reqwest::Client::builder()
        .timeout(TOKEN_ENDPOINT_TIMEOUT)
        .build()
        .map_err(|e| Refresh::Failed(e.into()))?;
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", id)
        .append_pair("client_secret", secret)
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", refresh_token)
        .finish();
    let answer: serde_json::Value = http
        .post(format!(
            "{}/login/oauth/access_token",
            app.config.github_web()
        ))
        .header(reqwest::header::ACCEPT, "application/json")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| Refresh::Failed(e.into()))?
        .json()
        .await
        .map_err(|e| Refresh::Failed(e.into()))?;
    granted(&answer)
}

/// The token endpoint's answer, as GitHub documents it: an error, or the new pair.
fn granted(answer: &serde_json::Value) -> Result<Granted, Refresh> {
    if let Some(err) = answer.get("error").and_then(|e| e.as_str()) {
        return Err(if err == "bad_refresh_token" {
            Refresh::Rejected
        } else {
            Refresh::Failed(anyhow!("token endpoint: {err}"))
        });
    }
    let secs = |k: &str| {
        answer
            .get(k)
            .and_then(|v| v.as_i64())
            .map(Duration::seconds)
    };
    let text = |k: &str| answer.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    Ok(Granted {
        access: text("access_token")
            .ok_or_else(|| Refresh::Failed(anyhow!("token endpoint: no access_token")))?,
        expires_in: secs("expires_in"),
        refresh: text("refresh_token"),
        refresh_expires_in: secs("refresh_token_expires_in"),
    })
}

/// Drop the tokens of people with no session that still counts: as an App, a session counts
/// for seven days after its team check (ADR 0022). Run on the full poll lane. Not in dev mode
/// (`RUBATO_DEV_USER`), where people act without sessions.
pub async fn sweep(app: &App) -> Result<u64> {
    if app.config.github_app.is_none() || app.config.dev_user.is_some() {
        return Ok(0);
    }
    let cutoff = ts(Utc::now() - Duration::seconds(super::team::TEAM_RECORD_MAX_AGE_SECS));
    let gone = sqlx::query(
        "DELETE FROM user_tokens WHERE login NOT IN (SELECT login FROM web_sessions WHERE team_checked_at IS NOT NULL AND team_checked_at >= ?)",
    )
    .bind(cutoff)
    .execute(&app.db)
    .await?
    .rows_affected();
    if gone > 0 {
        tracing::info!(gone, "user tokens dropped: no session that still counts");
    }
    Ok(gone)
}

/// After a logout: drop the tokens when it was the person's last session.
pub async fn after_logout(db: &SqlitePool, login: &str) -> Result<()> {
    let left: i64 = sqlx::query("SELECT COUNT(*) AS n FROM web_sessions WHERE login = ?")
        .bind(login)
        .fetch_one(db)
        .await?
        .get("n");
    if left == 0 {
        delete(db, login).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_sealed_token_opens_only_with_its_key() {
        let key = TokenKey([7; 32]);
        let sealed = seal(&key, "ghu_secret").unwrap();
        assert!(!sealed.contains("ghu_secret"));
        assert_ne!(
            sealed,
            seal(&key, "ghu_secret").unwrap(),
            "a fresh nonce each time"
        );
        assert_eq!(open(&key, &sealed).unwrap(), "ghu_secret");
        assert!(open(&TokenKey([8; 32]), &sealed).is_err());
    }

    #[test]
    fn reads_the_token_endpoint_answers() {
        let ok = granted(&json!({"access_token": "ghu_a", "expires_in": 28800, "refresh_token": "ghr_b", "refresh_token_expires_in": 15897600, "token_type": "bearer"})).ok().unwrap();
        assert_eq!(
            (ok.access.as_str(), ok.refresh.as_deref()),
            ("ghu_a", Some("ghr_b"))
        );
        assert_eq!(ok.expires_in, Some(Duration::hours(8)));
        assert!(matches!(
            granted(&json!({"error": "bad_refresh_token"})),
            Err(Refresh::Rejected)
        ));
        assert!(matches!(
            granted(&json!({"error": "incorrect_client_credentials"})),
            Err(Refresh::Failed(_))
        ));
        let no_expiry = granted(&json!({"access_token": "ghu_a", "token_type": "bearer"}))
            .ok()
            .unwrap();
        assert!(no_expiry.expires_in.is_none() && no_expiry.refresh.is_none());
    }
}
