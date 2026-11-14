//! Composition root: the periodic poll loop and the supervision of background tasks.
//!
//! The poll keeps the order of design §9.1: reload the map when `map_age == 0`, then the open
//! issues of the map's repos and the installation's coverage (issue #48), then per issue
//! `upsert_meta → find_branch → (plan, PR) when a branch exists → decisions`, then rate limit,
//! gauges and `notify("poll")`. The branch is looked up once here and passed to both slices, so
//! `github_poll` and `plans` name no other slice.

use crate::kernel::state::{App, LoadedMap};
use crate::{
    approve, auth, companion_link, github_poll, launch, map, map_draft, new_issue, overview, pins,
    plans, queue,
};
use anyhow::{Result, anyhow};
use rubato_protocol::otel::event;
use rubato_protocol::{IssueRef, SessionStatus};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::{Id, JoinSet};

/// The map is refetched every this many seconds of poll intervals.
const MAP_RELOAD_SECS: u64 = 300;

/// A full tick at least this often (design 49 S4): comments, closed issues in repos the map does
/// not list, and the pull request look-up for a branch that has never had one.
pub const FULL_LANE_SECS: u64 = 300;

/// What a tick reads (design 49 S4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Everything.
    Full,
    /// No comments, no first pull request look-ups, no reads of settled issues.
    Fast,
}

impl Lane {
    fn name(self) -> &'static str {
        match self {
            Lane::Full => "full",
            Lane::Fast => "fast",
        }
    }
}

/// Which lane each tick of the loop takes: the first full, then full once `FULL_LANE_SECS` of
/// intervals have passed since the last full one.
pub struct LaneClock {
    secs: u64,
    since_full: Option<u64>,
}

impl LaneClock {
    pub fn new(interval_secs: u64) -> Self {
        Self {
            secs: interval_secs,
            since_full: None,
        }
    }

    pub fn next_lane(&mut self) -> Lane {
        match self.since_full {
            Some(s) if s < FULL_LANE_SECS => {
                self.since_full = Some(s + self.secs);
                Lane::Fast
            }
            _ => {
                self.since_full = Some(self.secs);
                Lane::Full
            }
        }
    }
}

/// The poll loop's memory between ticks, for the life of the process (design 49 D3): the issues
/// already given a full pass, and the plan path and branch head last fetched per issue. Losing
/// it costs one full pass.
#[derive(Default)]
pub struct Poller {
    seen: HashSet<IssueRef>,
    plan_heads: HashMap<IssueRef, (String, String)>,
    /// Closed issues whose last full pass has not yet succeeded.
    last_pass: HashSet<IssueRef>,
    /// Untracked open issues with an `issue/` branch: the plan path and head last looked at
    /// (ADR 0032). A head is read once; a head without a plan file costs nothing after that.
    discovered: HashMap<IssueRef, (String, String)>,
}

/// One full pass over every issue of interest, with nothing remembered from earlier ticks.
pub async fn poll_once(app: &App) -> Result<()> {
    Poller::default().tick(app, Lane::Full).await
}

