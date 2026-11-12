//! In-memory GitHub for tests and for a server with no token. Records every call in order.

use super::{
    BranchHead, Comment, GitHubApi, IssueInfo, OpenIssueInfo, OpenIssues, OpenPulls, PullInfo,
};
use anyhow::Result;
use anyhow::anyhow;
use async_trait::async_trait;
use rubato_protocol::ChecksStatus;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct FakeInner {
    pub files: HashMap<(String, String, String), String>,
    pub branches: HashMap<String, Vec<(String, String)>>,
    pub issues: HashMap<(String, u64), IssueInfo>,
    pub comments: HashMap<(String, u64), Vec<Comment>>,
    pub pulls: HashMap<(String, String), PullInfo>,
    pub checks: HashMap<String, ChecksStatus>,
    pub rate: (u64, u64),
    /// `Some(budget)`: `rate_limit` answers `budget` minus every call recorded so far, as if
    /// each call counted against an hour's budget (issue #49).
    pub budget: Option<u64>,
    /// The open pull request listing fails with this text.
    pub open_pulls_error: Option<String>,
    /// Reading comments fails with this text.
    pub comments_error: Option<String>,
    /// Open issues per repo, as a listing answers them.
    pub open: HashMap<String, Vec<OpenIssueInfo>>,
    /// Repos whose listing fails, with the error text.
    pub open_errors: HashMap<String, String>,
    /// `Some` makes this fake an App installation covering these repos.
    pub installation: Option<Vec<String>>,
    /// Whether writes are allowed (tests of the proposal flow turn this on).
    pub writes: bool,
    /// A write call that fails when it is made, by its name (`create_ref`, `put_file`,
    /// `create_pull`, `close_pull`, `delete_ref`), with the error text.
    pub fail_write: Option<(&'static str, String)>,
    /// `create_pull` creates the pull request and then fails, as a call that timed out does.
    pub pull_times_out: bool,
    next_pull: u64,
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

    async fn branches(&self, repo: &str, prefix: &str) -> Result<Vec<BranchHead>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("branches", format!("{repo} {prefix}")));
        Ok(g.branches
            .get(repo)
            .map(|b| {
                b.iter()
                    .filter(|(n, _)| n.starts_with(prefix))
                    .map(|(name, sha)| BranchHead {
                        name: name.clone(),
                        sha: sha.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn open_pulls(&self, repo: &str) -> Result<OpenPulls> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("open_pulls", repo.to_owned()));
        if let Some(e) = &g.open_pulls_error {
            return Err(anyhow!("{e}"));
        }
        let mut pulls: Vec<(String, PullInfo)> = g
            .pulls
            .iter()
            .filter(|((r, _), p)| r == repo && p.open)
            .map(|((_, branch), p)| (branch.clone(), p.clone()))
            .collect();
        pulls.sort_by_key(|(_, p)| p.number);
        Ok(OpenPulls {
            pulls,
            truncated: false,
        })
    }

    async fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueInfo>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("issue", format!("{repo}#{number}")));
        Ok(g.issues.get(&(repo.to_owned(), number)).cloned())
    }

    async fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<Comment>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("issue_comments", format!("{repo}#{number}")));
        if let Some(e) = &g.comments_error {
            return Err(anyhow!("{e}"));
        }
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
        if let Some(budget) = g.budget {
            return Ok((budget.saturating_sub(g.calls.len() as u64), budget));
        }
        Ok(g.rate)
    }

    async fn open_issues(&self, repo: &str, max: usize) -> Result<OpenIssues> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("open_issues", repo.to_owned()));
        if let Some(e) = g.open_errors.get(repo) {
            return Err(anyhow!("{e}"));
        }
        let all = g.open.get(repo).cloned().unwrap_or_default();
        let truncated = all.len() > max;
        Ok(OpenIssues {
            issues: all.into_iter().take(max).collect(),
            truncated,
        })
    }

    fn is_installation(&self) -> bool {
        self.inner.lock().unwrap().installation.is_some()
    }

    async fn installation_repos(&self) -> Result<Vec<String>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("installation_repos", String::new()));
        Ok(g.installation.clone().unwrap_or_default())
    }

    fn can_write(&self) -> bool {
        self.inner.lock().unwrap().writes
    }

    async fn create_ref(&self, repo: &str, branch: &str, sha: &str) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.write("create_ref", format!("{repo} {branch} {sha}"))?;
        let branches = g.branches.entry(repo.to_owned()).or_default();
        if !branches.iter().any(|(n, _)| n == branch) {
            branches.push((branch.to_owned(), sha.to_owned()));
        }
        Ok(())
    }

    async fn put_file(
        &self,
        repo: &str,
        branch: &str,
        path: &str,
        content: &str,
        message: &str,
    ) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.write("put_file", format!("{repo} {branch} {path} {message}"))?;
        g.files.insert(
            (repo.to_owned(), path.to_owned(), branch.to_owned()),
            content.to_owned(),
        );
        Ok(())
    }

    async fn create_pull(
        &self,
        repo: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String)> {
        let mut g = self.inner.lock().unwrap();
        g.write(
            "create_pull",
            format!("{repo} {head} {base} {title}\n{body}"),
        )?;
        g.next_pull += 1;
        let number = 100 + g.next_pull;
        let url = format!("https://github.com/{repo}/pull/{number}");
        g.pulls.insert(
            (repo.to_owned(), head.to_owned()),
            PullInfo {
                number,
                open: true,
                draft: false,
                head_sha: "proposed".into(),
                merged_at: None,
                updated_at: chrono::Utc::now(),
                url: url.clone(),
            },
        );
        if g.pull_times_out {
            g.pull_times_out = false;
            return Err(anyhow!("operation timed out"));
        }
        Ok((number, url))
    }

    async fn close_pull(&self, repo: &str, number: u64) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.write("close_pull", format!("{repo} {number}"))?;
        for ((r, _), p) in g.pulls.iter_mut() {
            if r == repo && p.number == number {
                p.open = false;
            }
        }
        Ok(())
    }

    async fn delete_ref(&self, repo: &str, branch: &str) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        g.write("delete_ref", format!("{repo} {branch}"))?;
        if let Some(b) = g.branches.get_mut(repo) {
            b.retain(|(n, _)| n != branch);
        }
        Ok(())
    }
}

impl FakeInner {
    /// Record a write call; fail it when writes are off or the test asked for this one to fail.
    fn write(&mut self, name: &'static str, args: String) -> Result<()> {
        self.calls.push((name, args));
        if !self.writes {
            return Err(anyhow!("this GitHub connection cannot write"));
        }
        if let Some((failing, why)) = &self.fail_write
            && *failing == name
        {
            return Err(anyhow!("{why}"));
        }
        Ok(())
    }
}
