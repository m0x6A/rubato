//! Composition root: the periodic poll loop and the supervision of background tasks.
//!
//! The loop keeps its deadlines in a [`Cadence`] (design 208 D8). With webhooks on (ADR 0074) a
//! tick reads everything only on a catch-up, and a delivery wakes the loop for a refresh of what
//! it names ([`Poller::refresh`]).
//!
//! A pass keeps the order of design §9.1: reload the map when it is due, then the open
//! issues of the map's repos and the installation's coverage (issue #48), then per issue
//! `upsert_meta → find_branch → (plan, PR) when a branch exists → decisions`, then rate limit,
//! gauges and `notify("poll")`. The branch is looked up once here and passed to both slices, so
//! `github_poll` and `plans` name no other slice.

use crate::kernel::state::{App, LoadedMap};
use crate::{
    agent_reports, approve, auth, companion_link, github_poll, launch, map, map_draft, new_issue,
    overview, pairing, pins, plan_status, plans, pulls, queue, rooms, session_messages, traffic,
    webhooks,
};
use anyhow::{Result, anyhow};
use chrono::Utc;
use rubato_protocol::otel::event;
use rubato_protocol::{IssueRef, SessionStatus};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::{Id, JoinSet};

/// Fill the kernel's session cache from the stored snapshots, once at start, so a slice that
/// reads it (approval's "is the owner's session working") sees what the overview sees before
/// any companion reconnects.
pub async fn warm_sessions(app: &App) -> Result<()> {
    let snapshots = companion_link::session_snapshots(&app.db).await?;
    let n = snapshots.len();
    for (companion_id, sessions) in snapshots {
        app.set_sessions(companion_id, sessions);
    }
    tracing::info!(companions = n, "session snapshots loaded");
    Ok(())
}

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

/// The map is reloaded at most this often by the clock (and at once on a map-branch push).
const MAP_RELOAD_DUE_SECS: u64 = MAP_RELOAD_SECS;

/// After a webhook wake, wait this long to gather a burst (a push brings a check suite and
/// several check runs), design 208 A3.
pub const WAKE_GATHER_SECS: u64 = 2;

/// What is due at a moment of the loop (design 208 D8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub lane: Lane,
    /// Read everything. Always with webhooks off; every catch-up interval with them on.
    pub catchup: bool,
    pub map: bool,
}

/// The loop's deadlines in seconds of a monotonic clock (design 208 D8): the tick, the 300 s
/// full lane, the 300 s map reload and, with webhooks on, the catch-up. Kept by time, so a
/// webhook wake between ticks moves none of them. Pure: `run` and the tests drive it.
#[derive(Debug, Clone)]
pub struct Cadence {
    tick: u64,
    catchup: Option<u64>,
    next_tick: u64,
    next_full: u64,
    next_map: u64,
    next_catchup: u64,
}

impl Cadence {
    /// `catchup`: `None` with webhooks off (every tick reads everything), else its interval.
    pub fn new(tick_secs: u64, catchup: Option<u64>) -> Self {
        Self {
            tick: tick_secs.max(1),
            catchup,
            next_tick: 0,
            next_full: 0,
            next_map: 0,
            next_catchup: 0,
        }
    }

    /// What is due at `now`, or `None` before the next tick. Taking a tick moves its deadlines.
    pub fn at(&mut self, now: u64) -> Option<Due> {
        if now < self.next_tick {
            return None;
        }
        self.next_tick = now + self.tick;
        let catchup = match self.catchup {
            None => true,
            Some(c) if now >= self.next_catchup => {
                self.next_catchup = now + c;
                true
            }
            Some(_) => false,
        };
        let full = now >= self.next_full || (self.catchup.is_some() && catchup);
        if full {
            self.next_full = now + FULL_LANE_SECS;
        }
        let map = now >= self.next_map;
        if map {
            self.next_map = now + MAP_RELOAD_DUE_SECS;
        }
        Some(Due {
            lane: if full { Lane::Full } else { Lane::Fast },
            catchup,
            map,
        })
    }

    /// Seconds from `now` to the next tick.
    pub fn until_next(&self, now: u64) -> u64 {
        self.next_tick.saturating_sub(now)
    }
}

