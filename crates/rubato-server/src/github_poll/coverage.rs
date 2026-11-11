//! Which repos the GitHub App installation covers (ADR 0022). Reads turn a 404 into "absent"
//! (`kernel/github/api.rs`), which was harmless behind a broad personal token; behind an
//! installation a repo left out of it would look like "no branch, no plan, no issue". So each
//! tick the installation's repositories are listed and every repo the map or a tracked issue
//! names that is not among them is recorded, warned about and shown as a finding.

use super::store;
use crate::kernel::state::App;
use chrono::Utc;
use std::collections::BTreeSet;

/// Record the repos of `needed` the installation does not cover. On a port that is not an
/// installation nothing is called: the gauge is 0 and nothing is uncovered.
pub async fn check_coverage(app: &App, needed: &BTreeSet<String>) {
    let uncovered: Vec<String> = if app.github.is_installation() {
        match app.github.installation_repos().await {
            Ok(covered) => needed
                .iter()
                .filter(|r| !covered.iter().any(|c| c.eq_ignore_ascii_case(r)))
                .cloned()
                .collect(),
            Err(e) => {
                // Unknown is not "all covered": keep what the last good check found.
                tracing::warn!(error = %e, "listing the installation's repositories failed");
                return;
            }
        }
    } else {
        Vec::new()
    };
    for repo in &uncovered {
        tracing::warn!(rubato.repo = %repo, "repo not covered by the installation");
    }
    app.metrics
        .github_repos_uncovered
        .record(uncovered.len() as u64, &[]);
    if let Err(e) = store::replace_uncovered(&app.db, &uncovered, Utc::now()).await {
        tracing::error!(error = %e, "storing uncovered repos failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::db::connect;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use crate::kernel::telemetry::{capture, metric_reader};
    use rubato_protocol::otel::metric;
    use std::sync::Arc;

    #[tokio::test]
    async fn a_repo_outside_the_installation_is_a_finding_not_an_absence() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = connect(&url).await.unwrap();
        let github = Arc::new(FakeGitHub::new());
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(pool, Config::for_tests(url, "m0x6A"), github.clone());
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        let needed: BTreeSet<String> = ["o/covered", "o/left-out"].map(String::from).into();
        let gauge = |r: &metric_reader::Reader| {
            let p = r.points(metric::GITHUB_REPOS_UNCOVERED);
            assert_eq!(p.len(), 1, "{p:?}");
            p[0].value
        };

        // Not an installation (a personal token, the fake): no call, nothing uncovered, 0.
        check_coverage(&app, &needed).await;
        assert!(github.calls().is_empty(), "{:?}", github.calls());
        assert_eq!(gauge(&reader), 0);
        assert!(store::uncovered(&app.db).await.unwrap().is_empty());

        // An installation that covers one of the two.
        github.with(|g| g.installation = Some(vec!["O/Covered".into()]));
        let (events, _guard) = capture::start();
        check_coverage(&app, &needed).await;
        assert_eq!(github.calls(), [("installation_repos", String::new())]);
        assert_eq!(store::uncovered(&app.db).await.unwrap(), ["o/left-out"]);
        assert_eq!(gauge(&reader), 1);
        {
            let events = events.lock().unwrap();
            let w = events
                .iter()
                .find(|e| e.message == "repo not covered by the installation")
                .expect("a warn line");
            assert_eq!(w.level, tracing::Level::WARN);
            assert_eq!(
                w.fields.get("rubato.repo").map(String::as_str),
                Some("o/left-out")
            );
        }

        // The repo is added to the installation: the finding goes away and the gauge is 0.
        github.with(|g| g.installation = Some(vec!["o/covered".into(), "o/left-out".into()]));
        check_coverage(&app, &needed).await;
        assert!(store::uncovered(&app.db).await.unwrap().is_empty());
        assert_eq!(gauge(&reader), 0);
        println!("checked 2 repos against 3 installations");
    }
}
