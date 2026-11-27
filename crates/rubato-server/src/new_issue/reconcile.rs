//! Every poll tick, after the open-issues listing: finish the creations GitHub did not confirm.
//! An issue the App wrote whose body carries a `Rubato-Request:` line of an `opening` row is
//! that row's issue, the lowest number when there are several (issue #165); the line on anyone
//! else's issue is ignored, logged and counted. A row unseen for [`OPENING_LIMIT`] in a repo
//! listed whole is `failed`. The gauge `rubato.issues.opening` is recorded every tick, zeros
//! included.

use super::handlers::count;
use super::store;
use crate::kernel::state::App;
use crate::pins;
use chrono::{Duration, Utc};
use rubato_protocol::IssueRef;
use std::collections::{HashMap, HashSet};

/// How long a create may stay unconfirmed before it is given up.
pub const OPENING_LIMIT: Duration = Duration::minutes(10);

/// An open issue whose body names a request id, and who wrote it.
#[derive(Debug, Clone)]
pub struct Found {
    pub issue: IssueRef,
    pub url: String,
    pub request_id: String,
    /// Login of the issue's author.
    pub author: String,
    /// The author is a Bot account.
    pub author_is_bot: bool,
}

/// Why `f` is not the App's own issue, or `None` when it is: a Bot, and the App's login when
/// the port knows it (before the first installation read, any Bot).
fn not_the_apps(f: &Found, app_login: Option<&str>) -> Option<String> {
    if !f.author_is_bot {
        return Some("not a bot".into());
    }
    match app_login {
        Some(login) if !f.author.eq_ignore_ascii_case(login) => {
            Some(format!("bot {}, not {login}", f.author))
        }
        _ => None,
    }
}

/// `fresh`: the repos this tick listed whole. Only there does "not seen" mean "not created".
pub async fn reconcile(app: &App, found: &[Found], fresh: &HashSet<String>) {
    let rows = match store::opening(&app.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "reading the opening issue creations failed");
            return;
        }
    };
    let app_login = app.github.app_login();
    let opening: HashSet<(&str, &str)> = rows
        .iter()
        .map(|c| (c.repo.as_str(), c.request_id.as_str()))
        .collect();
    // Per repo and request id, the App's issue with the lowest number: the App sends each id
    // once, so a second issue with the line is a copy.
    let mut by_id: HashMap<(&str, &str), &Found> = HashMap::new();
    for f in found {
        let key = (f.issue.repo.as_str(), f.request_id.as_str());
        if let Some(why) = not_the_apps(f, app_login.as_deref()) {
            if opening.contains(&key) {
                count(app, "ignored");
                tracing::info!(
                    rubato.issue = %f.issue,
                    request_id = %f.request_id,
                    author = %f.author,
                    reason = %why,
                    "request line ignored"
                );
            }
            continue;
        }
        by_id
            .entry(key)
            .and_modify(|k| {
                if f.issue.number < k.issue.number {
                    *k = f;
                }
            })
            .or_insert(f);
    }
    let mut changed = false;
    let mut still = 0u64;
    for c in &rows {
        let done = if let Some(f) = by_id.get(&(c.repo.as_str(), c.request_id.as_str())) {
            finish(app, c, f).await
        } else if fresh.contains(&c.repo) && Utc::now() - c.created_at > OPENING_LIMIT {
            give_up(app, c).await
        } else {
            false
        };
        if done {
            changed = true;
        } else {
            still += 1;
        }
    }
    app.metrics.issues_opening.record(still, &[]);
    if changed {
        app.notify("issue");
    }
}

