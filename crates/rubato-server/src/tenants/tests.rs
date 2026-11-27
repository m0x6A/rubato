//! Plan 345 task 2: the tenancy store's rules and the seed (design §3, §3.3, §8).

use super::store;
use super::{DEFAULT_TENANT, start};
use crate::kernel::config::{Config, GitHubApp};
use crate::kernel::db::ts;
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState};
use chrono::{Duration, Utc};
use rubato_protocol::Role;
use rubato_protocol::tenant::reason;
use std::sync::Arc;

async fn server(cfg: impl FnOnce(&mut Config)) -> (App, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let mut c = Config::for_tests(url.clone(), "m0x6A");
    cfg(&mut c);
    let pool = crate::kernel::db::connect(&url).await.unwrap();
    (AppState::new(pool, c, Arc::new(FakeGitHub::new())), tmp)
}

async fn user(app: &App, login: &str, id: Option<i64>, session_days_ago: Option<i64>) {
    sqlx::query("INSERT INTO users (login, github_id, created_at) VALUES (?, ?, ?)")
        .bind(login)
        .bind(id)
        .bind(ts(Utc::now()))
        .execute(&app.db)
        .await
        .unwrap();
    if let Some(d) = session_days_ago {
        let at = ts(Utc::now() - Duration::days(d));
        sqlx::query(
            "INSERT INTO web_sessions (id, login, created_at, team_checked_at) VALUES (?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(login)
        .bind(&at)
        .bind(&at)
        .execute(&app.db)
        .await
        .unwrap();
    }
}

fn app_cfg(installation_id: u64) -> GitHubApp {
    GitHubApp {
        app_id: 1,
        private_key: "unused".into(),
        installation_id,
    }
}

#[tokio::test]
async fn a_tenant_id_is_checked_and_taken_once() {
    let (app, _t) = server(|_| {}).await;
    let mut tx = app.db.begin().await.unwrap();
    let now = Utc::now();
    assert_eq!(
        store::create_tenant(&mut tx, "Acme", "Acme", "op", now)
            .await
            .unwrap(),
        Err(store::Refused(reason::BAD_ID))
    );
    assert_eq!(
        store::create_tenant(&mut tx, "acme", "Acme", "op", now)
            .await
            .unwrap(),
        Ok(())
    );
    assert_eq!(
        store::create_tenant(&mut tx, "acme", "Again", "op", now)
            .await
            .unwrap(),
        Err(store::Refused(reason::DUPLICATE))
    );
}

#[tokio::test]
async fn a_deleted_tenants_id_is_not_reused_while_its_rows_remain() {
    let (app, _t) = server(|_| {}).await;
    let mut tx = app.db.begin().await.unwrap();
    let now = Utc::now();
    store::create_tenant(&mut tx, "acme", "Acme", "op", now)
        .await
        .unwrap()
        .unwrap();
    store::add_member(&mut tx, "acme", 7, "ada", Role::Admin, "op", now)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("DELETE FROM tenants WHERE id = 'acme'")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        store::create_tenant(&mut tx, "acme", "Acme", "op", now)
            .await
            .unwrap(),
        Err(store::Refused(reason::ID_HAS_ROWS))
    );
}

#[tokio::test]
async fn an_installation_belongs_to_one_tenant() {
    let (app, _t) = server(|_| {}).await;
    let mut tx = app.db.begin().await.unwrap();
    let now = Utc::now();
    for t in ["acme", "bcorp"] {
        store::create_tenant(&mut tx, t, t, "op", now)
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(
        store::attach_installation(&mut tx, "acme", 11, "op", now)
            .await
            .unwrap(),
        Ok(())
    );
    assert_eq!(
        store::attach_installation(&mut tx, "acme", 11, "op", now)
            .await
            .unwrap(),
        Ok(()),
        "attaching again to the same tenant changes nothing"
    );
    assert_eq!(
        store::attach_installation(&mut tx, "bcorp", 11, "op", now)
            .await
            .unwrap(),
        Err(store::Refused(reason::INSTALLATION_TAKEN))
    );
}

#[tokio::test]
async fn a_detached_installation_with_kept_rows_cannot_move_until_purged() {
    let (app, _t) = server(|_| {}).await;
    let mut tx = app.db.begin().await.unwrap();
    let now = Utc::now();
    for t in ["acme", "bcorp"] {
        store::create_tenant(&mut tx, t, t, "op", now)
            .await
            .unwrap()
            .unwrap();
    }
    store::attach_installation(&mut tx, "acme", 11, "op", now)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("INSERT INTO tenant_repos (repo, tenant_id, installation_id, seen_at) VALUES ('acme/api', 'acme', 11, ?)")
        .bind(ts(now))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE tenant_installations SET detached_at = ? WHERE installation_id = 11")
        .bind(ts(now))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE tenant_repos SET detached_at = ? WHERE installation_id = 11")
        .bind(ts(now))
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        store::attach_installation(&mut tx, "bcorp", 11, "op", now)
            .await
            .unwrap(),
        Err(store::Refused(reason::ROWS_OF_OTHER_TENANT))
    );
    // Back to the tenant it left: its repositories are live again.
    store::attach_installation(&mut tx, "acme", 11, "op", now)
        .await
        .unwrap()
        .unwrap();
    let detached: Option<String> =
        sqlx::query_scalar("SELECT detached_at FROM tenant_repos WHERE repo = 'acme/api'")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(detached, None);
}