impl Poller {
    /// One tick over every issue of interest.
    pub async fn tick(&mut self, app: &App, lane: Lane) -> Result<()> {
        let Some(map) = app.map.read().await.clone() else {
            tracing::warn!("no map loaded; skipping poll");
            return Ok(());
        };
        let mut issues: BTreeSet<IssueRef> = pins::list(&app.db)
            .await?
            .into_iter()
            .map(|p| p.issue)
            .collect();
        issues.extend(launch::launched_issues(&app.db).await?);
        let sessions = companion_link::all_sessions(&app.db).await?;
        let live: HashSet<IssueRef> = sessions
            .iter()
            .filter(|s| s.status != SessionStatus::Ended)
            .filter_map(|s| s.issue.clone())
            .collect();
        issues.extend(sessions.into_iter().filter_map(|s| s.issue));
        issues.extend(plans::all(&app.db).await?.into_iter().map(|p| p.issue));

        // Before the per-issue reads: every open issue of the map's repos (one listing per
        // repo), then, as an installation only, which repos it covers.
        let listed = github_poll::list_open_issues(app, &map.map).await;
        // Creations GitHub did not confirm, found by their `Rubato-Request:` line (ADR 0025).
        let found: Vec<new_issue::Found> = listed
            .requests()
            .into_iter()
            .map(|(issue, url, request_id)| new_issue::Found {
                issue,
                url,
                request_id,
            })
            .collect();
        new_issue::reconcile(app, &found, listed.fresh_repos()).await;
        // `Map:` lines of trusted authors pin their issues (ADR 0026). No GitHub call: the
        // bodies came with the listing. A newly pinned issue is read from the next tick on.
        let lines: Vec<pins::Line> = listed
            .map_lines()
            .iter()
            .map(|l| pins::Line {
                issue: l.issue.clone(),
                target: l.target.clone(),
                author: l.author.clone(),
            })
            .collect();
        pins::apply_map_lines(app, &map.map, &map.version, &lines).await;
        let mut repos: BTreeSet<String> = map
            .map
            .repos_named()
            .into_iter()
            .map(str::to_owned)
            .collect();
        repos.extend(issues.iter().map(|i| i.repo.clone()));
        github_poll::check_coverage(app, &repos).await;
        // As an installation, on the full lane: whether Issues: write is granted (ADR 0025), so a
        // re-approved installation is noticed within five minutes and no restart.
        if lane == Lane::Full
            && app.github.is_installation()
            && let Err(e) = app.github.refresh_issue_permission().await
        {
            tracing::warn!(error = %format!("{e:#}"), "reading the installation's permissions failed");
        }

        let mut view = github_poll::RepoView::default();
        let approved = github_poll::approved_branches(&app.db).await?;
        let stored: HashMap<IssueRef, String> = plans::all(&app.db)
            .await?
            .into_iter()
            .map(|p| (p.issue, p.branch))
            .collect();
        let (mut settled, mut failed) = (0usize, 0usize);
        for issue in &issues {
            let busy = live.contains(issue);
            let prefer = github_poll::BranchPrefs {
                approved: approved.get(issue).cloned(),
                stored: stored.get(issue).cloned(),
            };
            match self
                .poll_issue(app, &map, issue, busy, &listed, &mut view, lane, &prefer)
                .await
            {
                Ok(true) => settled += 1,
                Ok(false) => {}
                Err(e) => {
                    failed += 1;
                    tracing::warn!(issue = %issue, error = %e, "poll failed for issue");
                }
            }
        }
        let discovered = self
            .discover(app, &map, &issues, &listed, &mut view, &approved)
            .await;
        // Every open pull request of every map repo (ADR 0034): the listing the per-issue pass
        // already made is reused, so a repo is listed once per tick. Checks of pull requests no
        // tracked issue owns are read on the full lane while they are due.
        let owned: HashSet<(String, u64)> = github_poll::all_prs(&app.db)
            .await?
            .into_values()
            .map(|p| (p.repo, p.number))
            .collect();
        let pulls =
            github_poll::list_repo_pulls(app, &map.map, &mut view, &owned, lane == Lane::Full)
                .await;
        // After the comments pass: approvals posted from Rubato that GitHub did not confirm,
        // and the tokens of people with no session that still counts (ADR 0030).
        if lane == Lane::Full {
            approve::reconcile(app).await;
            if let Err(e) = auth::sweep_tokens(app).await {
                tracing::error!(error = %format!("{e:#}"), "sweeping user tokens failed");
            }
        }
        // Every tick: info on the full lane (every 300 s), debug on the fast one.
        let (tracked, repos) = (issues.len(), view.repos());
        if lane == Lane::Full {
            tracing::info!(
                lane = lane.name(),
                tracked,
                settled,
                failed,
                repos,
                discovered,
                pulls_listed = pulls.listed,
                pulls_failed = pulls.failed,
                pull_checks = pulls.checks_read,
                "poll tick"
            );
        } else {
            tracing::debug!(
                lane = lane.name(),
                tracked,
                settled,
                failed,
                repos,
                "poll tick"
            );
        }
        // After the per-issue reads and before the rate-limit read: the map proposal, if any.
        if map_draft::check_proposal(app).await {
            // Merged: load the map again now, so the draft can empty without waiting for the
            // next scheduled reload. A failed load keeps the old map and the draft stays.
            if let Err(e) = map::reload(app).await {
                tracing::error!(error = %e, "map reload after a merged proposal failed");
            }
            map_draft::clear_merged(app).await;
        }
        map_draft::record_gauges(app).await;
        queue::record_depth(app).await;
        github_poll::record_rate_limit(app).await;
        overview::refresh_gauges(app, &map).await?;
        app.notify("poll");
        Ok(())
    }

