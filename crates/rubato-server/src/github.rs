//! GitHub behind one trait (seam `server-github`): octocrab in production, an in-memory fake
//! loaded from recorded responses in tests. Every call is a `rubato.github.call` span.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rubato_protocol::ChecksStatus;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueInfo {
    pub title: String,
    pub state: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullInfo {
    pub number: u64,
    pub open: bool,
    pub draft: bool,
    pub head_sha: String,
    pub merged_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub url: String,
}

#[async_trait]
pub trait GitHubApi: Send + Sync {
    /// File content at a ref, or None when absent.
    async fn file(&self, repo: &str, path: &str, git_ref: &str) -> Result<Option<String>>;
    /// Commit sha a branch points at.
    async fn branch_sha(&self, repo: &str, branch: &str) -> Result<Option<String>>;
    /// Branch names starting with `prefix`.
    async fn branches_with_prefix(&self, repo: &str, prefix: &str) -> Result<Vec<String>>;
    async fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueInfo>>;
    async fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<Comment>>;
    /// The newest PR whose head is `branch`.
    async fn pull_for_branch(&self, repo: &str, branch: &str) -> Result<Option<PullInfo>>;
    async fn checks(&self, repo: &str, sha: &str) -> Result<ChecksStatus>;
    /// (remaining, limit) of the core REST budget.
    async fn rate_limit(&self) -> Result<(u64, u64)>;
}

fn split_repo(repo: &str) -> Result<(&str, &str)> {
    repo.split_once('/').context("repo must be owner/name")
}

// ----- octocrab ------------------------------------------------------------------------------

pub struct Octo {
    client: octocrab::Octocrab,
}

impl Octo {
    pub fn new(token: &str) -> Result<Self> {
        Ok(Self {
            client: octocrab::Octocrab::builder()
                .personal_token(token.to_owned())
                .build()?,
        })
    }
}

