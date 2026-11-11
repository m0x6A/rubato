//! Composition root: the periodic poll loop and the supervision of background tasks.
//!
//! The poll keeps the order of design §9.1: reload the map when `map_age == 0`, then the open
//! issues of the map's repos and the installation's coverage (issue #48), then per issue
//! `upsert_meta → find_branch → (plan, PR) when a branch exists → decisions`, then rate limit,
//! gauges and `notify("poll")`. The branch is looked up once here and passed to both slices, so
//! `github_poll` and `plans` name no other slice.

use crate::kernel::state::{App, LoadedMap};
use crate::{companion_link, github_poll, launch, map, overview, pins, plans};
use anyhow::{Result, anyhow};
use rubato_protocol::IssueRef;
use rubato_protocol::otel::event;
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::{Id, JoinSet};

/// The map is refetched every this many seconds of poll intervals.
const MAP_RELOAD_SECS: u64 = 300;

/// One pass over every issue of interest.
pub async fn poll_once(app: &App) -> Result<()> {
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
    issues.extend(
        companion_link::all_sessions(&app.db)
            .await?
            .into_iter()
            .filter_map(|s| s.issue),
    );
    issues.extend(plans::all(&app.db).await?.into_iter().map(|p| p.issue));

    // Before the per-issue reads: every open issue of the map's repos (one listing per repo),
    // then, as an installation only, which repos it covers.
    github_poll::list_open_issues(app, &map.map).await;
    let mut repos: BTreeSet<String> = map
        .map
        .repos_named()
        .into_iter()
        .map(str::to_owned)
        .collect();
    repos.extend(issues.iter().map(|i| i.repo.clone()));
    github_poll::check_coverage(app, &repos).await;

    for issue in &issues {
        if let Err(e) = poll_issue(app, &map, issue).await {
            tracing::warn!(issue = %issue, error = %e, "poll failed for issue");
        }
    }
    github_poll::record_rate_limit(app).await;
    overview::refresh_gauges(app, &map).await?;
    app.notify("poll");
    Ok(())
}

/// Any error skips the rest of this issue; the caller logs it once.
async fn poll_issue(app: &App, map: &LoadedMap, issue: &IssueRef) -> Result<()> {
    github_poll::upsert_meta(app, issue).await?;
    if let Some(branch) = github_poll::find_branch(app, issue).await? {
        plans::fetch_and_store(app, map, issue, &branch).await?;
        github_poll::poll_pr(app, issue, &branch).await?;
    }
    github_poll::poll_decisions(app, issue).await
}

/// The poll loop. Counts `rubato.poll.ticks` once per tick, so `ServerPollSilent` sees it stop.
pub async fn run(app: App, mut shutdown: watch::Receiver<bool>) {
    let mut map_age = 0u64;
    loop {
        if map_age == 0
            && let Err(e) = map::reload(&app).await
        {
            tracing::error!(error = %e, "map reload failed");
        }
        if let Err(e) = poll_once(&app).await {
            tracing::error!(error = %e, "poll failed");
        }
        app.metrics.poll_ticks.add(1, &[]);
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