    /// Any error skips the rest of this issue; the caller logs it once. An issue not yet seen
    /// gets a full pass whatever the lane, and is remembered only once a pass succeeds; so is
    /// the last pass of a closed issue (design 49 §9.1). Returns whether the issue was settled.
    #[allow(clippy::too_many_arguments)]
    async fn poll_issue(
        &mut self,
        app: &App,
        map: &LoadedMap,
        issue: &IssueRef,
        live_session: bool,
        listed: &github_poll::Listed,
        view: &mut github_poll::RepoView,
        lane: Lane,
        prefer: &github_poll::BranchPrefs,
    ) -> Result<bool> {
        use github_poll::Standing;
        let first = !self.seen.contains(issue);
        let full = lane == Lane::Full || first;
        let mut standing = github_poll::read_meta(app, issue, listed, full).await?;
        if standing == Standing::Settled && self.last_pass.contains(issue) {
            standing = Standing::JustClosed;
        }
        // Closed, but work is still in flight: keep reading its branch and pull request.
        let busy = live_session || github_poll::has_open_pr(app, issue).await?;
        if standing == Standing::Settled && !first && !busy {
            tracing::debug!(rubato.issue = %issue, "settled: closed, not read");
            return Ok(true);
        }
        if standing == Standing::JustClosed {
            self.last_pass.insert(issue.clone());
        }
        let full = full || standing == Standing::JustClosed;
        let read_from = match view.branch(app, issue, prefer).await? {
            Some(head) => {
                let key = (plans::plan_path(map, issue, &head.name), head.sha.clone());
                if self.plan_heads.get(issue) != Some(&key) {
                    plans::fetch_and_store(app, map, issue, &head.name).await?;
                    self.plan_heads.insert(issue.clone(), key);
                }
                github_poll::poll_pr(app, issue, &head.name, view, full).await?;
                Some(head.name)
            }
            None => {
                github_poll::poll_pr_of_gone_branch(app, issue).await?;
                None
            }
        };
        if full {
            let read = github_poll::poll_comments(app, issue).await?;
            // An approval that names another branch is compared with the plan there: one file
            // read on the full lane (ADR 0027). The branch the plan is read from needs none.
            if let Some(a) = read.newest_approval
                && let Some(branch) = a.branch.as_deref()
                && read_from.as_deref() != Some(branch)
            {
                let sha = plans::hash_on_branch(app, map, issue, branch).await?;
                github_poll::set_target_sha(&app.db, issue, a.comment_id, sha.as_deref()).await?;
            }
        }
        self.seen.insert(issue.clone());
        if self.last_pass.remove(issue) && !busy {
            tracing::info!(rubato.issue = %issue, "issue closed; settled until it reopens");
        }
        Ok(false)
    }

    /// Plans of open issues nobody tracks yet (ADR 0032). For every repo the map names, its
    /// `issue/` refs (one listing per repo per tick, shared with the tracked issues); for each
    /// open listed issue that is not tracked and has a ref, the plan file is read once per head.
    /// A plan found makes the issue tracked from the next tick on (`plans::all`). No pull
    /// request, comment or checks call is made for an untracked issue. Returns how many plans
    /// were found this tick.
    async fn discover(
        &mut self,
        app: &App,
        map: &LoadedMap,
        tracked: &BTreeSet<IssueRef>,
        listed: &github_poll::Listed,
        view: &mut github_poll::RepoView,
        approved: &HashMap<IssueRef, String>,
    ) -> usize {
        let mut untracked: Vec<&IssueRef> = listed
            .open_issues()
            .filter(|i| !tracked.contains(*i))
            .collect();
        untracked.sort();
        let mut found = 0;
        for repo in map.map.repos_named() {
            let heads = view.heads(app, repo).await;
            for issue in untracked.iter().filter(|i| i.repo == repo) {
                if !heads
                    .iter()
                    .any(|b| IssueRef::number_from_branch(&b.name) == Some(issue.number))
                {
                    continue;
                }
                let prefer = github_poll::BranchPrefs {
                    approved: approved.get(*issue).cloned(),
                    stored: None,
                };
                let head = match view.branch(app, issue, &prefer).await {
                    Ok(Some(h)) => h,
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::warn!(rubato.issue = %issue, error = %e, "plan discovery failed");
                        continue;
                    }
                };
                let key = (plans::plan_path(map, issue, &head.name), head.sha.clone());
                if self.discovered.get(*issue) == Some(&key) {
                    continue;
                }
                match plans::fetch_and_store(app, map, issue, &head.name).await {
                    Ok(()) => {
                        self.discovered.insert((*issue).clone(), key);
                        if plans::has_plan(&app.db, issue).await.unwrap_or(false) {
                            found += 1;
                            tracing::info!(rubato.issue = %issue, branch = %head.name, "plan discovered");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(rubato.issue = %issue, error = %e, "plan discovery failed");
                    }
                }
            }
        }
        found
    }
}

