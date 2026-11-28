//! Plan 345 task 2: the identity guard (Decision 15) and what the adapter tells `Me`.

use super::Admission;
use crate::kernel::config::Config;
use crate::kernel::db::ts;
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState};
use chrono::Utc;
use rubato_protocol::Role;
use std::sync::Arc;

async fn server() -> (App, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = crate::kernel::db::connect(&url).await.unwrap();
    let app = AppState::new(
        pool,
        Config::for_tests(url, "m0x6A"),
        Arc::new(FakeGitHub::new()),
    );
    app.set_admission(Arc::new(Admission));
    (app, tmp)
}

async fn exec(app: &App, sql: &str, binds: &[&str]) {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(*b);
    }
    q.execute(&app.db).await.unwrap();
}

/// `ada` as GitHub id 7: a session, a token, a live pairing with one queued issue, and a
/// membership of `acme`.
async fn ada_holds_everything(app: &App) {
    let now = ts(Utc::now());
    exec(
        app,
        "INSERT INTO users (login, github_id, created_at) VALUES ('ada', 7, ?)",
        &[&now],
    )
    .await;
    exec(
        app,
        "INSERT INTO web_sessions (id, login, created_at) VALUES ('s1', 'ada', ?)",
        &[&now],
    )
    .await;
    exec(app, "INSERT INTO user_tokens (login, access_ct, refresh_ct, expires_at, updated_at) VALUES ('ada', 'a', 'r', ?, ?)", &[&now, &now]).await;
    exec(app, "INSERT INTO pairings (companion_id, user, token_hash, hostname, created_at) VALUES ('00000000-0000-0000-0000-0000000000aa', 'ada', 'h1', 'laptop', ?)", &[&now]).await;
    exec(app, "INSERT INTO companion_queue (tenant_id, companion_id, repo, number, position, queued_by, queued_at) VALUES ('default', '00000000-0000-0000-0000-0000000000aa', 'acme/api', 1, 0, 'ada', ?)", &[&now]).await;
    let mut tx = app.db.begin().await.unwrap();
    crate::tenants::create_tenant(&mut tx, "acme", "Acme", "op", Utc::now())
        .await
        .unwrap()
        .unwrap();
    crate::tenants::add_member(&mut tx, "acme", 7, "ada", Role::Admin, "op", Utc::now())
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
}

async fn count(app: &App, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&app.db).await.unwrap()
}

#[tokio::test]
async fn the_same_id_changes_nothing() {
    let (app, _t) = server().await;
    ada_holds_everything(&app).await;
    assert!(!app.admission().before_login(&app, "ada", 7).await.unwrap());
    assert_eq!(count(&app, "SELECT COUNT(*) FROM web_sessions").await, 1);
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM pairings WHERE revoked_at IS NULL"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_reused_login_revokes_what_the_old_person_held() {
    let (app, _t) = server().await;
    ada_holds_everything(&app).await;
    // Someone else now owns the login `ADA` on GitHub, with id 99.
    assert!(app.admission().before_login(&app, "ADA", 99).await.unwrap());
    assert_eq!(count(&app, "SELECT COUNT(*) FROM web_sessions").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM user_tokens").await, 0);
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM pairings WHERE revoked_at IS NULL"
        )
        .await,
        0
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM companion_queue").await, 0);
    // The new person inherits no membership; the old one keeps theirs under their id.
    assert!(
        app.admission()
            .tenants_of(&app, 99)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(app.admission().tenants_of(&app, 7).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_user_with_no_stored_id_is_not_a_change() {
    let (app, _t) = server().await;
    exec(
        &app,
        "INSERT INTO users (login, created_at) VALUES ('old', ?)",
        &[&ts(Utc::now())],
    )
    .await;
    exec(
        &app,
        "INSERT INTO web_sessions (id, login, created_at) VALUES ('s', 'old', ?)",
        &[&ts(Utc::now())],
    )
    .await;
    assert!(!app.admission().before_login(&app, "old", 5).await.unwrap());
    assert!(
        !app.admission()
            .before_login(&app, "never-seen", 6)
            .await
            .unwrap()
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM web_sessions").await, 1);
}

#[tokio::test]
async fn me_lists_the_persons_tenants_and_a_login_marks_them_joined() {
    let (app, _t) = server().await;
    ada_holds_everything(&app).await;
    app.admission().after_login(&app, "Ada", 7).await.unwrap();
    let t = app.admission().tenants_of(&app, 7).await.unwrap();
    assert_eq!((t[0].id.as_str(), t[0].role), ("acme", Role::Admin));
    let m = crate::tenants::members(&app.db, "acme").await.unwrap();
    assert_eq!((m[0].login.as_str(), m[0].joined), ("Ada", true));
}
