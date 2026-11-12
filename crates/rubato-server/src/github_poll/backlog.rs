//! Every open issue of the repos the map names (ADR 0019): one listing per repo per tick, the
//! repo's rows replaced on success and kept on failure, with the time and the error recorded so
//! a stale or failed listing is never shown as "no issues".

use super::{store, suggest};
use crate::kernel::github::OpenIssueInfo;
use crate::kernel::state::App;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{IssueRef, MapRef, SystemMap, parse_request_line};
use std::collections::{HashMap, HashSet};

/// At most this many open issues per repo: five pages, so five calls per repo per tick.
pub const BACKLOG_MAX_PER_REPO: usize = 500;

/// What this tick's listings say, for the per-issue reads (issue #49): the repos listed whole
/// (not failed, not cut) and their open issues.
#[derive(Debug, Default)]
pub struct Listed {
    fresh: HashSet<String>,
    open: HashMap<IssueRef, OpenIssueInfo>,
    map_lines: Vec<MapLine>,
}

/// A `Map:` line from a trusted author, for the pins slice to apply (ADR 0026).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapLine {
    pub issue: IssueRef,
    pub target: MapRef,
    /// Login of the issue's author, who placed it.
    pub author: String,
}

impl Listed {
    /// This tick's `Map:` lines from trusted authors, in listing order.
    pub fn map_lines(&self) -> &[MapLine] {
        &self.map_lines
    }

    /// Whether every open issue of `repo` is in this tick's listing.
    pub fn is_fresh(&self, repo: &str) -> bool {
        self.fresh.contains(repo)
    }

    /// The issue as this tick listed it, if it is open.
    pub fn open(&self, issue: &IssueRef) -> Option<&OpenIssueInfo> {
        self.open.get(issue)
    }

    /// The repos listed whole this tick.
    pub fn fresh_repos(&self) -> &HashSet<String> {
        &self.fresh
    }

    /// Listed issues whose body carries a `Rubato-Request:` line (ADR 0025), as
    /// (issue, page, request id): how a create GitHub did not confirm is found.
    pub fn requests(&self) -> Vec<(IssueRef, String, String)> {
        let mut v: Vec<_> = self
            .open
            .iter()
            .filter_map(|(issue, i)| {
                parse_request_line(&i.body).map(|id| (issue.clone(), i.url.clone(), id))
            })
            .collect();
        v.sort();
        v
    }
}

/// List each repo the map names and store the result. Never fails the poll: a repo that cannot
/// be listed keeps its rows and gets its error recorded.
pub async fn list_open_issues(app: &App, map: &SystemMap) -> Listed {
    let repos: Vec<String> = map.repos_named().into_iter().map(str::to_owned).collect();
    let mut listed = Listed::default();
    let mut counts = Counts::default();
    for repo in &repos {
        match app.github.open_issues(repo, BACKLOG_MAX_PER_REPO).await {
            Ok(listing) => {
                if !listing.truncated {
                    listed.fresh.insert(repo.clone());
                }
                let mut placements = HashMap::new();
                for i in &listing.issues {
                    let p = suggest::place(map, i);
                    counts.listed += 1;
                    if let Some(target) = &p.pin {
                        counts.map_lines += 1;
                        listed.map_lines.push(MapLine {
                            issue: IssueRef::new(repo.clone(), i.number),
                            target: target.clone(),
                            author: i.author.clone(),
                        });
                    } else if p.untrusted_line.is_some() {
                        // Counted per listing, not once: the line stays untrusted every tick.
                        counts.untrusted += 1;
                        app.metrics
                            .backlog_map_lines
                            .add(1, &[KeyValue::new(attr::OUTCOME, "untrusted_author")]);
                    }
                    if p.pin.is_none() && p.suggestion.is_none() {
                        counts.none += 1;
                    }
                    if p.suggestion.is_some() {
                        counts.suggested += 1;
                    }
                    placements.insert(i.number, p);
                }
                listed.open.extend(
                    listing
                        .issues
                        .iter()
                        .map(|i| (IssueRef::new(repo.clone(), i.number), i.clone())),
                );
                if listing.truncated {
                    tracing::warn!(
                        rubato.repo = %repo,
                        count = listing.issues.len(),
                        "backlog truncated"
                    );
                }
                if let Err(e) =
                    store::replace_open_issues(&app.db, repo, &listing, &placements, Utc::now())
                        .await
                {
                    tracing::error!(rubato.repo = %repo, error = %e, "storing open issues failed");
                }
            }
            Err(e) => {
                tracing::warn!(rubato.repo = %repo, error = %e, "open issues listing failed");
                if let Err(e) = store::record_listing_error(&app.db, repo, &format!("{e:#}")).await
                {
                    tracing::error!(rubato.repo = %repo, error = %e, "storing the listing error failed");
                }
            }
        }
    }
    // One line per tick: how many listed issues placed themselves, were placed by someone
    // outside the team, got a suggestion, or neither.
    tracing::info!(
        listed = counts.listed,
        map_lines = counts.map_lines,
        untrusted = counts.untrusted,
        suggested = counts.suggested,
        none = counts.none,
        "backlog placement"
    );
    record_gauges(app, &repos).await;
    listed
}