/// The poll loop. Counts `rubato.poll.ticks` once per tick, so `ServerPollSilent` sees it stop.
pub async fn run(app: App, mut shutdown: watch::Receiver<bool>) {
    let mut map_age = 0u64;
    let mut poller = Poller::default();
    let mut lanes = LaneClock::new(app.config.limits.github_poll_interval_secs.max(5));
    loop {
        if map_age == 0
            && let Err(e) = map::reload(&app).await
        {
            tracing::error!(error = %e, "map reload failed");
        }
        if let Err(e) = poller.tick(&app, lanes.next_lane()).await {
            tracing::error!(error = %e, "poll failed");
        }
        app.metrics.record_poll_tick(&app.config.limits);
        let secs = app.config.limits.github_poll_interval_secs.max(5);
        map_age = (map_age + secs) % MAP_RELOAD_SECS;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
            _ = shutdown.changed() => return,
        }
    }
}

/// The server's background tasks: the poll loop, essential.
pub fn spawn(app: App, shutdown: watch::Receiver<bool>) -> Tasks {
    let mut tasks = Tasks::default();
    tasks.spawn("poll", Kind::Essential, async move {
        run(app, shutdown).await;
        Ok(())
    });
    tasks
}

// ----- supervision (design D12) --------------------------------------------------------------

/// An essential task ending while the process runs is fatal; a best-effort one is noted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Essential,
    BestEffort,
}