#[tokio::test]
async fn the_last_admin_is_kept() {
    let (app, _t) = server(|_| {}).await;
    let mut tx = app.db.begin().await.unwrap();
    let now = Utc::now();
    store::create_tenant(&mut tx, "acme", "Acme", "op", now)
        .await
        .unwrap()
        .unwrap();
    store::add_member(&mut tx, "acme", 1, "ada", Role::Admin, "op", now)
        .await
        .unwrap()
        .unwrap();
    store::add_member(&mut tx, "acme", 2, "bob", Role::Member, "ada", now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store::add_member(&mut tx, "acme", 2, "bob", Role::Admin, "ada", now)
            .await
            .unwrap(),
        Err(store::Refused(reason::ALREADY_MEMBER))
    );
    assert_eq!(
        store::set_role(&mut tx, "acme", 1, Role::Member)
            .await
            .unwrap(),
        Err(store::Refused(reason::LAST_ADMIN))
    );
    assert_eq!(
        store::remove_member(&mut tx, "acme", 1).await.unwrap(),
        Err(store::Refused(reason::LAST_ADMIN))
    );
    store::set_role(&mut tx, "acme", 2, Role::Admin)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store::remove_member(&mut tx, "acme", 1).await.unwrap(),
        Ok(store::Removed {
            last_membership: true
        })
    );
    assert_eq!(
        store::remove_member(&mut tx, "acme", 1).await.unwrap(),
        Err(store::Refused(reason::NOT_A_MEMBER))
    );
}

