//! Per-issue reads of GitHub: issue metadata, the issue branch, its PR and checks, `Decision:`
//! (ADR 0014), `Review:` and `Approved:` (ADR 0027) comments from members (ADR 0065); and the rate
//! limit. `tasks::Poller` calls these in order (design §9.1).
//!
//! Issue #49 (ADR 0023, proposed): branches and open pull requests are listed once per repo per
//! tick through a [`RepoView`]; title and state come from the open-issues listing when the issue
//! is in it; a closed issue is settled until it is listed open again.

use super::backlog::Listed;
use super::store::{self, IssueMeta};
use crate::kernel::github::{BranchHead, OpenPulls, PullInfo};
use crate::kernel::state::App;
use anyhow::{Result, anyhow};
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{
    ApprovalRecord, ApprovalSource, ChecksStatus, DecisionRecord, IssueRef, PullRequestState,
    ReviewRecord, trusted_author,
};
use std::collections::{HashMap, HashSet};

/// Where an issue stands after its metadata step this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Open, or its state is not known: read it as usual.
    Active,
    /// Seen closed for the first time this tick: one last full pass.
    JustClosed,
    /// Closed, and known closed before this tick: no calls of its own.
    Settled,
}

const CLOSED: &str = "closed";

/// Title, URL and state from the listing when the issue is in it; otherwise one `issue` call,
/// unless the issue is already known closed (fresh listing) or this is a fast tick in a repo
/// that was not listed whole (design 49 S2, S6).
pub async fn read_meta(
    app: &App,
    issue: &IssueRef,
    listed: &Listed,
    full: bool,
) -> Result<Standing> {
    let t = &app.repo_tenant(&issue.repo)?;
    let stored = store::issue_meta(&app.db, t, issue).await?;
    if let Some(info) = listed.open(issue) {
        let meta = IssueMeta {
            title: Some(info.title.clone()),
            url: Some(info.url.clone()),
            github_state: Some("open".into()),
        };
        if stored.as_ref() != Some(&meta) {
            if stored
                .as_ref()
                .is_some_and(|m| m.github_state.as_deref() == Some(CLOSED))
            {
                tracing::info!(rubato.issue = %issue, "issue reopened; polled again");
            }
            store::upsert_issue_meta(&app.db, t, issue, &meta).await?;
        }
        return Ok(Standing::Active);
    }
    let was_closed = stored
        .as_ref()
        .is_some_and(|m| m.github_state.as_deref() == Some(CLOSED));
    let read_now = if listed.is_fresh(&issue.repo) {
        // Not in a whole listing: closed (or gone). Ask once, when that is news.
        !was_closed
    } else {
        full || stored.is_none()
    };
    if !read_now {
        return Ok(if was_closed {
            Standing::Settled
        } else {
            Standing::Active
        });
    }
    let closed = match app
        .github
        .unscoped(&issue.repo)
        .issue(&issue.repo, issue.number)
        .await?
    {
        Some(info) => {
            let closed = info.state == CLOSED;
            store::upsert_issue_meta(
                &app.db,
                t,
                issue,
                &IssueMeta {
                    title: Some(info.title),
                    url: Some(info.url),
                    github_state: Some(info.state),
                },
            )
            .await?;
            closed
        }
        None => false,
    };
    Ok(match (closed, was_closed) {
        (true, false) => Standing::JustClosed,
        (true, true) => Standing::Settled,
        (false, _) => Standing::Active,
    })
}

/// Which of several branches of one issue a caller already knows to prefer (ADR 0032).
#[derive(Debug, Clone, Copy, Default)]
pub struct BranchPrefs<'a> {
    /// The branch the newest `Approved:` names.
    pub approved: Option<&'a str>,
    /// The branch the stored plan was read from.
    pub stored: Option<&'a str>,
}

/// The listings of one tick, per repo, made on first use and shared by that repo's issues. A
/// failed listing is kept too, so it is not retried within the tick.
#[derive(Default)]
pub struct RepoView {
    branches: HashMap<String, Result<Vec<BranchHead>, String>>,
    /// The open pull request listing as GitHub answered it, cut or not, or why it failed. Shared
    /// by the per-issue pass and the per-repo listing (ADR 0034); a cut or failed listing makes
    /// issues fall back to `pull_for_branch`.
    pulls: HashMap<String, Result<OpenPulls, String>>,
}

impl RepoView {
    /// How many repos this tick listed anything for.
    pub fn repos(&self) -> usize {
        self.branches.len().max(self.pulls.len())
    }