/// What a pass reads (design 208 D7).
#[derive(Debug, Clone, Copy)]
pub enum Scope<'a> {
    /// Everything: every tick with webhooks off, each catch-up with them on.
    All,
    /// What webhook deliveries name, and tracked issues not read since start.
    Targets(&'a [webhooks::Target]),
}

/// What one pass read: each issue `poll_issue` ran for, and whether it succeeded.
#[derive(Debug, Default)]
pub struct PassReport {
    pub read: HashMap<IssueRef, bool>,
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
    /// Count what a catch-up finds that no delivery brought (ADR 0074): on while webhooks are.
    count_changes: bool,
    /// Unix seconds of the newest such change.
    last_change_at: Option<u64>,
}

/// One full pass over every issue of interest, with nothing remembered from earlier ticks.
pub async fn poll_once(app: &App) -> Result<()> {
    Poller::default().tick(app, Lane::Full).await
}

/// What a catch-up compares for an issue before and after its read (design 208 D10).
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    issue: Option<(Option<String>, Option<String>)>,
    pull: Option<(u64, bool, bool, bool, rubato_protocol::ChecksStatus)>,
    plan: Option<(String, String)>,
}

/// Settle the rows a pass started from (design 208 D9): a row whose issue's read failed gets
/// another attempt; every other row is done.
async fn settle_pending(app: &App, pending: &[webhooks::Pending], report: &PassReport) {
    if pending.is_empty() {
        return;
    }
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for p in pending {
        match p.target.issue.as_ref().and_then(|i| report.read.get(i)) {
            Some(false) => failed.push((
                p.delivery_id.clone(),
                p.target
                    .issue
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
            )),
            _ => done.push(p.delivery_id.clone()),
        }
    }
    webhooks::settle(app, &done, &failed, report.read.len()).await;
}

/// Whether a target is a push to a workspace's map branch.
fn is_map_push(app: &App, t: &webhooks::Target) -> bool {
    let Some(branch) = t.pushed_branch.as_deref() else {
        return false;
    };
    app.workspace_entries()
        .0
        .iter()
        .any(|w| w.map_repo.eq_ignore_ascii_case(&t.repo) && w.map_ref == branch)
}

impl Poller {
    /// A poller for webhooks on: it counts what a catch-up finds that no delivery brought.
    pub fn counting_changes() -> Self {
        Self {
            count_changes: true,
            ..Self::default()
        }
    }

    /// Unix seconds of the newest change a catch-up found that no delivery brought (D10).
    pub fn last_change_at(&self) -> Option<u64> {
        self.last_change_at
    }

    /// One tick over every issue of interest; the rate limit is read last on every path, an
    /// early return or an error included (issue #208).
    pub async fn tick(&mut self, app: &App, lane: Lane) -> Result<()> {
        let r = async {
            if self.pass(app, lane, Scope::All).await?.is_some() {
                self.housekeeping(app, lane).await?;
            }
            Ok(())
        }
        .await;
        github_poll::record_rate_limit(app).await;
        r
    }

    /// One 60 s tick with webhooks on (design 208 D8): a catch-up reads everything; otherwise
    /// what pending deliveries name and the tracked issues not read since start. The rows read
    /// at the start are settled; housekeeping and the rate-limit read follow.
    pub async fn webhook_tick(&mut self, app: &App, lane: Lane, catchup: bool) -> Result<()> {
        let pending = webhooks::pending(app).await;
        let r = async {
            let targets: Vec<webhooks::Target> = pending.iter().map(|p| p.target.clone()).collect();
            let scope = if catchup {
                Scope::All
            } else {
                Scope::Targets(&targets)
            };
            let lane = if catchup { Lane::Full } else { lane };
            if let Some(report) = self.pass(app, lane, scope).await? {
                settle_pending(app, &pending, &report).await;
                self.housekeeping(app, lane).await?;
            }
            Ok(())
        }
        .await;
        app.metrics
            .catchup_last_change_at
            .record(self.last_change_at.unwrap_or(0), &[]);
        github_poll::record_rate_limit(app).await;
        r
    }