#[derive(Default)]
pub struct Tasks {
    set: JoinSet<Result<()>>,
    names: HashMap<Id, (&'static str, Kind)>,
}

impl Tasks {
    pub fn spawn(
        &mut self,
        name: &'static str,
        kind: Kind,
        fut: impl Future<Output = Result<()>> + Send + 'static,
    ) {
        let handle = self.set.spawn(fut);
        self.names.insert(handle.id(), (name, kind));
    }
}

/// A dropped sender counts as shutdown too.
fn shutdown_requested(rx: &watch::Receiver<bool>) -> bool {
    *rx.borrow() || rx.has_changed().is_err()
}

/// Await the tasks until shutdown. An essential task that returns or panics while shutdown is
/// not requested logs `rubato.task.exited` and returns `Err`. On shutdown every task is aborted
/// and `Cancelled` is clean.
pub async fn supervise(mut tasks: Tasks, mut shutdown: watch::Receiver<bool>) -> Result<()> {
    loop {
        if shutdown_requested(&shutdown) {
            tasks.set.abort_all();
            while let Some(r) = tasks.set.join_next_with_id().await {
                if let Err(e) = r
                    && !e.is_cancelled()
                {
                    let name = tasks.names.get(&e.id()).map_or("unknown", |n| n.0);
                    tracing::warn!(rubato.task = name, error = %e, "task failed during shutdown");
                }
            }
            return Ok(());
        }
        tokio::select! {
            _ = shutdown.changed() => {}
            Some(r) = tasks.set.join_next_with_id(), if !tasks.set.is_empty() => {
                let (id, error) = match r {
                    Ok((id, Ok(()))) => (id, "returned".to_owned()),
                    Ok((id, Err(e))) => (id, format!("{e:#}")),
                    Err(e) => (e.id(), e.to_string()),
                };
                let (name, kind) = tasks.names.remove(&id).unwrap_or(("unknown", Kind::Essential));
                if shutdown_requested(&shutdown) {
                    continue;
                }
                match kind {
                    Kind::Essential => {
                        tracing::error!(rubato.task = name, error = %error, "{}", event::TASK_EXITED);
                        tasks.set.abort_all();
                        return Err(anyhow!("essential task {name} exited: {error}"));
                    }
                    Kind::BestEffort => {
                        tracing::info!(rubato.task = name, "best-effort task ended");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;

    fn exited(events: &capture::Events) -> usize {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.message == event::TASK_EXITED)
            .count()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn essential_exit_while_running_is_an_error_with_one_event() {
        let (events, _g) = capture::start();
        let (_tx, rx) = watch::channel(false);
        let mut t = Tasks::default();
        t.spawn("poll", Kind::Essential, async { Err(anyhow!("boom")) });
        t.spawn("forever", Kind::Essential, std::future::pending());
        let r = tokio::time::timeout(Duration::from_secs(1), supervise(t, rx))
            .await
            .expect("supervise returns at once");
        assert!(format!("{:#}", r.unwrap_err()).contains("poll"));
        assert_eq!(exited(&events), 1);
        let ev = events.lock().unwrap();
        let e = ev.iter().find(|e| e.message == event::TASK_EXITED).unwrap();
        assert_eq!(
            e.fields.get("rubato.task").map(String::as_str),
            Some("poll")
        );
        assert_eq!(e.fields.get("error").map(String::as_str), Some("boom"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn essential_panic_is_an_error() {
        let (events, _g) = capture::start();
        let (_tx, rx) = watch::channel(false);
        let mut t = Tasks::default();
        t.spawn("poll", Kind::Essential, async { panic!("kaboom") });
        assert!(supervise(t, rx).await.is_err());
        assert_eq!(exited(&events), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn best_effort_exit_parks_without_an_event() {
        let (events, _g) = capture::start();
        let (tx, rx) = watch::channel(false);
        let mut t = Tasks::default();
        t.spawn("events", Kind::BestEffort, async { Ok(()) });
        t.spawn("forever", Kind::Essential, std::future::pending());
        let sup = tokio::spawn(supervise(t, rx));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !sup.is_finished(),
            "a best-effort exit does not end supervision"
        );
        tx.send(true).unwrap();
        let r = tokio::time::timeout(Duration::from_secs(1), sup).await;
        assert!(r.expect("Ok within 1 s").unwrap().is_ok());
        assert_eq!(exited(&events), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_aborts_tasks_that_never_return() {
        let (events, _g) = capture::start();
        let (tx, rx) = watch::channel(false);
        let mut t = Tasks::default();
        t.spawn("hooks", Kind::Essential, std::future::pending());
        let sup = tokio::spawn(supervise(t, rx));
        tx.send(true).unwrap();
        let r = tokio::time::timeout(Duration::from_secs(1), sup).await;
        assert!(r.expect("Ok within 1 s").unwrap().is_ok());
        assert_eq!(exited(&events), 0);
    }

    /// Design 49 S4: the first tick is full, then a full tick once 300 s of intervals have
    /// passed, whether or not the interval divides 300.
    #[test]
    fn the_full_lane_comes_by_elapsed_time() {
        let full_ticks = |secs: u64, n: usize| -> Vec<usize> {
            let mut c = LaneClock::new(secs);
            (0..n).filter(|_| c.next_lane() == Lane::Full).collect()
        };
        assert_eq!(full_ticks(60, 11), [0, 5, 10]);
        // 45 s: seven ticks (315 s) between full ones, never longer.
        assert_eq!(full_ticks(45, 15), [0, 7, 14]);
        assert_eq!(full_ticks(300, 3), [0, 1, 2]);
        println!("checked 3 intervals");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_dropped_sender_is_a_shutdown() {
        let (tx, rx) = watch::channel(false);
        let mut t = Tasks::default();
        t.spawn("hooks", Kind::Essential, std::future::pending());
        drop(tx);
        let r = tokio::time::timeout(Duration::from_secs(1), supervise(t, rx)).await;
        assert!(r.expect("Ok within 1 s").is_ok());
    }
}