#[tokio::test]
async fn removal_says_whether_the_person_is_still_in_another_tenant() {
    let (app, _t) = server(|_| {}).await;
    let mut tx = app.db.begin().await.unwrap();
    let now = Utc::now();
    for t in ["acme", "bcorp"] {
        store::create_tenant(&mut tx, t, t, "op", now)
            .await
            .unwrap()
            .unwrap();
        store::add_member(&mut tx, t, 1, "ada", Role::Admin, "op", now)
            .await
            .unwrap()
            .unwrap();
        store::add_member(&mut tx, t, 2, "bob", Role::Member, "ada", now)
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(
        store::remove_member(&mut tx, "acme", 2).await.unwrap(),
        Ok(store::Removed {
            last_membership: false
        })
    );
    tx.commit().await.unwrap();
    let left = store::tenants_of(&app.db, 2).await.unwrap();
    assert_eq!(
        left.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["bcorp"]
    );
    assert_eq!(store::role_of(&app.db, "acme", 2).await.unwrap(), None);
    assert_eq!(
        store::role_of(&app.db, "bcorp", 2).await.unwrap(),
        Some(Role::Member)
    );
}

#[tokio::test]
async fn a_login_refreshes_the_members_login_and_marks_them_joined() {
    let (app, _t) = server(|_| {}).await;
    let mut tx = app.db.begin().await.unwrap();
    let now = Utc::now();
    store::create_tenant(&mut tx, "acme", "Acme", "op", now)
        .await
        .unwrap()
        .unwrap();
    store::add_member(&mut tx, "acme", 3, "old-name", Role::Member, "op", now)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    assert!(!store::members(&app.db, "acme").await.unwrap()[0].joined);
    store::note_login(&app.db, 3, "new-name", now)
        .await
        .unwrap();
    let m = &store::members(&app.db, "acme").await.unwrap()[0];
    assert_eq!((m.login.as_str(), m.joined), ("new-name", true));
}

#[tokio::test]
async fn the_seed_makes_default_from_the_server_as_it_was() {
    let (app, _t) = server(|c| {
        c.github_app = Some(app_cfg(4242));
        c.operators = vec![100];
    })
    .await;
    user(&app, "mattias", Some(100), Some(2)).await; // operator, recent
    user(&app, "ada", Some(101), Some(29)).await; // recent
    user(&app, "gone", Some(102), Some(45)).await; // no session for 45 days
    user(&app, "nobody", None, Some(1)).await; // recent, no GitHub id
    let s = start(&app).await.unwrap();
    assert_eq!(s.tenant.as_deref(), Some(DEFAULT_TENANT));
    assert_eq!(s.installation, Some(4242));
    assert_eq!((s.members, s.admins), (2, 1));
    assert_eq!(s.left_out, ["gone", "nobody"]);
    assert_eq!(
        store::role_of(&app.db, "default", 100).await.unwrap(),
        Some(Role::Admin)
    );
    assert_eq!(
        store::role_of(&app.db, "default", 101).await.unwrap(),
        Some(Role::Member)
    );
    assert_eq!(
        store::installations_of(&app.db, "default").await.unwrap(),
        [4242]
    );
}

#[tokio::test]
async fn the_seed_runs_once() {
    let (app, _t) = server(|c| c.github_app = Some(app_cfg(4242))).await;
    assert!(start(&app).await.unwrap().tenant.is_some());
    user(&app, "late", Some(5), Some(0)).await;
    let again = start(&app).await.unwrap();
    assert_eq!(again, super::Seeded::default());
    assert_eq!(store::role_of(&app.db, "default", 5).await.unwrap(), None);
}

#[tokio::test]
async fn an_operator_is_admitted_by_the_seed_even_without_a_recent_session() {
    let (app, _t) = server(|c| c.operators = vec![9]).await;
    user(&app, "op", Some(9), None).await;
    let s = start(&app).await.unwrap();
    assert_eq!((s.members, s.admins, s.installation), (1, 1, None));
}

#[tokio::test]
async fn a_listing_adds_tombstones_and_never_takes_another_tenants_repository() {
    let (app, _t) = server(|_| {}).await;
    let mut tx = app.db.begin().await.unwrap();
    let now = Utc::now();
    for (t, id) in [("acme", 11), ("bcorp", 22)] {
        store::create_tenant(&mut tx, t, t, "op", now)
            .await
            .unwrap()
            .unwrap();
        store::attach_installation(&mut tx, t, id, "op", now)
            .await
            .unwrap()
            .unwrap();
    }
    let listed = |r: &[&str]| r.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let s = store::sync_repos(&mut tx, "acme", 11, &listed(&["acme/api", "acme/web"]), now)
        .await
        .unwrap();
    assert_eq!(s.added, ["acme/api", "acme/web"]);
    // bcorp's installation names acme's repository: it stays acme's.
    let s = store::sync_repos(&mut tx, "bcorp", 22, &listed(&["bcorp/x", "ACME/api"]), now)
        .await
        .unwrap();
    assert_eq!(
        (s.added, s.conflicts),
        (listed(&["bcorp/x"]), listed(&["ACME/api"]))
    );
    // acme's listing loses a repository: tombstoned, not deleted.
    let s = store::sync_repos(&mut tx, "acme", 11, &listed(&["acme/api"]), now)
        .await
        .unwrap();
    assert_eq!(s.removed, ["acme/web"]);
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tenant_repos WHERE repo = 'acme/web' AND detached_at IS NOT NULL",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(n, 1);
    tx.commit().await.unwrap();
    let routes = store::live_routes(&app.db).await.unwrap();
    let mut repos: Vec<(String, u64)> = routes.repos.into_iter().collect();
    repos.sort();
    assert_eq!(
        repos,
        [("acme/api".to_owned(), 11), ("bcorp/x".to_owned(), 22)]
    );
}

#[tokio::test]
async fn a_reinstall_into_the_same_tenant_brings_its_repositories_back_live() {
    let (app, _t) = server(|_| {}).await;
    let mut tx = app.db.begin().await.unwrap();
    let now = Utc::now();
    store::create_tenant(&mut tx, "acme", "Acme", "op", now)
        .await
        .unwrap()
        .unwrap();
    store::attach_installation(&mut tx, "acme", 11, "op", now)
        .await
        .unwrap()
        .unwrap();
    store::sync_repos(&mut tx, "acme", 11, &["acme/api".to_owned()], now)
        .await
        .unwrap();
    assert_eq!(
        store::detach_installation(&mut tx, 11, now).await.unwrap(),
        ["acme/api"]
    );
    // Uninstalled and installed again: GitHub gives a new id.
    store::attach_installation(&mut tx, "acme", 33, "op", now)
        .await
        .unwrap()
        .unwrap();
    let s = store::sync_repos(&mut tx, "acme", 33, &["acme/api".to_owned()], now)
        .await
        .unwrap();
    assert_eq!(s.added, ["acme/api"]);
    assert!(store::set_suspended(&mut tx, 33, Some(now)).await.unwrap());
    tx.commit().await.unwrap();
    let routes = store::live_routes(&app.db).await.unwrap();
    assert_eq!(routes.repos.get("acme/api"), Some(&33));
    assert!(routes.installations[&33].suspended);
    assert!(!routes.installations.contains_key(&11), "detached");
}