    /// The branch `issue/<n>-<slug>` the plan is read from, with its head, if one exists. With
    /// several (a stack), in order (ADR 0032): the branch the newest `Approved:` names; the
    /// branch whose open pull request merges into a branch outside `issue/` (the bottom of the
    /// stack, into the default branch); the branch the stored plan was read from; the first by
    /// name.
    pub async fn branch(
        &mut self,
        app: &App,
        issue: &IssueRef,
        prefer: &BranchPrefs<'_>,
    ) -> Result<Option<BranchHead>> {
        if !self.branches.contains_key(&issue.repo) {
            let listed = app
                .github
                .unscoped(&issue.repo)
                .branches(&issue.repo, "issue/")
                .await
                .map_err(|e| format!("{e:#}"));
            self.branches.insert(issue.repo.clone(), listed);
        }
        let candidates: Vec<BranchHead> = match &self.branches[&issue.repo] {
            Ok(heads) => heads
                .iter()
                .filter(|b| IssueRef::number_from_branch(&b.name) == Some(issue.number))
                .cloned()
                .collect(),
            Err(e) => return Err(anyhow!("branch listing failed: {e}")),
        };
        if candidates.len() <= 1 {
            return Ok(candidates.into_iter().next());
        }
        let named = |name: Option<&str>| {
            name.and_then(|n| candidates.iter().find(|b| b.name == n).cloned())
        };
        if let Some(b) = named(prefer.approved) {
            return Ok(Some(b));
        }
        for b in &candidates {
            if let Some(Some(pr)) = self.open_pull(app, &issue.repo, &b.name).await
                && pr
                    .base
                    .as_deref()
                    .is_some_and(|base| !base.starts_with("issue/"))
            {
                return Ok(Some(b.clone()));
            }
        }
        Ok(named(prefer.stored).or_else(|| candidates.into_iter().next()))
    }

    /// The issue numbers the `issue/` branches of a repo name, read on first use (one listing
    /// per repo per tick, shared with [`Self::branch`]). A set, not a copy of every branch
    /// (#171): the caller only asks whether an issue has one.
    pub async fn head_issues(&mut self, app: &App, repo: &str) -> HashSet<u64> {
        if !self.branches.contains_key(repo) {
            let listed = app
                .github
                .unscoped(repo)
                .branches(repo, "issue/")
                .await
                .map_err(|e| format!("{e:#}"));
            self.branches.insert(repo.to_owned(), listed);
        }
        match &self.branches[repo] {
            Ok(heads) => heads
                .iter()
                .filter_map(|b| IssueRef::number_from_branch(&b.name))
                .collect(),
            Err(_) => HashSet::new(),
        }
    }

    /// `Some(pr)` when the open listing has a pull request from `branch`, `Some(None)` when the
    /// listing is whole and has none, `None` when there is no usable listing.
    async fn open_pull(&mut self, app: &App, repo: &str, branch: &str) -> Option<Option<PullInfo>> {
        match self.pull_listing(app, repo).await {
            Ok(l) if !l.truncated => Some(
                l.pulls
                    .iter()
                    .find(|(b, _)| b == branch)
                    .map(|(_, pr)| pr.clone()),
            ),
            _ => None,
        }
    }