    /// A webhook wake (design 208 D6): only what the pending deliveries name, then the web is
    /// told. No housekeeping, no deadline moves.
    pub async fn refresh(&mut self, app: &App) -> Result<()> {
        let pending = webhooks::pending(app).await;
        if pending.is_empty() {
            return Ok(());
        }
        let targets: Vec<webhooks::Target> = pending.iter().map(|p| p.target.clone()).collect();
        if let Some(report) = self.pass(app, Lane::Fast, Scope::Targets(&targets)).await? {
            settle_pending(app, &pending, &report).await;
            app.notify("webhook");
        }
        Ok(())
    }

    /// The GitHub reads of one pass (design §9.1, design 208 D7). `None` when no map is loaded.
    async fn pass(
        &mut self,
        app: &App,
        lane: Lane,
        scope: Scope<'_>,
    ) -> Result<Option<PassReport>> {
        let targets = match scope {
            Scope::All => None,
            Scope::Targets(t) => Some(t),
        };
        // A push to a map branch: the maps first, so this pass reads with the new one.
        if targets.is_some_and(|t| t.iter().any(|t| is_map_push(app, t)))
            && let Err(e) = map::reload(app).await
        {
            tracing::error!(error = %e, "map reload after a map-branch push failed");
        }
        let Some(map) = app.default_map().await else {
            tracing::warn!("no map loaded; skipping poll");
            return Ok(None);
        };
        // Pinned in any workspace: each issue is read once, whatever views show it (ADR 0048).
        let mut issues: BTreeSet<IssueRef> = pins::list_all(&app.db)
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
        // The issues deliveries name, and whether one asks for the full lane.
        let mut named: HashMap<IssueRef, bool> = HashMap::new();
        for t in targets.unwrap_or_default() {
            if let Some(i) = &t.issue {
                *named.entry(i.clone()).or_default() |= t.full;
            }
        }
        let named_untracked: BTreeSet<IssueRef> = named
            .keys()
            .filter(|i| !issues.contains(*i))
            .cloned()
            .collect();

        // Before the per-issue reads: every open issue of the map's repos (one listing per
        // repo), then, as an installation only, which repos it covers.
        // Every loaded workspace map, in id order: the listing is over their union (ADR 0048).
        // Pointer copies under one guard, then borrowed: no map is copied (#171).
        let maps: Vec<(String, std::sync::Arc<LoadedMap>)> = app
            .maps
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), std::sync::Arc::clone(v)))
            .collect();
        let systems: Vec<(&str, &rubato_protocol::SystemMap)> =
            maps.iter().map(|(k, v)| (k.as_str(), &v.map)).collect();
        // With targets, only the repos they name, unless this is a full lane (every 300 s, so
        // the backlog and `Map:` lines keep their pace, design 208 D7).
        let list_only: Option<BTreeSet<String>> = match targets {
            Some(t) if lane == Lane::Fast => Some(
                t.iter()
                    .filter(|t| {
                        t.list_issues
                            || t.issue
                                .as_ref()
                                .is_some_and(|i| named_untracked.contains(i))
                    })
                    .map(|t| t.repo.to_ascii_lowercase())
                    .collect(),
            ),
            _ => None,
        };
        let listed = match &list_only {
            Some(only) if only.is_empty() => github_poll::Listed::default(),
            only => github_poll::list_open_issues_in(app, &systems, only.as_ref()).await,
        };
        // Creations GitHub did not confirm, found by their `Rubato-Request:` line (ADR 0025).
        let found: Vec<new_issue::Found> = listed
            .requests()
            .into_iter()
            .map(|r| new_issue::Found {
                issue: r.issue,
                url: r.url,
                request_id: r.request_id,
                author: r.author,
                author_is_bot: r.author_is_bot,
            })
            .collect();
        new_issue::reconcile(app, &found, listed.fresh_repos()).await;
        // `Map:` lines of trusted authors pin their issues (ADR 0026). No GitHub call: the
        // bodies came with the listing. A newly pinned issue is read from the next tick on.
        for (ws, m) in &maps {
            let lines: Vec<pins::Line> = listed
                .map_lines()
                .iter()
                .filter(|l| &l.workspace == ws)
                .map(|l| pins::Line {
                    issue: l.issue.clone(),
                    target: l.target.clone(),
                    author: l.author.clone(),
                })
                .collect();
            pins::apply_map_lines(app, ws, &m.map, &m.version, &lines).await;
        }
        // Every distinct repo of every workspace, once (ADR 0048), and the repos of tracked work.
        let mut repos: BTreeSet<String> = app.union_repos().await.into_iter().collect();
        repos.extend(issues.iter().map(|i| i.repo.clone()));
        *app.polled_repos.lock().unwrap() = repos.iter().map(|r| r.to_ascii_lowercase()).collect();
        if targets.is_none() || lane == Lane::Full {
            github_poll::check_coverage(app, &repos).await;
        }
        // As an installation, on the full lane: whether Issues: write is granted (ADR 0025), so a
        // re-approved installation is noticed within five minutes and no restart.
        if lane == Lane::Full
            && app.github.is_installation()
            && let Err(e) = app.github.refresh_issue_permission().await
        {
            tracing::warn!(error = %format!("{e:#}"), "reading the installation's permissions failed");
        }

        let mut view = github_poll::RepoView::default();
        // One list of counted approvals, comments under the cutover and Rubato approvals
        // (ADR 0054), built here so `github_poll` never names `approve`.
        let approvals = approve::approvals(app).await?;
        let approved = github_poll::approved_branches(&approvals.counted);
        let stored: HashMap<IssueRef, String> = plans::all(&app.db)
            .await?
            .into_iter()
            .map(|p| (p.issue, p.branch))
            .collect();
        // A catch-up with webhooks on counts what it finds for issues it has read before, in
        // repos the installation covers (GitHub sends webhooks for no other, design 208 D10).
        let counting = self.count_changes && targets.is_none();
        let uncovered: HashSet<String> = if counting {
            github_poll::uncovered(&app.db)
                .await?
                .into_iter()
                .map(|r| r.to_ascii_lowercase())
                .collect()
        } else {
            HashSet::new()
        };
        let mut report = PassReport::default();
        let (mut settled, mut failed) = (0usize, 0usize);
        for issue in &issues {
            let issue_lane = match targets {
                None => lane,
                Some(_) => match named.get(issue) {
                    Some(true) => Lane::Full,
                    Some(false) => Lane::Fast,
                    // Not read since start: its first pass, as on every tick before (D7).
                    None if !self.seen.contains(issue) => Lane::Fast,
                    None => continue,
                },
            };
            let busy = live.contains(issue);
            let prefer = github_poll::BranchPrefs {
                approved: approved.get(issue).map(String::as_str),
                stored: stored.get(issue).map(String::as_str),
            };
            let compare = counting
                && self.seen.contains(issue)
                && !uncovered.contains(&issue.repo.to_ascii_lowercase());
            let before = if compare {
                Some(self.seen_of(app, issue).await?)
            } else {
                None
            };
            match self
                .poll_issue(
                    app, &map, issue, busy, &listed, &mut view, issue_lane, &prefer,
                )
                .await
            {
                Ok(true) => {
                    settled += 1;
                    report.read.insert(issue.clone(), true);
                }
                Ok(false) => {
                    report.read.insert(issue.clone(), true);
                }
                Err(e) => {
                    failed += 1;
                    report.read.insert(issue.clone(), false);
                    tracing::warn!(issue = %issue, error = %e, "poll failed for issue");
                }
            }
            if let Some(before) = before {
                let after = self.seen_of(app, issue).await?;
                self.count_change(app, issue, &before, &after);
            }
        }
        let discovered = self
            .discover(
                app,
                &map,
                &issues,
                &listed,
                &mut view,
                &approved,
                targets.map(|_| &named_untracked),
            )
            .await;
        // Every open pull request of every map repo (ADR 0034): the listing the per-issue pass
        // already made is reused, so a repo is listed once per tick. Checks of pull requests no
        // tracked issue owns are read on the full lane while they are due. With targets, only
        // the repos a pull request or check delivery names, with their checks (D5).
        let pulls_only: Option<BTreeSet<String>> = targets.map(|t| {
            t.iter()
                .filter(|t| t.list_pulls)
                .map(|t| t.repo.to_ascii_lowercase())
                .collect()
        });
        let pulls = if pulls_only.as_ref().is_some_and(BTreeSet::is_empty) {
            github_poll::PullsPass::default()
        } else {
            let owned: HashSet<(String, u64)> = github_poll::all_prs(&app.db)
                .await?
                .into_values()
                .map(|p| (p.repo, p.number))
                .collect();
            github_poll::list_repo_pulls_in(
                app,
                &map.map,
                &mut view,
                &owned,
                lane == Lane::Full || pulls_only.is_some(),
                pulls_only.as_ref(),
            )
            .await
        };
        // Merges sent from Rubato that GitHub did not confirm, against the listings just made in
        // full (ADR 0033, plan 82 Decision 12). A delivery-scoped pass lists some repos whole.
        pulls::reconcile(app, &pulls.complete).await;
        // Every pass: info on the full lane (every 300 s) and for catch-ups, debug otherwise.
        let (tracked, repos) = (issues.len(), view.repos());
        let scope_name = if targets.is_some() { "targets" } else { "all" };
        if lane == Lane::Full {
            tracing::info!(
                lane = lane.name(),
                scope = scope_name,
                tracked,
                read = report.read.len(),
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
                scope = scope_name,
                tracked,
                read = report.read.len(),
                settled,
                failed,
                repos,
                "poll tick"
            );
        }
        Ok(Some(report))
    }

    /// The local work of a tick, after its GitHub reads: link comments, seam notices, room
    /// sweeps, the map proposal, gauges and the web fan-out. The rate-limit read is the caller's.
    async fn housekeeping(&mut self, app: &App, lane: Lane) -> Result<()> {
        // Every tick, after the plans and comments are read: handoff commits get their link
        // comment, found or posted once, and the handoff gauges are recorded (ADR 0055).
        approve::reconcile_links(app).await;
        // Every tick, after the plans are read: one notice per pair of issue and seam in the
        // rooms of two open plans that touch the same seam (ADR 0057).
        if let Err(e) = rooms::post_seam_notices(app).await {
            tracing::error!(error = %format!("{e:#}"), "posting seam notices failed");
        }
        // Agents not addressed for the idle limit, or whose session ended, leave their rooms.
        if let Err(e) = rooms::sweep_agents(app).await {
            tracing::error!(error = %format!("{e:#}"), "sweeping room agents failed");
        }
        // After the comments pass: approvals posted from Rubato that GitHub did not confirm,
        // and the tokens of people with no session that still counts (ADR 0030).
        if lane == Lane::Full {
            approve::reconcile(app).await;
            if let Err(e) = auth::sweep_tokens(app).await {
                tracing::error!(error = %format!("{e:#}"), "sweeping user tokens failed");
            }
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
        let sessions = companion_link::all_sessions(&app.db)
            .await
            .unwrap_or_default();
        session_messages::record_gauges(app, &sessions).await;
        // Lines of launches no companion reports any more (a revoked one sends no snapshot).
        if let Err(e) = agent_reports::prune_absent(&app.db, &app.latest_sessions()).await {
            tracing::error!(error = %format!("{e:#}"), "pruning agent reports failed");
        }
        webhooks::record(app).await;
        // #69: a companion silent with its socket open counts its sessions as unknown.
        companion_link::mark_quiet(app, app.clock.now());
        // The overviews built for the gauges also tell each launched session where its plan
        // stands (ADR 0071, plan 226 §4); one workspace failing stops neither.
        let refreshed = overview::refresh(app).await;
        plan_status::on_tick(app, &refreshed.built).await;
        if let Some(e) = refreshed.error {
            return Err(e);
        }
        app.notify("poll");
        Ok(())
    }

    /// What a catch-up compares for one issue (D10): stored title and state, the pull request's
    /// state, and the plan head last read. Database reads only.
    async fn seen_of(&self, app: &App, issue: &IssueRef) -> Result<Seen> {
        let meta = github_poll::issue_meta(&app.db, issue).await?;
        let pr = github_poll::pr_for(&app.db, issue).await?;
        Ok(Seen {
            issue: meta.map(|m| (m.title, m.github_state)),
            pull: pr.map(|p| (p.number, p.open, p.draft, p.merged_at.is_some(), p.checks)),
            plan: self.plan_heads.get(issue).cloned(),
        })
    }

    fn count_change(&mut self, app: &App, issue: &IssueRef, before: &Seen, after: &Seen) {
        let kinds = [
            ("issue", before.issue != after.issue),
            ("pull", before.pull != after.pull),
            ("plan", before.plan != after.plan),
        ];
        for (kind, changed) in kinds {
            if !changed {
                continue;
            }
            let now = u64::try_from(app.clock.now().timestamp()).unwrap_or(0);
            self.last_change_at = Some(now);
            app.metrics.catchup_changes.add(
                1,
                &[opentelemetry::KeyValue::new(
                    rubato_protocol::otel::attr::CHANGE_KIND,
                    kind,
                )],
            );
            tracing::info!(rubato.issue = %issue, kind, "catch-up found a change no delivery brought");
        }
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
        prefer: &github_poll::BranchPrefs<'_>,
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
                    plans::fetch_and_store(app, map, issue, &head.name, &head.sha).await?;
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
                if let Some(id) = a.comment_id {
                    github_poll::set_target_sha(&app.db, issue, id, sha.as_deref()).await?;
                }
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
    #[allow(clippy::too_many_arguments)]
    async fn discover(
        &mut self,
        app: &App,
        map: &LoadedMap,
        tracked: &BTreeSet<IssueRef>,
        listed: &github_poll::Listed,
        view: &mut github_poll::RepoView,
        approved: &HashMap<IssueRef, String>,
        only: Option<&BTreeSet<IssueRef>>,
    ) -> usize {
        let mut untracked: Vec<&IssueRef> = listed
            .open_issues()
            .filter(|i| !tracked.contains(*i))
            .filter(|i| only.is_none_or(|o| o.contains(*i)))
            .collect();
        untracked.sort();
        let mut found = 0;
        let union = app.union_repos().await;
        for repo in union.iter().map(String::as_str) {
            // A delivery scope lists refs only where it names an untracked issue (ADR 0074).
            if only.is_some() && !untracked.iter().any(|i| i.repo == repo) {
                continue;
            }
            let heads = view.head_issues(app, repo).await;
            for issue in untracked.iter().filter(|i| i.repo == repo) {
                if !heads.contains(&issue.number) {
                    continue;
                }
                let prefer = github_poll::BranchPrefs {
                    approved: approved.get(*issue).map(String::as_str),
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
                match plans::fetch_and_store(app, map, issue, &head.name, &head.sha).await {
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
/// Deadlines come from a [`Cadence`] on a monotonic clock. With webhooks on (ADR 0074) a
/// delivery wakes the loop between ticks for a refresh of what it names; a wake moves no
/// deadline and is not a tick.
pub async fn run(app: App, mut shutdown: watch::Receiver<bool>) {
    let start = tokio::time::Instant::now();
    let webhooks_on = app.config.webhooks_on();
    let mut cadence = Cadence::new(
        app.config.limits.github_poll_interval_secs.max(5),
        webhooks_on.then_some(app.config.github_catchup_secs),
    );
    let mut poller = if webhooks_on {
        Poller::counting_changes()
    } else {
        Poller::default()
    };
    loop {
        if let Some(due) = cadence.at(start.elapsed().as_secs()) {
            if due.map
                && let Err(e) = map::reload(&app).await
            {
                tracing::error!(error = %e, "map reload failed");
            }
            let ticked = if webhooks_on {
                poller.webhook_tick(&app, due.lane, due.catchup).await
            } else {
                poller.tick(&app, due.lane).await
            };
            if let Err(e) = ticked {
                tracing::error!(error = %e, "poll failed");
            }
            // Outside the tick, so a missing map or a failing GitHub call does not stop it (#168).
            match pairing::sweep_codes(&app.db, Utc::now()).await {
                Ok(0) => {}
                Ok(gone) => tracing::info!(gone, "pairing codes swept"),
                Err(e) => {
                    tracing::error!(error = %format!("{e:#}"), "sweeping pairing codes failed")
                }
            }
            app.metrics.record_poll_tick(&app.config.limits);
        }
        let wait = cadence.until_next(start.elapsed().as_secs());
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
            _ = app.webhook_wake.notified(), if webhooks_on => {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(WAKE_GATHER_SECS)) => {}
                    _ = shutdown.changed() => return,
                }
                if let Err(e) = poller.refresh(&app).await {
                    tracing::error!(error = %e, "webhook refresh failed");
                }
            }
            _ = shutdown.changed() => return,
        }
    }
}

/// The server's background tasks: the poll loop, essential; the traffic supervisor, essential
/// and always running (ADR 0075): it follows the workspace registry and runs one fetch task per
/// workspace with a source, so a source set at runtime starts without a restart.
pub fn spawn(app: App, shutdown: watch::Receiver<bool>) -> Tasks {
    let mut tasks = Tasks::default();
    tasks.spawn(
        "traffic",
        Kind::Essential,
        traffic::run(app.clone(), shutdown.clone()),
    );
    tasks.spawn("session_messages_sweep", Kind::Essential, {
        let (app, shutdown) = (app.clone(), shutdown.clone());
        async move {
            session_messages::run_sweep(app, shutdown).await;
            Ok(())
        }
    });
    // Right after a companion is welcomed, its launches' plan statuses (plan 226 §4).
    tasks.spawn("plan_status_welcome", Kind::BestEffort, {
        let (app, mut shutdown) = (app.clone(), shutdown.clone());
        async move {
            let mut welcomed = app.subscribe_welcomed();
            loop {
                tokio::select! {
                    _ = shutdown.changed() => return Ok(()),
                    got = welcomed.recv() => match got {
                        Ok(c) => plan_status::on_welcome(&app, c).await,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                    },
                }
            }
        }
    });
    // The context graph's timer, only with its flag on (ADR 0084). Essential: it never ends on
    // an error, so ending is a fault.
    if let Some(cfg) = app.config.context_graph.clone() {
        let (app, shutdown) = (app.clone(), shutdown.clone());
        tasks.spawn("context_graph", Kind::Essential, async move {
            let every = std::time::Duration::from_secs(u64::from(cfg.regen_secs));
            crate::context_graph::run(app, every, shutdown).await;
            Ok(())
        });
    }
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

    /// Design 208 D8, webhooks off: every tick reads everything; a full lane every 300 s and a
    /// map reload every 300 s, by the clock, as the loop counted them before.
    #[test]
    fn cadence_with_webhooks_off_is_every_tick() {
        let mut c = Cadence::new(60, None);
        let mut steps = 0;
        let (mut full, mut maps) = (Vec::new(), Vec::new());
        for now in 0..=660u64 {
            if let Some(d) = c.at(now) {
                steps += 1;
                assert!(d.catchup, "every tick reads everything at {now}");
                if d.lane == Lane::Full {
                    full.push(now);
                }
                if d.map {
                    maps.push(now);
                }
            }
        }
        assert_eq!(steps, 12, "a tick every 60 s");
        assert_eq!(full, [0, 300, 600]);
        assert_eq!(maps, [0, 300, 600]);
        println!("checked {steps} ticks");
    }

    /// Design 208 D8, webhooks on: ticks every 60 s, full lanes every 300 s, catch-ups every
    /// 900 s (each a full lane), map reloads every 300 s; asking between ticks (a wake) moves
    /// nothing.
    #[test]
    fn cadence_with_webhooks_on() {
        let mut c = Cadence::new(60, Some(900));
        let (mut ticks, mut catchups, mut full, mut maps) = (0, Vec::new(), Vec::new(), 0);
        let mut asked = 0;
        for now in 0..=3600u64 {
            // A burst of wakes in every second: each one asks, and only a due tick answers.
            for _ in 0..3 {
                asked += 1;
                if let Some(d) = c.at(now) {
                    ticks += 1;
                    if d.catchup {
                        catchups.push(now);
                        assert_eq!(d.lane, Lane::Full, "a catch-up is a full lane");
                    }
                    if d.lane == Lane::Full {
                        full.push(now);
                    }
                    if d.map {
                        maps += 1;
                    }
                }
            }
        }
        assert_eq!(ticks, 61);
        assert_eq!(catchups, [0, 900, 1800, 2700, 3600]);
        assert_eq!(full.len(), 13, "every 300 s: {full:?}");
        assert_eq!(maps, 13);
        assert_eq!(c.until_next(3600), 60);
        println!("checked {asked} asks, {ticks} ticks");
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
