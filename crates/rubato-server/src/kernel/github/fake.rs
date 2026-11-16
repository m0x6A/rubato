//! In-memory GitHub for tests and for a server with no token. Records every call in order.

use super::{
    BranchHead, Comment, GitHubApi, IssueInfo, OpenIssueInfo, OpenIssues, OpenPulls, PullFull,
    PullInfo, RepoAccess, ReviewInfo,
};
use anyhow::Result;
use anyhow::anyhow;
use async_trait::async_trait;
use rubato_protocol::ChecksStatus;
use rubato_protocol::pulls::{ChangedFile, CheckRun, MergeMethod};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct FakeInner {
    pub files: HashMap<(String, String, String), String>,
    /// Reading any file fails with this text (a contents call that errors, issue #82).
    pub file_error: Option<String>,
    /// (repo, path, sha) → the login of the last commit to that file (ADR 0058).
    pub commit_authors: HashMap<(String, String, String), String>,
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
    /// The open pull request listing says it was cut at one page (ADR 0034), as GitHub does
    /// past 100 open pull requests.
    pub open_pulls_truncated: bool,
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
    /// The installation's Issues permission is `write` (ADR 0025). Creating issues also needs
    /// `writes`.
    pub issues_write: bool,
    /// `create_issue` creates the issue (it shows in the listing) and then fails, as a call
    /// that timed out does.
    pub issue_times_out: bool,
    next_issue: u64,
    /// What `app_login` answers: the login the App's own issues carry (issue #165). `None`
    /// stands for an installation not read yet.
    pub app_login: Option<String>,
    /// `comment_as` answers with this status and posts nothing (ADR 0030).
    pub comment_refused: Option<u16>,
    /// `comment_as` posts the comment and then fails, as a call that timed out does.
    pub comment_times_out: bool,
    next_comment: u64,
    /// Every read but `rate_limit` fails with `API rate limit exceeded`, as GitHub answers once
    /// the hour's budget is gone (issue #208).
    pub exhausted: bool,
    /// A pull request's own read (`pull`), by (repo, number). Without one, `pull` answers from
    /// `pulls`: same repo, mergeable, base sha `base-<base>`.
    pub pull_details: HashMap<(String, u64), PullFull>,
    /// `pull` waits this long before answering, so two views can overlap (single flight).
    pub pull_delay: Option<std::time::Duration>,
    /// `pull` fails with this text.
    pub pull_error: Option<String>,
    /// Reviews per (repo, number), oldest first.
    pub reviews: HashMap<(String, u64), Vec<ReviewInfo>>,
    /// Changed files per (repo, number).
    pub files_changed: HashMap<(String, u64), Vec<ChangedFile>>,
    /// Check runs per sha. A sha with none has no runs.
    pub check_runs: HashMap<String, Vec<CheckRun>>,
    /// What `repo_as` answers per (login, repo); without a row, push and every method.
    pub access: HashMap<(String, String), RepoAccess>,
    /// `merge_as` answers with this (status, message) and merges nothing.
    pub merge_refused: Option<(u16, String)>,
    /// `merge_as` fails as a timeout does; `true` merges first (the request reached GitHub).
    pub merge_times_out: Option<bool>,
    /// `merge_as` waits this long before answering, so two requests can overlap.
    pub merge_delay: Option<std::time::Duration>,
    /// Every merge made, as (login, repo, number, sha, method): GitHub's `merged_by`.
    pub merged: Vec<(String, String, u64, String, &'static str)>,
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
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
        if let Some(e) = &g.file_error {
            return Err(anyhow!("{e}"));
        }
        let at = |r: &str| {
            g.files
                .get(&(repo.to_owned(), path.to_owned(), r.to_owned()))
        };
        // A ref that is a branch's head sha reads that branch's file: tests keep files by branch
        // and move a head by changing its sha (ADR 0055 reads the plan at the head sha).
        let by_head = g
            .branches
            .get(repo)
            .and_then(|b| b.iter().find(|(_, sha)| sha == git_ref))
            .and_then(|(name, _)| at(name));
        Ok(at(git_ref).or(by_head).cloned())
    }

    async fn last_commit_author(
        &self,
        repo: &str,
        path: &str,
        sha: &str,
    ) -> Result<Option<String>> {
        let mut g = self.inner.lock().unwrap();
        g.calls
            .push(("last_commit_author", format!("{repo} {path} {sha}")));
        Ok(g.commit_authors
            .get(&(repo.to_owned(), path.to_owned(), sha.to_owned()))
            .cloned())
    }

    async fn branch_sha(&self, repo: &str, branch: &str) -> Result<Option<String>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("branch_sha", format!("{repo} {branch}")));
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
        Ok(g.branches
            .get(repo)
            .and_then(|b| b.iter().find(|(n, _)| n == branch))
            .map(|(_, sha)| sha.clone()))
    }

    async fn branches(&self, repo: &str, prefix: &str) -> Result<Vec<BranchHead>> {
        let mut g = self.inner.lock().unwrap();
        let heads: Vec<BranchHead> = g
            .branches
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
            .unwrap_or_default();
        // One call per page of 100 matching refs, as `git/matching-refs` pages (ADR 0032).
        for _ in 0..heads.len().div_ceil(100).max(1) {
            g.calls.push(("branches", format!("{repo} {prefix}")));
        }
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
        Ok(heads)
    }

    async fn open_pulls(&self, repo: &str) -> Result<OpenPulls> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("open_pulls", repo.to_owned()));
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
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
            truncated: g.open_pulls_truncated,
        })
    }

    async fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueInfo>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("issue", format!("{repo}#{number}")));
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
        Ok(g.issues.get(&(repo.to_owned(), number)).cloned())
    }

    async fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<Comment>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("issue_comments", format!("{repo}#{number}")));
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
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
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
        Ok(g.pulls.get(&(repo.to_owned(), branch.to_owned())).cloned())
    }

    async fn checks(&self, repo: &str, sha: &str) -> Result<ChecksStatus> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("checks", format!("{repo} {sha}")));
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
        Ok(g.checks.get(sha).copied().unwrap_or(ChecksStatus::None))
    }

    async fn pull(&self, repo: &str, number: u64) -> Result<Option<PullFull>> {
        let delay = {
            let mut g = self.inner.lock().unwrap();
            g.calls.push(("pull", format!("{repo}#{number}")));
            g.pull_delay
        };
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
        let g = self.inner.lock().unwrap();
        if let Some(e) = &g.pull_error {
            return Err(anyhow!("{e}"));
        }
        if let Some(p) = g.pull_details.get(&(repo.to_owned(), number)) {
            return Ok(Some(p.clone()));
        }
        Ok(g.pulls
            .iter()
            .find(|((r, _), p)| r == repo && p.number == number)
            .map(|((_, branch), p)| PullFull {
                info: p.clone(),
                branch: branch.clone(),
                body: String::new(),
                fork: false,
                mergeable: Some(true),
                base_sha: format!("base-{}", p.base.as_deref().unwrap_or("main")),
            }))
    }

    async fn pull_reviews(&self, repo: &str, number: u64) -> Result<Vec<ReviewInfo>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("pull_reviews", format!("{repo}#{number}")));
        Ok(g.reviews
            .get(&(repo.to_owned(), number))
            .cloned()
            .unwrap_or_default())
    }

    async fn pull_files(&self, repo: &str, number: u64) -> Result<Vec<ChangedFile>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("pull_files", format!("{repo}#{number}")));
        Ok(g.files_changed
            .get(&(repo.to_owned(), number))
            .cloned()
            .unwrap_or_default())
    }

    async fn check_runs(&self, repo: &str, sha: &str) -> Result<Vec<CheckRun>> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("check_runs", format!("{repo} {sha}")));
        Ok(g.check_runs.get(sha).cloned().unwrap_or_default())
    }

    /// A token `tok-<login>` acts as `<login>`, as `comment_as` does.
    async fn repo_as(&self, token: &str, repo: &str) -> Result<RepoAccess> {
        let mut g = self.inner.lock().unwrap();
        let Some(login) = token.strip_prefix("tok-") else {
            return Err(anyhow!("GitHub refused the token: 401"));
        };
        g.calls.push(("repo_as", format!("{login} {repo}")));
        Ok(g.access
            .get(&(login.to_owned(), repo.to_owned()))
            .cloned()
            .unwrap_or(RepoAccess {
                push: true,
                methods: vec![MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase],
            }))
    }

    /// A token `tok-<login>` merges as `<login>`; a head other than `sha` is GitHub's 409.
    async fn merge_as(
        &self,
        token: &str,
        repo: &str,
        number: u64,
        sha: &str,
        method: MergeMethod,
    ) -> std::result::Result<String, super::MergeError> {
        let delay = {
            let mut g = self.inner.lock().unwrap();
            g.calls.push((
                "merge_as",
                format!("{repo}#{number} {sha} {}", method.as_str()),
            ));
            g.merge_delay
        };
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
        let mut g = self.inner.lock().unwrap();
        let refused = |status: u16, message: &str| super::MergeError::Refused {
            status,
            message: message.into(),
        };
        if let Some((status, message)) = g.merge_refused.clone() {
            return Err(refused(status, &message));
        }
        let Some(login) = token.strip_prefix("tok-").map(str::to_owned) else {
            return Err(refused(401, "Bad credentials"));
        };
        let times_out = g.merge_times_out;
        if times_out == Some(false) {
            return Err(super::MergeError::Unknown("operation timed out".into()));
        }
        let now = chrono::Utc::now();
        let mut found = false;
        for ((r, _), p) in g.pulls.iter_mut() {
            if r == repo && p.number == number {
                if !p.open {
                    return Err(refused(405, "Pull Request is not mergeable"));
                }
                if p.head_sha != sha {
                    return Err(refused(
                        409,
                        "Head branch was modified. Review and try the merge again.",
                    ));
                }
                p.open = false;
                p.merged_at = Some(now);
                found = true;
            }
        }
        if let Some(p) = g.pull_details.get_mut(&(repo.to_owned(), number)) {
            p.info.open = false;
            p.info.merged_at = Some(now);
            found = true;
        }
        if !found {
            return Err(refused(404, "Not Found"));
        }
        g.merged.push((
            login,
            repo.to_owned(),
            number,
            sha.to_owned(),
            method.as_str(),
        ));
        if times_out == Some(true) {
            return Err(super::MergeError::Unknown("operation timed out".into()));
        }
        Ok(format!("merge-{number}"))
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
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
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
        if g.exhausted {
            return Err(anyhow!("API rate limit exceeded"));
        }
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
                base: None,
                title: title.to_owned(),
                author: "rubato[bot]".into(),
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

    fn can_create_issues(&self) -> bool {
        let g = self.inner.lock().unwrap();
        g.writes && g.issues_write
    }

    fn app_login(&self) -> Option<String> {
        self.inner.lock().unwrap().app_login.clone()
    }

    async fn refresh_issue_permission(&self) -> Result<bool> {
        let mut g = self.inner.lock().unwrap();
        // Only an installation reads its permissions, as the port does.
        if g.installation.is_some() {
            g.calls.push(("installation", String::new()));
        }
        Ok(g.writes && g.issues_write)
    }

    /// A token `tok-<login>` acts as `<login>`, a member; the comment lands in `comments`, so
    /// the next comments pass reads it.
    async fn comment_as(
        &self,
        token: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> std::result::Result<super::PostedComment, super::PostError> {
        let mut g = self.inner.lock().unwrap();
        g.calls
            .push(("comment_as", format!("{repo}#{number} {body}")));
        if let Some(status) = g.comment_refused {
            return Err(super::PostError::Refused { status });
        }
        let Some(login) = token.strip_prefix("tok-") else {
            return Err(super::PostError::Refused { status: 401 });
        };
        g.next_comment += 1;
        let id = 9_000_000 + g.next_comment;
        let url = format!("https://github.com/{repo}/issues/{number}#issuecomment-{id}");
        let comment = Comment {
            id,
            author: login.to_owned(),
            author_association: "MEMBER".into(),
            author_is_bot: false,
            body: body.to_owned(),
            created_at: chrono::Utc::now(),
            url: url.clone(),
        };
        g.comments
            .entry((repo.to_owned(), number))
            .or_default()
            .push(comment);
        if g.comment_times_out {
            return Err(super::PostError::Unknown("timed out".into()));
        }
        Ok(super::PostedComment { id, url })
    }

    async fn create_issue(&self, repo: &str, title: &str, body: &str) -> Result<(u64, String)> {
        let mut g = self.inner.lock().unwrap();
        g.calls.push(("create_issue", format!("{repo} {title}")));
        if !(g.writes && g.issues_write) {
            return Err(anyhow!("this GitHub connection cannot create issues"));
        }
        if let Some((failing, why)) = g.fail_write.clone()
            && failing == "create_issue"
        {
            if why.contains("403") {
                g.issues_write = false;
            }
            return Err(anyhow!("{why}"));
        }
        g.next_issue += 1;
        let number = 500 + g.next_issue;
        let url = format!("https://github.com/{repo}/issues/{number}");
        g.open
            .entry(repo.to_owned())
            .or_default()
            .push(OpenIssueInfo {
                number,
                title: title.to_owned(),
                url: url.clone(),
                labels: vec![],
                updated_at: chrono::Utc::now(),
                body: body.to_owned(),
                author: "rubato[bot]".into(),
                author_association: "NONE".into(),
                author_is_bot: true,
            });
        if g.issue_times_out {
            g.issue_times_out = false;
            return Err(anyhow!("operation timed out"));
        }
        Ok((number, url))
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