    /// The repo's open pull request listing, made on first use this tick (one `pulls` call per
    /// repo per tick, ADR 0034).
    pub async fn pull_listing(&mut self, app: &App, repo: &str) -> &Result<OpenPulls, String> {
        if !self.pulls.contains_key(repo) {
            let listing = app
                .github
                .unscoped(repo)
                .open_pulls(repo)
                .await
                .map_err(|e| format!("{e:#}"));
            match &listing {
                Ok(l) if l.truncated => {
                    tracing::warn!(rubato.repo = %repo, "pull listing cut; reading pull requests one by one");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(rubato.repo = %repo, error = %e, "pull listing failed"),
            }
            self.pulls.insert(repo.to_owned(), listing);
        }
        &self.pulls[repo]
    }
}

/// Whether the stored pull request of this issue is still open: a closed issue with an open
/// pull request is not settled (design 49 S6).
pub async fn has_open_pr(app: &App, issue: &IssueRef) -> Result<bool> {
    let t = &app.repo_tenant(&issue.repo)?;
    Ok(store::pr_for(&app.db, t, issue)
        .await?
        .is_some_and(|p| p.open && p.merged_at.is_none()))
}

/// The pull request step for an issue whose branch is not in the listing: a stored open pull
/// request is looked up by its own branch, so a merge with the branch deleted still reaches
/// *Done* (design 49 S5).
pub async fn poll_pr_of_gone_branch(app: &App, issue: &IssueRef) -> Result<()> {
    let t = &app.repo_tenant(&issue.repo)?;
    let Some(stored) = store::pr_for(&app.db, t, issue).await? else {
        return Ok(());
    };
    if !stored.open {
        return Ok(());
    }
    let branch = stored.branch.clone();
    let pr = app
        .github
        .unscoped(&issue.repo)
        .pull_for_branch(&issue.repo, &branch)
        .await?;
    store_pr(app, issue, &branch, pr).await
}

/// The newest PR for the branch; checks only while it is not merged. From the open listing when
/// it has the branch; otherwise one `pull_for_branch` when the stored PR was open (it just
/// closed or merged), when there is no usable listing, or on a full pass. A stored merged or
/// closed PR is final until the branch shows up in the open listing again (design 49 S5).
pub async fn poll_pr(
    app: &App,
    issue: &IssueRef,
    branch: &str,
    view: &mut RepoView,
    full: bool,
) -> Result<()> {
    let t = &app.repo_tenant(&issue.repo)?;
    let pr = match view.open_pull(app, &issue.repo, branch).await {
        Some(Some(pr)) => Some(pr),
        listing => {
            let stored = store::pr_for(&app.db, t, issue).await?;
            let look_up = match &stored {
                _ if listing.is_none() => true,
                Some(s) if s.branch == branch && !s.open => false,
                Some(s) if s.open => true,
                _ => full,
            };
            if !look_up {
                return Ok(());
            }
            app.github
                .unscoped(&issue.repo)
                .pull_for_branch(&issue.repo, branch)
                .await?
        }
    };
    store_pr(app, issue, branch, pr).await
}

async fn store_pr(app: &App, issue: &IssueRef, branch: &str, pr: Option<PullInfo>) -> Result<()> {
    let t = &app.repo_tenant(&issue.repo)?;
    if let Some(pr) = pr {
        let checks = if pr.merged_at.is_some() {
            ChecksStatus::Green
        } else {
            app.github
                .unscoped(&issue.repo)
                .checks(&issue.repo, &pr.head_sha)
                .await?
        };
        store::upsert_pr(
            &app.db,
            t,
            issue,
            &PullRequestState {
                repo: issue.repo.clone(),
                number: pr.number,
                branch: branch.to_owned(),
                open: pr.open,
                draft: pr.draft,
                checks,
                merged_at: pr.merged_at,
                updated_at: pr.updated_at,
                url: pr.url,
            },
        )
        .await?;
    }
    Ok(())
}

/// What one read of an issue's comments found besides decisions: the newest counted approval,
/// so the caller can compare it with the plan on the branch it names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommentsRead {
    pub newest_approval: Option<ApprovalRecord>,
}