#[derive(Debug, Default)]
struct Counts {
    listed: usize,
    map_lines: usize,
    untrusted: usize,
    suggested: usize,
    none: usize,
}

/// Both gauges on every tick, for every repo with a row: a gauge recorded only on change keeps
/// its last value. A repo the map no longer names is recorded once as 0 and its rows removed.
async fn record_gauges(app: &App, named: &[String]) {
    let (issues, repos) = match store::backlog(&app.db).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "reading the backlog failed");
            return;
        }
    };
    for r in &repos {
        let attrs = [KeyValue::new(attr::REPO, r.repo.clone())];
        if named.contains(&r.repo) {
            let n = issues.iter().filter(|i| i.issue.repo == r.repo).count();
            app.metrics.backlog_issues.record(n as u64, &attrs);
            let at = r
                .listed_at
                .and_then(|t| u64::try_from(t.timestamp()).ok())
                .unwrap_or(0);
            app.metrics.backlog_listed_at.record(at, &attrs);
        } else {
            app.metrics.backlog_issues.record(0, &attrs);
            app.metrics.backlog_listed_at.record(0, &attrs);
            if let Err(e) = store::forget_repo(&app.db, &r.repo).await {
                tracing::error!(rubato.repo = %r.repo, error = %e, "forgetting a repo failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::db::connect;
    use crate::kernel::github::{FakeGitHub, OpenIssueInfo};
    use crate::kernel::state::AppState;
    use crate::kernel::telemetry::{capture, metric_reader};
    use rubato_protocol::otel::metric;
    use rubato_protocol::{MapNode, NodeKind, RepoRef};
    use std::sync::Arc;

    fn issue(n: u64) -> OpenIssueInfo {
        OpenIssueInfo {
            number: n,
            title: format!("Issue {n}"),
            url: format!("https://github.com/o/r/issues/{n}"),
            labels: vec!["bug".into()],
            updated_at: Utc::now(),
            ..OpenIssueInfo::default()
        }
    }

    /// A map naming `o/listed` in `repos[]` and `o/node-only` on a node.
    fn map() -> SystemMap {
        SystemMap {
            version: 1,
            name: "t".into(),
            owner_rule: None,
            repos: vec![RepoRef {
                name: "o/listed".into(),
                plan_path: None,
            }],
            nodes: vec![MapNode {
                id: "a".into(),
                kind: NodeKind::Context,
                repo: Some("o/node-only".into()),
                path: None,
                description: None,
            }],
            seams: vec![],
            scenarios: vec![],
        }
    }

    async fn app(github: Arc<FakeGitHub>) -> (App, metric_reader::Reader, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(pool, Config::for_tests(url, "m0x6A"), github);
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        (app, reader, tmp)
    }

    fn value(reader: &metric_reader::Reader, name: &str, repo: &str) -> Option<u64> {
        reader
            .points(name)
            .into_iter()
            .find(|p| p.attrs.get(attr::REPO).map(String::as_str) == Some(repo))
            .map(|p| p.value)
    }

    #[tokio::test]
    async fn lists_each_map_repo_once_and_replaces_rows() {
        let github = Arc::new(FakeGitHub::new());
        // The fake answers as the port does: open issues only, pull requests already left out
        // (`open_issues_skips_pull_requests_and_pages` covers that filter).
        github.with(|g| {
            g.open
                .insert("o/listed".into(), vec![issue(1), issue(2), issue(3)]);
            g.open.insert("o/node-only".into(), vec![issue(9)]);
        });
        let (app, reader, _tmp) = app(github.clone()).await;
        list_open_issues(&app, &map()).await;

        let (issues, repos) = store::backlog(&app.db).await.unwrap();
        assert_eq!(issues.len(), 4, "{issues:?}");
        assert_eq!(repos.len(), 2);
        assert!(
            repos
                .iter()
                .all(|r| r.listed_at.is_some() && r.error.is_none())
        );
        assert_eq!(issues[0].labels, ["bug"]);
        // One call per repo, and a repo named only on a node is listed too.
        assert_eq!(
            github.calls(),
            [
                ("open_issues", "o/listed".to_string()),
                ("open_issues", "o/node-only".to_string())
            ]
        );
        assert_eq!(value(&reader, metric::BACKLOG_ISSUES, "o/listed"), Some(3));
        let at = value(&reader, metric::BACKLOG_LISTED_AT, "o/listed").unwrap();
        assert!(at > 1_700_000_000, "unix seconds: {at}");
        let point = &reader.points(metric::BACKLOG_LISTED_AT)[0];
        assert_eq!(point.unit, "", "no unit, or Prometheus renames the series");

        // Issue 2 closes: the next listing replaces the rows.
        github.with(|g| {
            g.open.insert("o/listed".into(), vec![issue(1), issue(3)]);
        });
        list_open_issues(&app, &map()).await;
        let (issues, _) = store::backlog(&app.db).await.unwrap();
        let listed: Vec<u64> = issues
            .iter()
            .filter(|i| i.issue.repo == "o/listed")
            .map(|i| i.issue.number)
            .collect();
        assert_eq!(listed, [1, 3]);
        assert_eq!(value(&reader, metric::BACKLOG_ISSUES, "o/listed"), Some(2));
        println!(
            "checked {} GitHub calls over 2 repos and 2 ticks",
            github.calls().len()
        );
    }

    #[tokio::test]
    async fn failed_listing_keeps_rows_and_records_error() {
        let github = Arc::new(FakeGitHub::new());
        github.with(|g| {
            g.open.insert("o/listed".into(), vec![issue(1)]);
        });
        let (app, reader, _tmp) = app(github.clone()).await;
        list_open_issues(&app, &map()).await;
        let (_, before) = store::backlog(&app.db).await.unwrap();
        let listed_at = before
            .iter()
            .find(|r| r.repo == "o/listed")
            .unwrap()
            .listed_at;
        let gauge_before = value(&reader, metric::BACKLOG_LISTED_AT, "o/listed");

        github.with(|g| {
            g.open_errors
                .insert("o/listed".into(), "rate limited".into());
        });
        let (events, _guard) = capture::start();
        list_open_issues(&app, &map()).await;
        let (issues, repos) = store::backlog(&app.db).await.unwrap();
        let repo = repos.iter().find(|r| r.repo == "o/listed").unwrap();
        assert_eq!(
            issues.len(),
            1,
            "the rows of the last good listing are kept"
        );
        assert_eq!(repo.listed_at, listed_at, "the last good time is kept");
        assert_eq!(repo.error.as_deref(), Some("rate limited"));
        assert_eq!(
            value(&reader, metric::BACKLOG_LISTED_AT, "o/listed"),
            gauge_before,
            "the gauge keeps the last good time, so the age grows"
        );
        {
            let events = events.lock().unwrap();
            let warned = events
                .iter()
                .find(|e| e.message == "open issues listing failed")
                .expect("a warn line");
            assert_eq!(warned.level, tracing::Level::WARN);
            assert_eq!(
                warned.fields.get("rubato.repo").map(String::as_str),
                Some("o/listed")
            );
        }

        // It recovers: the error clears.
        github.with(|g| g.open_errors.clear());
        list_open_issues(&app, &map()).await;
        let (_, repos) = store::backlog(&app.db).await.unwrap();
        assert!(repos.iter().all(|r| r.error.is_none()));
    }

    #[tokio::test]
    async fn cut_at_500_sets_truncated() {
        let github = Arc::new(FakeGitHub::new());
        github.with(|g| {
            g.open
                .insert("o/listed".into(), (1..=501).map(issue).collect());
        });
        let (app, reader, _tmp) = app(github).await;
        let (events, _guard) = capture::start();
        list_open_issues(&app, &map()).await;
        let (issues, repos) = store::backlog(&app.db).await.unwrap();
        assert_eq!(issues.len(), BACKLOG_MAX_PER_REPO);
        assert!(
            repos
                .iter()
                .find(|r| r.repo == "o/listed")
                .unwrap()
                .truncated
        );
        assert_eq!(
            value(&reader, metric::BACKLOG_ISSUES, "o/listed"),
            Some(500)
        );
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.message == "backlog truncated" && e.level == tracing::Level::WARN)
        );
    }

    #[tokio::test]
    async fn a_repo_the_map_drops_is_zeroed_once_and_forgotten() {
        let github = Arc::new(FakeGitHub::new());
        github.with(|g| {
            g.open.insert("o/node-only".into(), vec![issue(9)]);
        });
        let (app, reader, _tmp) = app(github).await;
        list_open_issues(&app, &map()).await;
        let mut smaller = map();
        smaller.nodes[0].repo = None;
        list_open_issues(&app, &smaller).await;
        assert_eq!(
            value(&reader, metric::BACKLOG_ISSUES, "o/node-only"),
            Some(0)
        );
        assert_eq!(
            value(&reader, metric::BACKLOG_LISTED_AT, "o/node-only"),
            Some(0)
        );
        let (issues, repos) = store::backlog(&app.db).await.unwrap();
        assert!(issues.iter().all(|i| i.issue.repo != "o/node-only"));
        assert!(repos.iter().all(|r| r.repo != "o/node-only"));
    }
}
