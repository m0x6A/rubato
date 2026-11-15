//! Everything the view and the merge rule need about one pull request, read through the port
//! (ADR 0034's per-view budget: pull, reviews, one page of files, check runs at the head; check
//! runs at the base only when the head has none; the plan file only when the branch names an
//! issue; and the repo as the person), plus the derived approval from the overview.

use super::cache::{Cache, Key, Outcome};
use crate::auth::{self, TokenError};
use crate::kernel::github::{PullFull, RepoAccess, ReviewInfo, summarize_check_runs};
use crate::kernel::state::{App, LoadedMap};
use crate::{github_poll, overview, plans};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::pulls::{
    ChangedFile, CheckRun, PersonFacts, PlanAtHead, PlanGate, PullFacts, PullState, plan_gate,
};
use rubato_protocol::{ChecksStatus, IssueRef, plan_fit, trusted_author};

/// The plan file at the head as read; parsed per request, since fit depends on the map.
#[derive(Debug, Clone)]
pub enum RawPlan {
    /// The branch names no issue, or the head has no plan file.
    Absent,
    /// Reading it failed (logged): the gate closes (plan 82 Decision 9).
    Unreadable,
    Text {
        path: String,
        text: String,
    },
}

/// What the installation read for one pull request.
pub struct Fetched {
    pub pull: PullFull,
    pub reviews: Vec<ReviewInfo>,
    pub files: Vec<ChangedFile>,
    pub runs: Vec<CheckRun>,
    /// Folded runs at the base, read only when the head has none.
    pub base_checks: Option<ChecksStatus>,
    pub plan: RawPlan,
    pub read_at: DateTime<Utc>,
}

/// The installation's part, the person's part and the gate, for one request.
pub struct Facts {
    fetched: std::sync::Arc<Option<Fetched>>,
    pub outcome: Outcome,
    pub person: Option<PersonFacts>,
    pub gate: PlanGate,
    pub issue: Option<IssueRef>,
    pub stacked_above: u32,
}

pub enum Gathered {
    /// GitHub has no such pull request.
    NotFound,
    Found(Box<Facts>),
}

pub type ViewCache = Cache<Option<Fetched>>;

/// Read (or reuse) the facts of `repo#number` for `login` in workspace `ws`. `fresh` bypasses
/// the cache and replaces its entry.
pub async fn gather(
    app: &App,
    ws: &str,
    map: &LoadedMap,
    repo: &str,
    number: u64,
    login: &str,
    fresh: bool,
) -> Result<Gathered> {
    let cache = app.slots.get::<ViewCache>();
    let key: Key = (ws.to_owned(), repo.to_owned(), number);
    let (fetched, outcome) = cache
        .get(key, fresh, || fetch(app, map, repo, number))
        .await?;
    if fetched.is_none() {
        return Ok(Gathered::NotFound);
    }
    let pull = &fetched.as_ref().as_ref().expect("checked above").pull;
    let person = person(app, &cache, login, repo, fresh).await;
    let issue =
        IssueRef::number_from_branch(&pull.branch).map(|n| IssueRef::new(repo.to_owned(), n));
    let summary = match &issue {
        Some(i) => overview::issue_summary(app, ws, i).await?,
        None => None,
    };
    let signal = summary.as_ref().is_some_and(|s| {
        s.plan_sha.is_some()
            || s.plan.is_some()
            || s.draft.is_some()
            || s.review.is_some()
            || s.approval.is_some()
    });
    let approval = summary
        .as_ref()
        .and_then(|s| s.approval.as_ref())
        .map(|a| &a.record);
    let head = match &fetched.as_ref().as_ref().expect("checked above").plan {
        RawPlan::Absent => PlanAtHead::Absent,
        RawPlan::Unreadable => PlanAtHead::Unreadable,
        RawPlan::Text { path, text } => {
            let parsed = plans::parse_plan(text, path).ok();
            PlanAtHead::Present {
                hash: plans::approval_hash(text),
                fits: parsed
                    .as_ref()
                    .is_some_and(|p| plan_fit(Some(p), None, &map.map).is_some_and(|f| f.fits)),
                record: parsed.and_then(|p| p.approval),
            }
        }
    };
    let gate = plan_gate(signal, &head, approval);
    let stacked_above = github_poll::repo_pulls(&app.db)
        .await?
        .iter()
        .filter(|r| r.repo.eq_ignore_ascii_case(repo))
        .flat_map(|r| &r.pulls)
        .filter(|p| p.base == pull.branch)
        .count() as u32;
    Ok(Gathered::Found(Box::new(Facts {
        fetched,
        outcome,
        person,
        gate,
        issue,
        stacked_above,
    })))
}

impl Facts {
    pub fn fetched(&self) -> &Fetched {
        self.fetched
            .as_ref()
            .as_ref()
            .expect("a found pull request")
    }