/// One pass over an issue's comments (one call, on the full lane): `Decision:` (ADR 0014),
/// `Review:` and `Approved:` (ADR 0027) comments, each counted only from members (ADR 0065). One
/// from a Bot or from someone who is not OWNER, MEMBER or COLLABORATOR is ignored and logged, and
/// a decision stored for it earlier is deleted. Signals whose comment is gone are dropped.
///
/// An `Approved:` comment posted at or after the cutover is stored, so the overview can show
/// it as `CommentApprovalIgnored`, but it is never the newest counted approval; the first read
/// of one counts on `rubato.plans.comment_after_cutover` (ADR 0054).
pub async fn poll_comments(app: &App, issue: &IssueRef) -> Result<CommentsRead> {
    let t = &app.repo_tenant(&issue.repo)?;
    let cutover = app.config.comment_approvals_until;
    let mut read = CommentsRead::default();
    let mut kept: Vec<u64> = Vec::new();
    for c in app
        .github
        .unscoped(&issue.repo)
        .issue_comments(&issue.repo, issue.number)
        .await?
    {
        let review = ReviewRecord::parse_body(&c.body);
        let approval = ApprovalRecord::parse_body(&c.body);
        let decision = DecisionRecord::parse_body(&c.body);
        if (review.is_some() || approval.is_some() || decision.is_some())
            && !trusted_author(&c.author_association, c.author_is_bot)
        {
            let kind = if review.is_some() {
                "review"
            } else if approval.is_some() {
                "approval"
            } else {
                "decision"
            };
            let reason = if c.author_is_bot {
                "bot".to_owned()
            } else {
                format!("association {}", c.author_association)
            };
            tracing::info!(
                rubato.issue = %issue,
                comment_id = c.id,
                author = %c.author,
                kind,
                reason = %reason,
                "{kind} ignored"
            );
            if decision.is_some() {
                // A row an older server stored before the author was checked (#164).
                store::delete_decision(&app.db, t, issue, c.id).await?;
            }
            continue;
        }
        if let Some(r) = review {
            let rec = ReviewRecord {
                issue: issue.clone(),
                comment_id: c.id,
                author: c.author.clone(),
                track: r.track,
                outcome: r.outcome,
                plan_sha: r.plan_sha,
                left_open: r.left_open,
                created_at: c.created_at,
                url: c.url.clone(),
            };
            store::upsert_signal(
                &app.db,
                t,
                issue,
                c.id,
                "review",
                &serde_json::to_string(&rec)?,
            )
            .await?;
            kept.push(c.id);
            continue;
        }
        if let Some(a) = approval {
            let rec = ApprovalRecord {
                issue: issue.clone(),
                source: ApprovalSource::Comment,
                comment_id: Some(c.id),
                approval_id: None,
                bundle_hash: None,
                author: c.author.clone(),
                sha: a.sha,
                branch: a.branch,
                note: a.note,
                created_at: c.created_at,
                url: c.url.clone(),
            };
            let new = store::upsert_signal(
                &app.db,
                t,
                issue,
                c.id,
                "approval",
                &serde_json::to_string(&rec)?,
            )
            .await?;
            kept.push(c.id);
            if !rec.counts(cutover) {
                if new {
                    app.metrics.plans_comment_after_cutover.add(1, &[]);
                    tracing::info!(
                        rubato.issue = %issue,
                        comment_id = c.id,
                        author = %c.author,
                        "approval comment after the cutover not counted"
                    );
                }
                continue;
            }
            if read
                .newest_approval
                .as_ref()
                .is_none_or(|n| (n.created_at, n.comment_id) < (rec.created_at, rec.comment_id))
            {
                read.newest_approval = Some(rec);
            }
            continue;
        }
        // A link comment (ADR 0055): kept whoever wrote it; `approve` matches it to the
        // approver, so nobody else's comment settles a link.
        if let Some(approval_id) = super::links::link_approval(&c.body) {
            let rec = super::links::LinkComment {
                issue: issue.clone(),
                comment_id: c.id,
                author: c.author.clone(),
                approval_id,
                url: c.url.clone(),
                created_at: c.created_at,
            };
            store::upsert_signal(
                &app.db,
                t,
                issue,
                c.id,
                "link",
                &serde_json::to_string(&rec)?,
            )
            .await?;
            kept.push(c.id);
            continue;
        }
        if let Some(body) = decision {
            store::upsert_decision(
                &app.db,
                t,
                &DecisionRecord {
                    issue: issue.clone(),
                    comment_id: c.id,
                    author: c.author,
                    title: body.title,
                    context: body.context,
                    options: body.options,
                    chosen: body.chosen,
                    why: body.why,
                    created_at: c.created_at,
                    url: c.url,
                },
            )
            .await?;
        }
    }
    store::drop_signals_except(&app.db, t, issue, &kept).await?;
    Ok(read)
}

/// Each installation's own budget (design §5.3): every installation has its own, so the gauge
/// carries `rubato.github.installation` (absent with one context) and `rubato.tenant`.
pub async fn record_rate_limit(app: &App) {
    for live in app.github.live() {
        let mut labels = vec![KeyValue::new(attr::TENANT, live.tenant.to_string())];
        if let Some(id) = live.installation {
            labels.push(KeyValue::new(attr::GITHUB_INSTALLATION, id as i64));
        }
        let installation = live.installation.unwrap_or(0);
        match live.client.rate_limit().await {
            Ok((remaining, limit)) => {
                app.metrics.rate_limit_remaining.record(remaining, &labels);
                app.metrics.rate_limit_limit.record(limit, &labels);
                if limit > 0 && remaining * 10 < limit {
                    tracing::warn!(
                        remaining,
                        limit,
                        rubato.github.installation = installation,
                        rubato.tenant = %live.tenant,
                        "GitHub rate limit under 10%"
                    );
                }
            }
            // Not debug: a gauge that stops updating keeps its last value, and
            // GitHubRateLimitLow would stay quiet on it (issue #208).
            Err(e) => tracing::warn!(
                rubato.github.installation = installation,
                rubato.tenant = %live.tenant,
                error = %format!("{e:#}"),
                "rate limit read failed"
            ),
        }
    }
}
