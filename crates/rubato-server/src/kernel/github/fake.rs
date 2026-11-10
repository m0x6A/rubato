//! In-memory GitHub for tests and for a server with no token. Records every call in order.

use super::{Comment, GitHubApi, IssueInfo, PullInfo};
use anyhow::Result;
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