    pub fn pull_facts(&self) -> PullFacts {
        let p = &self.fetched().pull;
        PullFacts {
            state: state_of(p),
            fork: p.fork,
            draft: p.info.draft,
            head_sha: p.info.head_sha.clone(),
            mergeable: p.mergeable,
            checks: self.checks(),
            base_checks: self.fetched().base_checks,
        }
    }

    pub fn checks(&self) -> ChecksStatus {
        fold(&self.fetched().runs)
    }
}

pub fn state_of(p: &PullFull) -> PullState {
    if p.info.merged_at.is_some() {
        PullState::Merged
    } else if p.info.open {
        PullState::Open
    } else {
        PullState::Closed
    }
}

fn fold(runs: &[CheckRun]) -> ChecksStatus {
    summarize_check_runs(
        runs.iter()
            .map(|r| (r.status.as_str(), r.conclusion.as_deref())),
    )
}

/// The installation's reads. `None` when GitHub has no such pull request.
async fn fetch(app: &App, map: &LoadedMap, repo: &str, number: u64) -> Result<Option<Fetched>> {
    let gh = &app.github;
    let Some(pull) = gh.pull(repo, number).await? else {
        return Ok(None);
    };
    let head = pull.info.head_sha.clone();
    let (reviews, files, runs) = tokio::join!(
        gh.pull_reviews(repo, number),
        gh.pull_files(repo, number),
        gh.check_runs(repo, &head),
    );
    let (reviews, files, runs) = (reviews?, files?, runs?);
    let base_checks = if runs.is_empty() && !pull.base_sha.is_empty() {
        Some(fold(&gh.check_runs(repo, &pull.base_sha).await?))
    } else {
        None
    };
    let plan = match IssueRef::number_from_branch(&pull.branch) {
        None => RawPlan::Absent,
        Some(n) => {
            let issue = IssueRef::new(repo.to_owned(), n);
            // The plan row's path when Rubato holds one (plan 82 Decision 10).
            let path = plans::all(&app.db)
                .await?
                .into_iter()
                .find(|p| p.issue == issue)
                .map(|p| p.path)
                .unwrap_or_else(|| plans::plan_path(map, &issue, &pull.branch));
            match gh.file(repo, &path, &head).await {
                Ok(Some(text)) => RawPlan::Text { path, text },
                Ok(None) => RawPlan::Absent,
                Err(e) => {
                    tracing::warn!(rubato.repo = repo, rubato.pr = number, error = %format!("{e:#}"), "plan at the pull request head unreadable");
                    RawPlan::Unreadable
                }
            }
        }
    };
    Ok(Some(Fetched {
        pull,
        reviews,
        files,
        runs,
        base_checks,
        plan,
        read_at: Utc::now(),
    }))
}

/// The person's permission and the repo's merge methods, with their token; `None` without a
/// usable token or when the read fails (logged): the offer then says why.
async fn person(
    app: &App,
    cache: &ViewCache,
    login: &str,
    repo: &str,
    fresh: bool,
) -> Option<PersonFacts> {
    if !fresh && let Some(a) = cache.person(login, repo) {
        return Some(to_person(a));
    }
    let token = match auth::access_token(app, login).await {
        Ok(t) => t,
        Err(TokenError::NotStored | TokenError::Rejected) => return None,
        Err(TokenError::Failed(e)) => {
            tracing::warn!(login, error = %format!("{e:#}"), "token unavailable");
            return None;
        }
    };
    match app.github.repo_as(&token, repo).await {
        Ok(a) => {
            cache.keep_person(login, repo, a.clone());
            Some(to_person(a))
        }
        Err(e) => {
            tracing::warn!(login, rubato.repo = repo, error = %format!("{e:#}"), "reading the repo as the person failed");
            None
        }
    }
}

fn to_person(a: RepoAccess) -> PersonFacts {
    PersonFacts {
        push: a.push,
        methods: a.methods,
    }
}

/// The latest review per team reviewer, oldest first, and how many reviews came from outside the
/// team (design 82 A7).
pub fn team_reviews(reviews: &[ReviewInfo]) -> (Vec<rubato_protocol::pulls::PullReview>, u32) {
    let mut latest: std::collections::BTreeMap<String, &ReviewInfo> = Default::default();
    let mut outside = 0;
    for r in reviews {
        if !trusted_author(&r.author_association, r.author_is_bot) {
            outside += 1;
            continue;
        }
        let k = r.author.to_ascii_lowercase();
        if latest
            .get(&k)
            .is_none_or(|old| old.submitted_at <= r.submitted_at)
        {
            latest.insert(k, r);
        }
    }
    let mut out: Vec<_> = latest
        .into_values()
        .map(|r| rubato_protocol::pulls::PullReview {
            author: r.author.clone(),
            state: r.state.clone(),
            submitted_at: r.submitted_at,
        })
        .collect();
    out.sort_by_key(|r| r.submitted_at);
    (out, outside)
}