#[async_trait]
impl GitHubApi for Octo {
    #[tracing::instrument(name = "rubato.github.call", skip(self), fields(rubato.github.endpoint = "contents"))]
    async fn file(&self, repo: &str, path: &str, git_ref: &str) -> Result<Option<String>> {
        let (o, r) = split_repo(repo)?;
        match self
            .client
            .repos(o, r)
            .get_content()
            .path(path)
            .r#ref(git_ref)
            .send()
            .await
        {
            Ok(items) => Ok(items
                .items
                .into_iter()
                .next()
                .and_then(|i| i.decoded_content())),
            Err(octocrab::Error::GitHub { source, .. }) if source.status_code.as_u16() == 404 => {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(name = "rubato.github.call", skip(self), fields(rubato.github.endpoint = "branch"))]
    async fn branch_sha(&self, repo: &str, branch: &str) -> Result<Option<String>> {
        let (o, r) = split_repo(repo)?;
        let route = format!("/repos/{o}/{r}/branches/{branch}");
        match self
            .client
            .get::<serde_json::Value, _, ()>(route, None::<&()>)
            .await
        {
            Ok(v) => Ok(v
                .pointer("/commit/sha")
                .and_then(|s| s.as_str())
                .map(str::to_owned)),
            Err(octocrab::Error::GitHub { source, .. }) if source.status_code.as_u16() == 404 => {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(name = "rubato.github.call", skip(self), fields(rubato.github.endpoint = "branches"))]
    async fn branches_with_prefix(&self, repo: &str, prefix: &str) -> Result<Vec<String>> {
        let (o, r) = split_repo(repo)?;
        let page = self
            .client
            .repos(o, r)
            .list_branches()
            .per_page(100)
            .send()
            .await?;
        let all = self.client.all_pages(page).await?;
        Ok(all
            .into_iter()
            .map(|b| b.name)
            .filter(|n| n.starts_with(prefix))
            .collect())
    }

    #[tracing::instrument(name = "rubato.github.call", skip(self), fields(rubato.github.endpoint = "issue"))]
    async fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueInfo>> {
        let (o, r) = split_repo(repo)?;
        match self.client.issues(o, r).get(number).await {
            Ok(i) => Ok(Some(IssueInfo {
                title: i.title,
                state: format!("{:?}", i.state).to_lowercase(),
                url: i.html_url.to_string(),
            })),
            Err(octocrab::Error::GitHub { source, .. }) if source.status_code.as_u16() == 404 => {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(name = "rubato.github.call", skip(self), fields(rubato.github.endpoint = "comments"))]
    async fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<Comment>> {
        let (o, r) = split_repo(repo)?;
        let page = self
            .client
            .issues(o, r)
            .list_comments(number)
            .per_page(100)
            .send()
            .await?;
        let all = self.client.all_pages(page).await?;
        Ok(all
            .into_iter()
            .map(|c| Comment {
                id: c.id.into_inner(),
                author: c.user.login,
                body: c.body.unwrap_or_default(),
                created_at: c.created_at,
                url: c.html_url.to_string(),
            })
            .collect())
    }

    #[tracing::instrument(name = "rubato.github.call", skip(self), fields(rubato.github.endpoint = "pulls"))]
    async fn pull_for_branch(&self, repo: &str, branch: &str) -> Result<Option<PullInfo>> {
        let (o, r) = split_repo(repo)?;
        let page = self
            .client
            .pulls(o, r)
            .list()
            .head(format!("{o}:{branch}"))
            .state(octocrab::params::State::All)
            .per_page(10)
            .send()
            .await?;
        let mut pulls: Vec<_> = page.items;
        pulls.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
        Ok(pulls.into_iter().next().map(|p| PullInfo {
            number: p.number,
            open: matches!(p.state, Some(octocrab::models::IssueState::Open)),
            draft: p.draft.unwrap_or(false),
            head_sha: p.head.sha,
            merged_at: p.merged_at,
            updated_at: p.updated_at.unwrap_or_else(Utc::now),
            url: p.html_url.map(|u| u.to_string()).unwrap_or_default(),
        }))
    }

    #[tracing::instrument(name = "rubato.github.call", skip(self), fields(rubato.github.endpoint = "check-runs"))]
    async fn checks(&self, repo: &str, sha: &str) -> Result<ChecksStatus> {
        let (o, r) = split_repo(repo)?;
        let route = format!("/repos/{o}/{r}/commits/{sha}/check-runs?per_page=100");
        let v: serde_json::Value = self.client.get(route, None::<&()>).await?;
        let runs = v
            .get("check_runs")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(summarize_check_runs(runs.iter().map(|run| {
            (
                run.get("status").and_then(|s| s.as_str()).unwrap_or(""),
                run.get("conclusion").and_then(|s| s.as_str()),
            )
        })))
    }

    #[tracing::instrument(name = "rubato.github.call", skip(self), fields(rubato.github.endpoint = "rate_limit"))]
    async fn rate_limit(&self) -> Result<(u64, u64)> {
        let v: serde_json::Value = self.client.get("/rate_limit", None::<&()>).await?;
        let core = v.pointer("/resources/core").cloned().unwrap_or_default();
        Ok((
            core.get("remaining").and_then(|x| x.as_u64()).unwrap_or(0),
            core.get("limit").and_then(|x| x.as_u64()).unwrap_or(0),
        ))
    }
}

/// Check runs → one status. Any failure is red, any incomplete is pending, none configured is none.
pub fn summarize_check_runs<'a>(
    runs: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> ChecksStatus {
    let mut any = false;
    let mut pending = false;
    let mut red = false;
    for (status, conclusion) in runs {
        any = true;
        if status != "completed" {
            pending = true;
            continue;
        }
        match conclusion {
            Some("success") | Some("neutral") | Some("skipped") => {}
            _ => red = true,
        }
    }
    if !any {
        ChecksStatus::None
    } else if red {
        ChecksStatus::Red
    } else if pending {
        ChecksStatus::Pending
    } else {
        ChecksStatus::Green
    }
}

// ----- fake, for tests and for a server with no token ----------------------------------------

#[derive(Default)]
pub struct FakeInner {
    pub files: HashMap<(String, String, String), String>,
    pub branches: HashMap<String, Vec<(String, String)>>,
    pub issues: HashMap<(String, u64), IssueInfo>,
    pub comments: HashMap<(String, u64), Vec<Comment>>,
    pub pulls: HashMap<(String, String), PullInfo>,
    pub checks: HashMap<String, ChecksStatus>,
    pub rate: (u64, u64),
    /// Every call in order, as (method, args); the guard for the poll order (design D4).
    pub calls: Vec<(&'static str, String)>,
}

/// In-memory GitHub. Mutate through `with` in tests.
#[derive(Default)]
pub struct FakeGitHub {
    pub inner: Mutex<FakeInner>,
}

impl FakeGitHub {
    pub fn new() -> Self {
        let f = Self::default();
        f.inner.lock().unwrap().rate = (5000, 5000);
        f
    }

    pub fn with(&self, f: impl FnOnce(&mut FakeInner)) {
        f(&mut self.inner.lock().unwrap());
    }

    pub fn calls(&self) -> Vec<(&'static str, String)> {
        self.inner.lock().unwrap().calls.clone()
    }
}

#[async_trait]
impl GitHubApi for FakeGitHub {
    async fn file(&self, repo: &str, path: &str, git_ref: &str) -> Result<Option<String>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("file", format!("{repo} {path} {git_ref}")));
        Ok(g.files
            .get(&(repo.to_owned(), path.to_owned(), git_ref.to_owned()))
            .cloned())
    }

    async fn branch_sha(&self, repo: &str, branch: &str) -> Result<Option<String>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("branch_sha", format!("{repo} {branch}")));
        Ok(g.branches
            .get(repo)
            .and_then(|b| b.iter().find(|(n, _)| n == branch))
            .map(|(_, sha)| sha.clone()))
    }

    async fn branches_with_prefix(&self, repo: &str, prefix: &str) -> Result<Vec<String>> {
        let mut g = self.inner.lock().unwrap();
        g.calls
            .push(("branches_with_prefix", format!("{repo} {prefix}")));
        Ok(g.branches
            .get(repo)
            .map(|b| {
                b.iter()
                    .filter(|(n, _)| n.starts_with(prefix))
                    .map(|(n, _)| n.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueInfo>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("issue", format!("{repo}#{number}")));
        Ok(g.issues.get(&(repo.to_owned(), number)).cloned())
    }

    async fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<Comment>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("issue_comments", format!("{repo}#{number}")));
        Ok(g.comments
            .get(&(repo.to_owned(), number))
            .cloned()
            .unwrap_or_default())
    }

    async fn pull_for_branch(&self, repo: &str, branch: &str) -> Result<Option<PullInfo>> {
        let mut g = self.inner.lock().unwrap();
        g.calls
            .push(("pull_for_branch", format!("{repo} {branch}")));
        Ok(g.pulls.get(&(repo.to_owned(), branch.to_owned())).cloned())
    }

    async fn checks(&self, repo: &str, sha: &str) -> Result<ChecksStatus> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("checks", format!("{repo} {sha}")));
        Ok(g.checks.get(sha).copied().unwrap_or(ChecksStatus::None))
    }

    async fn rate_limit(&self) -> Result<(u64, u64)> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("rate_limit", String::new()));
        Ok(g.rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_runs_summarise() {
        assert_eq!(summarize_check_runs(std::iter::empty()), ChecksStatus::None);
        assert_eq!(
            summarize_check_runs(
                [
                    ("completed", Some("success")),
                    ("completed", Some("skipped"))
                ]
                .into_iter()
            ),
            ChecksStatus::Green
        );
        assert_eq!(
            summarize_check_runs(
                [("completed", Some("success")), ("in_progress", None)].into_iter()
            ),
            ChecksStatus::Pending
        );
        assert_eq!(
            summarize_check_runs(
                [("completed", Some("failure")), ("in_progress", None)].into_iter()
            ),
            ChecksStatus::Red
        );
    }
}