async fn finish(app: &App, c: &store::Creation, f: &Found) -> bool {
    match store::created(&app.db, &c.workspace, &c.request_id, f.issue.number, &f.url).await {
        Ok(true) => {}
        Ok(false) => return true,
        Err(e) => {
            tracing::error!(request_id = %c.request_id, error = %e, "storing a found issue failed");
            return false;
        }
    }
    if let Err(e) = pins::pin_created(app, &c.workspace, &f.issue, &c.target, &c.created_by).await {
        tracing::error!(rubato.issue = %f.issue, error = %e, "pinning a found issue failed");
    }
    count(app, "found");
    tracing::info!(
        rubato.issue = %f.issue,
        request_id = %c.request_id,
        "issue found by its Rubato-Request line"
    );
    true
}

async fn give_up(app: &App, c: &store::Creation) -> bool {
    let why = "GitHub did not confirm it and no listing showed it for ten minutes";
    match store::failed(&app.db, &c.workspace, &c.request_id, why).await {
        Ok(changed) => {
            if changed {
                count(app, "failed");
                tracing::warn!(
                    step = "unseen",
                    rubato.repo = %c.repo,
                    request_id = %c.request_id,
                    "issue creation failed"
                );
            }
            true
        }
        Err(e) => {
            tracing::error!(request_id = %c.request_id, error = %e, "failing an issue creation failed");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::db::connect;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use crate::kernel::telemetry::metric_reader;
    use rubato_protocol::MapRef;
    use rubato_protocol::otel::{attr, metric};
    use std::sync::Arc;

    fn found(n: u64, author: &str, bot: bool, id: &str) -> Found {
        Found {
            issue: IssueRef::new("o/r", n),
            url: format!("https://github.com/o/r/issues/{n}"),
            request_id: id.into(),
            author: author.into(),
            author_is_bot: bot,
        }
    }

    fn outcomes(reader: &metric_reader::Reader) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = reader
            .points(metric::ISSUES_CREATED)
            .into_iter()
            .map(|p| {
                (
                    p.attrs.get(attr::OUTCOME).cloned().unwrap_or_default(),
                    p.value,
                )
            })
            .collect();
        v.sort();
        v
    }

    /// Issue #165: copies of the line from a person and from another Bot are ignored and
    /// counted; the App's issue with the lowest number is found, and an id in another repo or
    /// of no opening row counts nothing.
    #[tokio::test]
    async fn only_the_apps_issue_finishes_a_creation() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let github = Arc::new(FakeGitHub::new());
        github.with(|g| g.app_login = Some("rubato[bot]".into()));
        let mut app = AppState::new(pool, Config::for_tests(url, "m0x6A"), github);
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        assert!(
            store::begin(
                &app.db,
                &crate::kernel::tenancy::WsKey::of_default(rubato_protocol::DEFAULT_WORKSPACE),
                "r-1",
                "o/r",
                &MapRef::node("web"),
                "T",
                "teammate",
                Utc::now(),
            )
            .await
            .unwrap()
        );
        let listed = [
            found(9, "stranger", false, "r-1"),
            found(8, "other-app[bot]", true, "r-1"),
            found(7, "Rubato[bot]", true, "r-1"),
            found(3, "rubato[bot]", true, "r-1"),
            found(2, "stranger", false, "r-2"),
            Found {
                issue: IssueRef::new("o/elsewhere", 1),
                ..found(1, "rubato[bot]", true, "r-1")
            },
        ];
        reconcile(&app, &listed, &HashSet::new()).await;
        let c = store::get(
            &app.db,
            &crate::kernel::tenancy::WsKey::of_default("default"),
            "r-1",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(c.number, Some(3), "the App's issue with the lowest number");
        let pins = crate::pins::list(
            &app.db,
            &crate::kernel::tenancy::WsKey::of_default(rubato_protocol::DEFAULT_WORKSPACE),
        )
        .await
        .unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].issue, IssueRef::new("o/r", 3));
        assert_eq!(pins[0].pinned_by, "teammate");
        assert_eq!(
            outcomes(&reader),
            [("found".to_owned(), 1), ("ignored".to_owned(), 2)]
        );
        println!("checked 6 listed requests against 1 opening row");
    }
}
