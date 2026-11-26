//! The GitHub port (seam `server-github`) and its octocrab adapter. Every call is a
//! `rubato.github.call` span that records its error; the client has connect and read timeouts
//! so a hung call ends as an error (design D15).

use crate::kernel::config::GitHubApp;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rubato_protocol::ChecksStatus;
use std::time::Duration;

/// Connect and read timeouts on the GitHub client. With octocrab's default three retries a
/// hung call ends after at most `4 × READ_TIMEOUT` = 60 s.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// The most pages of 100 issue comments read per issue: the newest 500 (design 193 D6).
pub const MAX_COMMENT_PAGES: usize = 5;
/// The page size asked of `git/matching-refs`, and the most pages one branch listing reads.
pub const REFS_PER_PAGE: usize = 100;
pub const MAX_REF_PAGES: u32 = 10;

/// `#[instrument(name = …)]` takes a bare identifier.
const GITHUB_CALL: &str = rubato_protocol::otel::span::GITHUB_CALL;

/// Count GitHub REST calls, one per page, by endpoint (`rubato.github.calls`, ADR 0032).
fn count_call(endpoint: &'static str, pages: u64) {
    use opentelemetry::KeyValue;
    use std::sync::OnceLock;
    static CALLS: OnceLock<opentelemetry::metrics::Counter<u64>> = OnceLock::new();
    CALLS
        .get_or_init(|| {
            opentelemetry::global::meter("rubato-server")
                .u64_counter(rubato_protocol::otel::metric::GITHUB_CALLS)
                .build()
        })
        .add(
            pages,
            &[KeyValue::new(
                rubato_protocol::otel::attr::GITHUB_ENDPOINT,
                endpoint,
            )],
        );
}

/// A repository's tarball at one commit, as GitHub serves it: gzip, with one top-level
/// directory. Held compressed, under the caller's cap; the caller decompresses it as a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tarball {
    Gz(Vec<u8>),
    /// Larger than the cap the caller gave. Not an error: the caller marks its source cut.
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueInfo {
    pub title: String,
    pub state: String,
    pub url: String,
}

/// One open issue from a repo listing.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize)]
pub struct OpenIssueInfo {
    pub number: u64,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
    /// The first [`BODY_EXCERPT_BYTES`] of the body: for the `Map:` line and the suggested
    /// place (ADR 0026). Kept in memory for one listing; never stored or sent to a browser.
    #[serde(default)]
    pub body: String,
    /// GitHub's `author_association` of the issue's author (`OWNER`, `MEMBER`,
    /// `COLLABORATOR`, `CONTRIBUTOR`, `NONE`, …): whether its `Map:` line is trusted.
    #[serde(default)]
    pub author_association: String,
    /// Login of the issue's author.
    #[serde(default)]
    pub author: String,
    /// The author is a Bot account (`user.type == "Bot"`), as an issue the App created is
    /// (ADR 0025, issue #165).
    #[serde(default)]
    pub author_is_bot: bool,
}

/// How much of an issue body is read for placement (ADR 0026, design A7).
pub const BODY_EXCERPT_BYTES: usize = 4096;

/// At most `max` bytes of `s`, cut at a character boundary.
pub fn excerpt(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// A repo's open issues, pull requests left out. `truncated` when the listing hit the cap.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenIssues {
    pub issues: Vec<OpenIssueInfo>,
    pub truncated: bool,
}

/// At most this many issues per page, and so `max / 100` calls per repo per listing.
pub const ISSUES_PER_PAGE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: u64,
    pub author: String,
    /// GitHub's `author_association`: OWNER, MEMBER, COLLABORATOR, CONTRIBUTOR, NONE, …
    pub author_association: String,
    /// The author is a Bot account (`user.type == "Bot"`).
    pub author_is_bot: bool,
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
    /// The branch it merges into, when known: the bottom of a stack targets the default branch
    /// (ADR 0032).
    pub base: Option<String>,
    /// The title, and the login of who opened it (issue #82, ADR 0034).
    pub title: String,
    pub author: String,
}

/// A branch and the commit it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchHead {
    pub name: String,
    pub sha: String,
}

/// A repo's open pull requests whose head branch is in the repo itself (forks left out), as
/// (head branch, pull request). `truncated` when the listing hit [`PULLS_PER_PAGE`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenPulls {
    pub pulls: Vec<(String, PullInfo)>,
    pub truncated: bool,
}

/// One pull request read on its own (`GET /repos/{o}/{r}/pulls/{n}`), for its view (ADR 0034).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullFull {
    pub info: PullInfo,
    /// The head branch.
    pub branch: String,
    pub body: String,
    /// The head is in another repo than the base.
    pub fork: bool,
    /// GitHub's `mergeable`: `None` while it is computing.
    pub mergeable: Option<bool>,
    /// The base commit GitHub reports, for "does the repo run checks at all" (design 82 A6).
    pub base_sha: String,
}

/// One review of a pull request, with what decides whether its author is on the team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewInfo {
    pub author: String,
    pub author_association: String,
    pub author_is_bot: bool,
    pub state: String,
    pub submitted_at: DateTime<Utc>,
}

/// What the person may do on a repo, read with their token (ADR 0033).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoAccess {
    pub push: bool,
    /// The merge methods the repo allows, in the order merge, squash, rebase.
    pub methods: Vec<rubato_protocol::pulls::MergeMethod>,
}

/// How a merge as the person failed (ADR 0033). `Refused` sent nothing that took effect: GitHub
/// answered 405 (not mergeable), 409 (the head moved), 403, 422. `Unknown` may have merged: the
/// caller must not send again and lets the poll find out.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("GitHub refused the merge ({status}): {message}")]
    Refused { status: u16, message: String },
    #[error("the merge may have happened: {0}")]
    Unknown(String),
}

/// One page of changed files per view (design 82 A3).
pub const FILES_PER_PAGE: usize = rubato_protocol::pulls::FILES_PER_PAGE_HINT;

/// A comment posted as a person through their user-to-server token (ADR 0030).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedComment {
    pub id: u64,
    pub url: String,
}

/// How a post as a person failed. `Unknown` means it may have been posted (a timeout, a
/// dropped connection): the caller must not post again and finds it in the comments instead.
#[derive(Debug, thiserror::Error)]
pub enum PostError {
    #[error("GitHub refused the comment: {status}")]
    Refused { status: u16 },
    #[error("the comment may have been posted: {0}")]
    Unknown(String),
}

/// The timeout of a write as a person. With retries off, the longest a post can take.
pub const USER_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// One page of open pull requests per repo per tick (issue #49).
pub const PULLS_PER_PAGE: usize = 100;

#[async_trait]
pub trait GitHubApi: Send + Sync {
    /// File content at a ref, or None when absent.
    async fn file(&self, repo: &str, path: &str, git_ref: &str) -> Result<Option<String>>;
    /// Commit sha a branch points at.
    async fn branch_sha(&self, repo: &str, branch: &str) -> Result<Option<String>>;
    /// The login of whoever made the last commit to `path` reachable from `sha` (ADR 0058):
    /// one `GET /repos/{o}/{r}/commits?path=&sha=&per_page=1`. None when unknown.
    async fn last_commit_author(
        &self,
        _repo: &str,
        _path: &str,
        _sha: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }
    /// Branches starting with `prefix`, with their head commits.
    async fn branches(&self, repo: &str, prefix: &str) -> Result<Vec<BranchHead>>;
    /// The repo's open pull requests from its own branches (issue #49).
    async fn open_pulls(&self, repo: &str) -> Result<OpenPulls>;
    async fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueInfo>>;
    async fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<Comment>>;
    /// The newest PR whose head is `branch`.
    async fn pull_for_branch(&self, repo: &str, branch: &str) -> Result<Option<PullInfo>>;
    async fn checks(&self, repo: &str, sha: &str) -> Result<ChecksStatus>;
    /// The default branch's head sha, for the context graph (ADR 0084): one call. `None` when
    /// the repository is not there, is empty, or this connection cannot see it.
    async fn default_head(&self, _repo: &str) -> Result<Option<String>> {
        Ok(None)
    }
    /// The repository's tarball at `sha`, at most `max_bytes` compressed (ADR 0084). One call,
    /// never retried here: the next tick is the retry.
    async fn tarball(&self, _repo: &str, _sha: &str, _max_bytes: u64) -> Result<Tarball> {
        Err(anyhow::anyhow!(
            "this GitHub connection cannot download a repository"
        ))
    }
    /// One pull request, any state; None when it does not exist (ADR 0034, read for a view).
    async fn pull(&self, _repo: &str, _number: u64) -> Result<Option<PullFull>> {
        Err(anyhow::anyhow!(
            "this GitHub connection cannot read a pull request"
        ))
    }
    /// One page of a pull request's reviews, oldest first.
    async fn pull_reviews(&self, _repo: &str, _number: u64) -> Result<Vec<ReviewInfo>> {
        Ok(Vec::new())
    }
    /// One page of [`FILES_PER_PAGE`] changed files.
    async fn pull_files(
        &self,
        _repo: &str,
        _number: u64,
    ) -> Result<Vec<rubato_protocol::pulls::ChangedFile>> {
        Ok(Vec::new())
    }
    /// One page of check runs at `sha`, each with its link.
    async fn check_runs(
        &self,
        _repo: &str,
        _sha: &str,
    ) -> Result<Vec<rubato_protocol::pulls::CheckRun>> {
        Ok(Vec::new())
    }
    /// The person's permission on the repo and the merge methods it allows, read with their
    /// user-to-server token (ADR 0033). Retries off, as every call as the person.
    async fn repo_as(&self, _token: &str, _repo: &str) -> Result<RepoAccess> {
        Err(anyhow::anyhow!(
            "this GitHub connection cannot act as a person"
        ))
    }
    /// Merge `number` at `sha` with `method`, as the person whose user-to-server token this is
    /// (ADR 0033): `PUT …/pulls/{n}/merge`. Sent once, retries off, [`USER_WRITE_TIMEOUT`]. The
    /// merge commit's sha.
    async fn merge_as(
        &self,
        _token: &str,
        _repo: &str,
        _number: u64,
        _sha: &str,
        _method: rubato_protocol::pulls::MergeMethod,
    ) -> std::result::Result<String, MergeError> {
        Err(MergeError::Refused {
            status: 501,
            message: "this GitHub connection cannot merge".into(),
        })
    }
    /// The local stand-in has one person and no tokens: it acts for whoever is logged in.
    fn acts_without_token(&self) -> bool {
        false
    }
    /// (remaining, limit) of the core REST budget.
    async fn rate_limit(&self) -> Result<(u64, u64)>;
    /// Open issues of a repo, pull requests excluded, at most `max` (ADR 0019).
    async fn open_issues(&self, repo: &str, max: usize) -> Result<OpenIssues>;
    /// Whether this port acts as a GitHub App installation (ADR 0022).
    fn is_installation(&self) -> bool {
        false
    }
    /// `owner/name` of every repo the installation covers. Only called on an installation.
    async fn installation_repos(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    /// Whether this port can write to GitHub: an installation, the local stand-in, or the fake
    /// when a test turns writes on. A personal-token client cannot (ADR 0018, 0022).
    fn can_write(&self) -> bool {
        false
    }

    // The writes of a map proposal (ADR 0018). Each is one step with a name on-call can read
    // in the log. A port that cannot write answers with an error.

    /// Create branch `branch` at `sha`. A branch that exists already is not an error: a
    /// proposal that is resumed finds its own branch.
    async fn create_ref(&self, _repo: &str, _branch: &str, _sha: &str) -> Result<()> {
        Err(anyhow::anyhow!("this GitHub connection cannot write"))
    }
    /// Write `content` to `path` on `branch` as one commit.
    async fn put_file(
        &self,
        _repo: &str,
        _branch: &str,
        _path: &str,
        _content: &str,
        _message: &str,
    ) -> Result<()> {
        Err(anyhow::anyhow!("this GitHub connection cannot write"))
    }
    /// Open a pull request from `head` into `base`: its number and its page.
    async fn create_pull(
        &self,
        _repo: &str,
        _head: &str,
        _base: &str,
        _title: &str,
        _body: &str,
    ) -> Result<(u64, String)> {
        Err(anyhow::anyhow!("this GitHub connection cannot write"))
    }
    async fn close_pull(&self, _repo: &str, _number: u64) -> Result<()> {
        Err(anyhow::anyhow!("this GitHub connection cannot write"))
    }
    /// Delete branch `branch`. A branch that is gone already is not an error.
    async fn delete_ref(&self, _repo: &str, _branch: &str) -> Result<()> {
        Err(anyhow::anyhow!("this GitHub connection cannot write"))
    }

    // Issues created from Rubato (ADR 0025).

    /// Whether this port can open issues: an installation whose permissions, as last read,
    /// include Issues: write; the local stand-in; the fake when a test says so.
    fn can_create_issues(&self) -> bool {
        false
    }
    /// Read the installation's Issues permission again and remember it. Called at start and on
    /// every full poll pass, so a re-approved installation is noticed without a restart.
    async fn refresh_issue_permission(&self) -> Result<bool> {
        Ok(self.can_create_issues())
    }
    /// The login the App's own issues carry, `<app_slug>[bot]`, as the last installation read
    /// gave it (issue #165). `None` until read, and on a port that is not an App.
    fn app_login(&self) -> Option<String> {
        None
    }
    /// Open an issue: its number and page. Sent once, with no retry: an issue POST is not
    /// idempotent. A 403 clears [`GitHubApi::can_create_issues`] until the next refresh.
    /// Post an issue comment as the person whose user-to-server token this is (ADR 0030). Sent
    /// once, retries off, [`USER_WRITE_TIMEOUT`].
    async fn comment_as(
        &self,
        _token: &str,
        _repo: &str,
        _number: u64,
        _body: &str,
    ) -> std::result::Result<PostedComment, PostError> {
        Err(PostError::Refused { status: 501 })
    }
    async fn create_issue(&self, _repo: &str, _title: &str, _body: &str) -> Result<(u64, String)> {
        Err(anyhow::anyhow!(
            "this GitHub connection cannot create issues"
        ))
    }
}

/// From one page of `GET /repos/{repo}/issues`: the issues, without the pull requests.
pub fn issues_from_page(page: &serde_json::Value) -> Vec<OpenIssueInfo> {
    page.as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|i| i.get("pull_request").is_none())
        .filter_map(|i| {
            Some(OpenIssueInfo {
                number: i.get("number")?.as_u64()?,
                title: i.get("title")?.as_str()?.to_owned(),
                url: i.get("html_url")?.as_str()?.to_owned(),
                labels: i
                    .get("labels")
                    .and_then(|l| l.as_array())
                    .map(|l| {
                        l.iter()
                            .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                updated_at: i
                    .get("updated_at")
                    .and_then(|t| t.as_str())
                    .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now),
                body: excerpt(
                    i.get("body").and_then(|b| b.as_str()).unwrap_or_default(),
                    BODY_EXCERPT_BYTES,
                ),
                author_association: i
                    .get("author_association")
                    .and_then(|a| a.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                author: i
                    .pointer("/user/login")
                    .and_then(|a| a.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                author_is_bot: i.pointer("/user/type").and_then(|t| t.as_str()) == Some("Bot"),
            })
        })
        .collect()
}

/// From `GET /repos/{o}/{r}/pulls/{n}`.
pub fn pull_from_json(v: &serde_json::Value) -> Option<PullFull> {
    let s = |p: &str| v.pointer(p).and_then(|x| x.as_str()).map(str::to_owned);
    let ts = |p: &str| {
        v.pointer(p)
            .and_then(|x| x.as_str())
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
    };
    let head_repo = s("/head/repo/full_name");
    let base_repo = s("/base/repo/full_name");
    Some(PullFull {
        info: PullInfo {
            number: v.get("number")?.as_u64()?,
            open: s("/state").as_deref() == Some("open"),
            draft: v.get("draft").and_then(|d| d.as_bool()).unwrap_or(false),
            head_sha: s("/head/sha")?,
            merged_at: ts("/merged_at"),
            updated_at: ts("/updated_at").unwrap_or_else(Utc::now),
            url: s("/html_url").unwrap_or_default(),
            base: s("/base/ref"),
            title: s("/title").unwrap_or_default(),
            author: s("/user/login").unwrap_or_default(),
        },
        branch: s("/head/ref")?,
        body: s("/body").unwrap_or_default(),
        // A head repo that was deleted reads as null: not this repo, so a fork.
        fork: match (&head_repo, &base_repo) {
            (Some(h), Some(b)) => !h.eq_ignore_ascii_case(b),
            _ => true,
        },
        mergeable: v.get("mergeable").and_then(|m| m.as_bool()),
        base_sha: s("/base/sha").unwrap_or_default(),
    })
}

/// From one page of `GET …/pulls/{n}/reviews`. Pending reviews (not submitted) are left out.
pub fn reviews_from_page(v: &serde_json::Value) -> Vec<ReviewInfo> {
    v.as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            Some(ReviewInfo {
                author: r.pointer("/user/login")?.as_str()?.to_owned(),
                author_association: r
                    .get("author_association")
                    .and_then(|a| a.as_str())
                    .unwrap_or("NONE")
                    .to_owned(),
                author_is_bot: r.pointer("/user/type").and_then(|t| t.as_str()) == Some("Bot"),
                state: r.get("state")?.as_str()?.to_owned(),
                submitted_at: r
                    .get("submitted_at")?
                    .as_str()
                    .and_then(|t| DateTime::parse_from_rfc3339(t).ok())?
                    .with_timezone(&Utc),
            })
        })
        .collect()
}

/// From one page of `GET …/pulls/{n}/files`.
pub fn files_from_page(v: &serde_json::Value) -> Vec<rubato_protocol::pulls::ChangedFile> {
    v.as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|f| {
            Some(rubato_protocol::pulls::ChangedFile {
                path: f.get("filename")?.as_str()?.to_owned(),
                additions: f.get("additions").and_then(|x| x.as_u64()).unwrap_or(0),
                deletions: f.get("deletions").and_then(|x| x.as_u64()).unwrap_or(0),
            })
        })
        .collect()
}

/// From `GET …/commits/{sha}/check-runs`.
pub fn check_runs_from_page(v: &serde_json::Value) -> Vec<rubato_protocol::pulls::CheckRun> {
    v.get("check_runs")
        .and_then(|c| c.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            Some(rubato_protocol::pulls::CheckRun {
                name: r.get("name")?.as_str()?.to_owned(),
                status: r.get("status")?.as_str()?.to_owned(),
                conclusion: r
                    .get("conclusion")
                    .and_then(|c| c.as_str())
                    .map(str::to_owned),
                url: r
                    .get("html_url")
                    .and_then(|u| u.as_str())
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect()
}

/// From `GET /repos/{o}/{r}` as the person: `permissions.push` and `allow_*_merge`.
pub fn access_from_repo(v: &serde_json::Value) -> RepoAccess {
    use rubato_protocol::pulls::MergeMethod;
    let on = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    let methods = [
        ("allow_merge_commit", MergeMethod::Merge),
        ("allow_squash_merge", MergeMethod::Squash),
        ("allow_rebase_merge", MergeMethod::Rebase),
    ]
    .into_iter()
    .filter(|(k, _)| on(k))
    .map(|(_, m)| m)
    .collect();
    RepoAccess {
        push: v.pointer("/permissions/push").and_then(|p| p.as_bool()) == Some(true),
        methods,
    }
}

/// `owner/name`, each half of `[A-Za-z0-9_.-]+` and neither `.` nor `..`, so no repo string
/// can add a path segment or a query to a REST path (issue #163).
fn split_repo(repo: &str) -> Result<(&str, &str)> {
    fn part(s: &str) -> bool {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    }
    repo.split_once('/')
        .filter(|(o, r)| part(o) && part(r))
        .with_context(|| {
            format!("repo must be owner/name of letters, digits, '_', '.' or '-', not {repo:?}")
        })
}

// ----- octocrab ------------------------------------------------------------------------------

pub struct Octo {
    client: octocrab::Octocrab,
    /// The REST root, for clients made per call (a person's token, ADR 0030).
    base: Option<String>,
    /// Acting as a GitHub App installation, not with a personal token.
    installation: bool,
    /// As an installation only: the App itself (a JWT), to read the installation's permissions,
    /// and an installation client with retries off, for the issue POST (ADR 0025).
    as_app: Option<AsApp>,
    /// The installation's Issues permission is `write`, as last read.
    issues_write: std::sync::atomic::AtomicBool,
    /// `<app_slug>[bot]` from the installation, as last read (issue #165).
    app_login: std::sync::Mutex<Option<String>>,
}

struct AsApp {
    app: octocrab::Octocrab,
    no_retry: octocrab::Octocrab,
    installation_id: u64,
}

impl Octo {
    pub fn new(token: &str) -> Result<Self> {
        Self::build(token, None, READ_TIMEOUT)
    }

    /// Against another base URI, for tests (and GitHub Enterprise).
    pub fn with_base_uri(token: &str, base_uri: &str, read_timeout: Duration) -> Result<Self> {
        Self::build(token, Some(base_uri), read_timeout)
    }

    fn build(token: &str, base_uri: Option<&str>, read_timeout: Duration) -> Result<Self> {
        let mut b = octocrab::Octocrab::builder()
            .set_connect_timeout(Some(CONNECT_TIMEOUT))
            .set_read_timeout(Some(read_timeout))
            .personal_token(token.to_owned());
        if let Some(uri) = base_uri {
            b = b.base_uri(uri)?;
        }
        Ok(Self {
            client: b.build()?,
            base: base_uri.map(str::to_owned),
            installation: false,
            as_app: None,
            issues_write: Default::default(),
            app_login: Default::default(),
        })
    }

    /// A client acting as the installation of a GitHub App (ADR 0022). Call from inside the
    /// runtime. The settings are checked by `kernel::github::check_app` before this is used.
    pub fn for_installation(app: &GitHubApp, base_uri: Option<&str>) -> Result<Self> {
        let crab = app_client(app, base_uri, READ_TIMEOUT)?;
        let id = octocrab::models::InstallationId(app.installation_id);
        let no_retry = app_client_with(app, base_uri, READ_TIMEOUT, false)?.installation(id)?;
        Ok(Self {
            client: crab.installation(id)?,
            base: base_uri.map(str::to_owned),
            installation: true,
            as_app: Some(AsApp {
                app: crab,
                no_retry,
                installation_id: app.installation_id,
            }),
            issues_write: Default::default(),
            app_login: Default::default(),
        })
    }
}

/// A client acting as a person, with their user-to-server token: retries off and
/// [`USER_WRITE_TIMEOUT`], since its only use is a write that must not be sent twice (ADR 0030).
fn user_client(token: &str, base_uri: Option<&str>) -> Result<octocrab::Octocrab> {
    use octocrab::service::middleware::retry::RetryConfig;
    let mut b = octocrab::Octocrab::builder()
        .add_retry_config(RetryConfig::None)
        .set_connect_timeout(Some(CONNECT_TIMEOUT))
        .set_read_timeout(Some(USER_WRITE_TIMEOUT))
        .personal_token(token.to_owned());
    if let Some(uri) = base_uri {
        b = b.base_uri(uri)?;
    }
    Ok(b.build()?)
}

/// A client authenticated as the App itself (a JWT), for `GET /app` and minting tokens.
pub(super) fn app_client(
    app: &GitHubApp,
    base_uri: Option<&str>,
    read_timeout: Duration,
) -> Result<octocrab::Octocrab> {
    app_client_with(app, base_uri, read_timeout, true)
}

/// As [`app_client`], with octocrab's retries on or off.
fn app_client_with(
    app: &GitHubApp,
    base_uri: Option<&str>,
    read_timeout: Duration,
    retries: bool,
) -> Result<octocrab::Octocrab> {
    use octocrab::service::middleware::retry::RetryConfig;
    let retry = if retries {
        RetryConfig::Simple(3)
    } else {
        RetryConfig::None
    };
    let mut b = octocrab::Octocrab::builder()
        .add_retry_config(retry)
        .set_connect_timeout(Some(CONNECT_TIMEOUT))
        .set_read_timeout(Some(read_timeout))
        .app_from_rsa_pem(
            octocrab::models::AppId(app.app_id),
            app.private_key.as_bytes(),
        )
        .context("the private key is not an RSA key in PEM form")?;
    if let Some(uri) = base_uri {
        b = b.base_uri(uri)?;
    }
    Ok(b.build()?)
}

#[async_trait]
impl GitHubApi for Octo {
    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "contents"))]
    async fn file(&self, repo: &str, path: &str, git_ref: &str) -> Result<Option<String>> {
        count_call("contents", 1);
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "commits"))]
    async fn last_commit_author(
        &self,
        repo: &str,
        path: &str,
        sha: &str,
    ) -> Result<Option<String>> {
        count_call("commits", 1);
        let (o, r) = split_repo(repo)?;
        let route = format!("/repos/{o}/{r}/commits");
        let v = self
            .client
            .get::<serde_json::Value, _, _>(
                route,
                Some(&[("path", path), ("sha", sha), ("per_page", "1")]),
            )
            .await?;
        // The GitHub login when the commit's author is a GitHub user, else the git author's name.
        Ok(v.pointer("/0/author/login")
            .or_else(|| v.pointer("/0/commit/author/name"))
            .and_then(|x| x.as_str())
            .map(str::to_owned))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "branch"))]
    async fn branch_sha(&self, repo: &str, branch: &str) -> Result<Option<String>> {
        count_call("branch", 1);
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "repo_head"))]
    async fn default_head(&self, repo: &str) -> Result<Option<String>> {
        count_call("repo_head", 1);
        let (o, r) = split_repo(repo)?;
        // `HEAD` is the default branch: its name is never asked for.
        let route = format!("/repos/{o}/{r}/commits/HEAD");
        match self
            .client
            .get::<serde_json::Value, _, ()>(route, None::<&()>)
            .await
        {
            Ok(v) => Ok(v.get("sha").and_then(|s| s.as_str()).map(str::to_owned)),
            // 404: not there, or not covered. 409: the repository is empty.
            Err(octocrab::Error::GitHub { source, .. })
                if matches!(source.status_code.as_u16(), 404 | 409) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "tarball"))]
    async fn tarball(&self, repo: &str, sha: &str, max_bytes: u64) -> Result<Tarball> {
        use http_body_util::BodyExt;
        count_call("tarball", 1);
        let (o, r) = split_repo(repo)?;
        // The redirect to the archive host is followed inside this one call and span.
        let resp = self
            .client
            .repos(o, r)
            .download_tarball(sha.to_owned())
            .await?;
        anyhow::ensure!(
            resp.status().is_success(),
            "tarball of {repo}: HTTP {}",
            resp.status()
        );
        let mut body = resp.into_body();
        let mut out: Vec<u8> = Vec::new();
        while let Some(frame) = body.frame().await {
            let Ok(data) = frame?.into_data() else {
                continue;
            };
            if (out.len() + data.len()) as u64 > max_bytes {
                return Ok(Tarball::TooLarge);
            }
            out.extend_from_slice(&data);
        }
        Ok(Tarball::Gz(out))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "branches"))]
    async fn branches(&self, repo: &str, prefix: &str) -> Result<Vec<BranchHead>> {
        // `git/matching-refs` filters by prefix on GitHub's side (ADR 0032). It answers with
        // every matching ref at once and ignores `per_page` and `page` (seen 2026-11-16: 144
        // refs for any page), so "a full page means there is another" never ends there. A page
        // of exactly `REFS_PER_PAGE` may be one of several; any other size is the whole answer.
        // A page that brings no new ref, or `MAX_REF_PAGES`, ends the listing whatever GitHub
        // does: one listing must never cost more than that many calls.
        #[derive(serde::Deserialize)]
        struct Obj {
            sha: String,
        }
        #[derive(serde::Deserialize)]
        struct Ref {
            #[serde(rename = "ref")]
            name: String,
            object: Obj,
        }
        split_repo(repo)?;
        let mut heads: Vec<BranchHead> = Vec::new();
        let mut names = std::collections::HashSet::new();
        let mut pages = 0u64;
        for page in 1..=MAX_REF_PAGES {
            let refs: Vec<Ref> = match self
                .client
                .get(
                    format!("/repos/{repo}/git/matching-refs/heads/{prefix}"),
                    Some(&[
                        ("per_page", REFS_PER_PAGE.to_string()),
                        ("page", page.to_string()),
                    ]),
                )
                .await
            {
                Ok(refs) => refs,
                Err(e) => {
                    count_call("branches", pages + 1);
                    return Err(e.into());
                }
            };
            pages += 1;
            let paged = refs.len() == REFS_PER_PAGE;
            let before = heads.len();
            heads.extend(refs.into_iter().filter_map(|r| {
                let name = r.name.strip_prefix("refs/heads/")?;
                names.insert(name.to_owned()).then(|| BranchHead {
                    name: name.to_owned(),
                    sha: r.object.sha,
                })
            }));
            if !paged || heads.len() == before {
                break;
            }
            if page == MAX_REF_PAGES {
                tracing::warn!(
                    rubato.repo = %repo,
                    prefix,
                    read = heads.len(),
                    pages,
                    "branch listing cut"
                );
            }
        }
        count_call("branches", pages);
        Ok(heads)
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "pulls"))]
    async fn open_pulls(&self, repo: &str) -> Result<OpenPulls> {
        count_call("pulls", 1);
        let (o, r) = split_repo(repo)?;
        let page = self
            .client
            .pulls(o, r)
            .list()
            .state(octocrab::params::State::Open)
            .per_page(PULLS_PER_PAGE as u8)
            .send()
            .await?;
        let truncated = page.items.len() >= PULLS_PER_PAGE;
        // Same rule as `pull_for_branch` (`head=owner:branch`): the head label names this owner.
        // GitHub logins are case-insensitive, and the map may spell the owner differently.
        let pulls =
            page.items
                .into_iter()
                .filter(|p| {
                    p.head.label.as_deref().is_some_and(|l| {
                        l.eq_ignore_ascii_case(&format!("{o}:{}", p.head.ref_field))
                    })
                })
                .map(|p| {
                    (
                        p.head.ref_field.clone(),
                        PullInfo {
                            number: p.number,
                            open: true,
                            draft: p.draft.unwrap_or(false),
                            head_sha: p.head.sha,
                            merged_at: p.merged_at,
                            updated_at: p.updated_at.unwrap_or_else(Utc::now),
                            url: p.html_url.map(|u| u.to_string()).unwrap_or_default(),
                            base: Some(p.base.ref_field.clone()),
                            title: p.title.clone().unwrap_or_default(),
                            author: p.user.as_ref().map(|u| u.login.clone()).unwrap_or_default(),
                        },
                    )
                })
                .collect();
        Ok(OpenPulls { pulls, truncated })
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "issue"))]
    async fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueInfo>> {
        count_call("issue", 1);
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "comments"))]
    async fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<Comment>> {
        let (o, r) = split_repo(repo)?;
        let issues = self.client.issues(o, r);
        let first = issues.list_comments(number).per_page(100).send().await?;
        let pages = first.number_of_pages().unwrap_or(1);
        // Design 193 D6: at most the newest MAX_COMMENT_PAGES pages; GitHub lists oldest first.
        let all = if pages as usize <= MAX_COMMENT_PAGES {
            let all = self.client.all_pages(first).await?;
            count_call("comments", u64::from(pages.max(1)));
            all
        } else {
            let from = pages - MAX_COMMENT_PAGES as u32 + 1;
            let mut all = Vec::new();
            for p in from..=pages {
                let page = issues
                    .list_comments(number)
                    .per_page(100)
                    .page(p)
                    .send()
                    .await?;
                all.extend(page.items);
            }
            count_call("comments", 1 + MAX_COMMENT_PAGES as u64);
            tracing::warn!(
                rubato.repo = repo,
                rubato.issue = number,
                pages,
                read = MAX_COMMENT_PAGES,
                "issue comments cut"
            );
            all
        };
        Ok(all
            .into_iter()
            .map(|c| Comment {
                id: c.id.into_inner(),
                author_association: c
                    .author_association
                    .as_ref()
                    .and_then(|a| serde_json::to_value(a).ok())
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "NONE".to_owned()),
                author_is_bot: c.user.r#type == "Bot",
                author: c.user.login,
                body: c.body.unwrap_or_default(),
                created_at: c.created_at,
                url: c.html_url.to_string(),
            })
            .collect())
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "pulls"))]
    async fn pull_for_branch(&self, repo: &str, branch: &str) -> Result<Option<PullInfo>> {
        count_call("pulls", 1);
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
            base: Some(p.base.ref_field.clone()),
            title: p.title.clone().unwrap_or_default(),
            author: p.user.as_ref().map(|u| u.login.clone()).unwrap_or_default(),
        }))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "check-runs"))]
    async fn checks(&self, repo: &str, sha: &str) -> Result<ChecksStatus> {
        count_call("check-runs", 1);
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "pull"))]
    async fn pull(&self, repo: &str, number: u64) -> Result<Option<PullFull>> {
        count_call("pull", 1);
        let (o, r) = split_repo(repo)?;
        let route = format!("/repos/{o}/{r}/pulls/{number}");
        match self
            .client
            .get::<serde_json::Value, _, ()>(route, None::<&()>)
            .await
        {
            Ok(v) => Ok(Some(
                pull_from_json(&v).context("a pull request without a number or head")?,
            )),
            Err(octocrab::Error::GitHub { source, .. }) if source.status_code.as_u16() == 404 => {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "pull_reviews"))]
    async fn pull_reviews(&self, repo: &str, number: u64) -> Result<Vec<ReviewInfo>> {
        count_call("pull_reviews", 1);
        let (o, r) = split_repo(repo)?;
        let route = format!("/repos/{o}/{r}/pulls/{number}/reviews?per_page=100");
        let v: serde_json::Value = self.client.get(route, None::<&()>).await?;
        Ok(reviews_from_page(&v))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "pull_files"))]
    async fn pull_files(
        &self,
        repo: &str,
        number: u64,
    ) -> Result<Vec<rubato_protocol::pulls::ChangedFile>> {
        count_call("pull_files", 1);
        let (o, r) = split_repo(repo)?;
        let route = format!("/repos/{o}/{r}/pulls/{number}/files?per_page={FILES_PER_PAGE}");
        let v: serde_json::Value = self.client.get(route, None::<&()>).await?;
        Ok(files_from_page(&v))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "check-runs"))]
    async fn check_runs(
        &self,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<rubato_protocol::pulls::CheckRun>> {
        count_call("check-runs", 1);
        let (o, r) = split_repo(repo)?;
        let route = format!("/repos/{o}/{r}/commits/{sha}/check-runs?per_page=100");
        let v: serde_json::Value = self.client.get(route, None::<&()>).await?;
        Ok(check_runs_from_page(&v))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self, token), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "repo"))]
    async fn repo_as(&self, token: &str, repo: &str) -> Result<RepoAccess> {
        count_call("repo", 1);
        let (o, r) = split_repo(repo)?;
        let crab = user_client(token, self.base.as_deref())?;
        let v: serde_json::Value = crab.get(format!("/repos/{o}/{r}"), None::<&()>).await?;
        Ok(access_from_repo(&v))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self, token), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "merge"))]
    async fn merge_as(
        &self,
        token: &str,
        repo: &str,
        number: u64,
        sha: &str,
        method: rubato_protocol::pulls::MergeMethod,
    ) -> std::result::Result<String, MergeError> {
        count_call("merge", 1);
        let (o, r) = split_repo(repo).map_err(|e| MergeError::Refused {
            status: 400,
            message: e.to_string(),
        })?;
        let crab = user_client(token, self.base.as_deref())
            .map_err(|e| MergeError::Unknown(format!("{e:#}")))?;
        let body = serde_json::json!({ "sha": sha, "merge_method": method.as_str() });
        match crab
            .put::<serde_json::Value, _, _>(
                format!("/repos/{o}/{r}/pulls/{number}/merge"),
                Some(&body),
            )
            .await
        {
            Ok(v) => Ok(v
                .get("sha")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_owned()),
            Err(octocrab::Error::GitHub { source, .. }) => Err(MergeError::Refused {
                status: source.status_code.as_u16(),
                message: source.message.clone(),
            }),
            Err(e) => Err(MergeError::Unknown(e.to_string())),
        }
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "rate_limit"))]
    async fn rate_limit(&self) -> Result<(u64, u64)> {
        count_call("rate_limit", 1);
        let v: serde_json::Value = self.client.get("/rate_limit", None::<&()>).await?;
        let core = v.pointer("/resources/core").cloned().unwrap_or_default();
        Ok((
            core.get("remaining").and_then(|x| x.as_u64()).unwrap_or(0),
            core.get("limit").and_then(|x| x.as_u64()).unwrap_or(0),
        ))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "issues"))]
    async fn open_issues(&self, repo: &str, max: usize) -> Result<OpenIssues> {
        count_call("issues", 1);
        let (o, r) = split_repo(repo)?;
        let pages = max.div_ceil(ISSUES_PER_PAGE).max(1);
        let mut out = OpenIssues::default();
        for page in 1..=pages {
            let route =
                format!("/repos/{o}/{r}/issues?state=open&per_page={ISSUES_PER_PAGE}&page={page}");
            let v: serde_json::Value = self.client.get(route, None::<&()>).await?;
            let rows = v.as_array().map(Vec::len).unwrap_or(0);
            out.issues.extend(issues_from_page(&v));
            if rows < ISSUES_PER_PAGE {
                return Ok(out);
            }
        }
        // Every page allowed was full: there may be more.
        out.truncated = true;
        out.issues.truncate(max);
        Ok(out)
    }

    fn is_installation(&self) -> bool {
        self.installation
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "installation_repositories"))]
    async fn installation_repos(&self) -> Result<Vec<String>> {
        count_call("installation_repositories", 1);
        let mut out = Vec::new();
        for page in 1..=10 {
            let route = format!("/installation/repositories?per_page=100&page={page}");
            let v: serde_json::Value = self.client.get(route, None::<&()>).await?;
            let repos = v
                .get("repositories")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            out.extend(
                repos
                    .iter()
                    .filter_map(|r| r.get("full_name").and_then(|n| n.as_str()))
                    .map(str::to_owned),
            );
            if repos.len() < 100 {
                break;
            }
        }
        Ok(out)
    }

    fn can_write(&self) -> bool {
        self.installation
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "create_ref"))]
    async fn create_ref(&self, repo: &str, branch: &str, sha: &str) -> Result<()> {
        count_call("create_ref", 1);
        let (o, r) = split_repo(repo)?;
        let reference = octocrab::params::repos::Reference::Branch(branch.to_owned());
        match self.client.repos(o, r).create_ref(&reference, sha).await {
            Ok(_) => Ok(()),
            // 422 "Reference already exists": the branch of a proposal being resumed.
            Err(octocrab::Error::GitHub { source, .. }) if source.status_code.as_u16() == 422 => {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self, content), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "put_contents"))]
    async fn put_file(
        &self,
        repo: &str,
        branch: &str,
        path: &str,
        content: &str,
        message: &str,
    ) -> Result<()> {
        count_call("put_contents", 1);
        let (o, r) = split_repo(repo)?;
        let repos = self.client.repos(o, r);
        // The file's blob sha on that branch: GitHub refuses an update without it, which is
        // what turns a change that landed in between into a conflict and not a silent revert.
        let existing = match repos.get_content().path(path).r#ref(branch).send().await {
            Ok(items) => items.items.into_iter().next().map(|i| i.sha),
            Err(octocrab::Error::GitHub { source, .. }) if source.status_code.as_u16() == 404 => {
                None
            }
            Err(e) => return Err(e.into()),
        };
        match existing {
            Some(sha) => {
                repos
                    .update_file(path, message, content, sha)
                    .branch(branch)
                    .send()
                    .await?;
            }
            None => {
                repos
                    .create_file(path, message, content)
                    .branch(branch)
                    .send()
                    .await?;
            }
        }
        Ok(())
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self, body), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "create_pull"))]
    async fn create_pull(
        &self,
        repo: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String)> {
        count_call("create_pull", 1);
        let (o, r) = split_repo(repo)?;
        let pr = self
            .client
            .pulls(o, r)
            .create(title, head, base)
            .body(body)
            .send()
            .await?;
        Ok((
            pr.number,
            pr.html_url.map(|u| u.to_string()).unwrap_or_default(),
        ))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "close_pull"))]
    async fn close_pull(&self, repo: &str, number: u64) -> Result<()> {
        count_call("close_pull", 1);
        let (o, r) = split_repo(repo)?;
        self.client
            .pulls(o, r)
            .update(number)
            .state(octocrab::params::pulls::State::Closed)
            .send()
            .await?;
        Ok(())
    }

    fn can_create_issues(&self) -> bool {
        self.installation && self.issues_write.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn app_login(&self) -> Option<String> {
        self.app_login.lock().unwrap().clone()
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "installation"))]
    async fn refresh_issue_permission(&self) -> Result<bool> {
        count_call("installation", 1);
        let Some(a) = &self.as_app else {
            return Ok(false);
        };
        let route = format!("/app/installations/{}", a.installation_id);
        let v: serde_json::Value = a.app.get(route, None::<&()>).await?;
        let write = v.pointer("/permissions/issues").and_then(|p| p.as_str()) == Some("write");
        // The App's issues carry `<app_slug>[bot]` as their author (issue #165).
        if let Some(slug) = v.get("app_slug").and_then(|s| s.as_str()) {
            *self.app_login.lock().unwrap() = Some(format!("{slug}[bot]"));
        }
        let was = self
            .issues_write
            .swap(write, std::sync::atomic::Ordering::Relaxed);
        if was != write {
            tracing::info!(
                issues_write = write,
                "installation Issues permission changed"
            );
        }
        Ok(write)
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self, token, body), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "create_comment"))]
    async fn comment_as(
        &self,
        token: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> std::result::Result<PostedComment, PostError> {
        count_call("create_comment", 1);
        let (o, r) = split_repo(repo).map_err(|_| PostError::Refused { status: 400 })?;
        let crab = user_client(token, self.base.as_deref())
            .map_err(|e| PostError::Unknown(format!("{e:#}")))?;
        match crab.issues(o, r).create_comment(number, body).await {
            Ok(c) => Ok(PostedComment {
                id: c.id.into_inner(),
                url: c.html_url.to_string(),
            }),
            Err(octocrab::Error::GitHub { source, .. }) => Err(PostError::Refused {
                status: source.status_code.as_u16(),
            }),
            Err(e) => Err(PostError::Unknown(e.to_string())),
        }
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self, body), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "create_issue"))]
    async fn create_issue(&self, repo: &str, title: &str, body: &str) -> Result<(u64, String)> {
        count_call("create_issue", 1);
        let (o, r) = split_repo(repo)?;
        let Some(a) = &self.as_app else {
            anyhow::bail!("this GitHub connection cannot create issues");
        };
        // A plain POST: the answer is read for its number and page only.
        let route = format!("/repos/{o}/{r}/issues");
        let sent = serde_json::json!({"title": title, "body": body});
        match a
            .no_retry
            .post::<_, serde_json::Value>(route, Some(&sent))
            .await
        {
            Ok(v) => Ok((
                v.get("number")
                    .and_then(|n| n.as_u64())
                    .context("the created issue has no number")?,
                v.get("html_url")
                    .and_then(|u| u.as_str())
                    .unwrap_or_default()
                    .to_owned(),
            )),
            Err(octocrab::Error::GitHub { source, .. }) if source.status_code.as_u16() == 403 => {
                self.issues_write
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                Err(anyhow::anyhow!(
                    "GitHub refused the issue (403): {}; the installation needs Issues: write",
                    source.message
                ))
            }
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(otel.kind = "client", peer.service = "github", rubato.github.endpoint = "delete_ref"))]
    async fn delete_ref(&self, repo: &str, branch: &str) -> Result<()> {
        count_call("delete_ref", 1);
        let (o, r) = split_repo(repo)?;
        let reference = octocrab::params::repos::Reference::Branch(branch.to_owned());
        match self.client.repos(o, r).delete_ref(&reference).await {
            Ok(_) => Ok(()),
            Err(octocrab::Error::GitHub { source, .. })
                if matches!(source.status_code.as_u16(), 404 | 422) =>
            {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #163: a repo string reaches REST paths only as a plain `owner/name`. Without the
    /// character check in `split_repo`, `o/r/../..` and friends split and this fails.
    #[test]
    fn split_repo_accepts_only_owner_slash_name() {
        let good = ["m0x6A/rubato", "o/r.js", "o/a_b-c", "O-1/R_2.x"];
        let bad = [
            "", "o", "o/", "/r", "o/r/x", "o/..", "o/.", "../r", "./r", "o/r?x=1", "o/r#1",
            "o/r%2F", "o r/x", "o/r\n", "o\\r/x", "ö/r",
        ];
        for repo in good {
            assert!(split_repo(repo).is_ok(), "{repo:?} should be accepted");
        }
        for repo in bad {
            assert!(split_repo(repo).is_err(), "{repo:?} should be refused");
        }
        assert_eq!(split_repo("m0x6A/rubato").unwrap(), ("m0x6A", "rubato"));
        println!("checked {} repo strings", good.len() + bad.len());
    }

    /// A listener that accepts and never answers: the call must end as an error, not hang.
    #[tokio::test(flavor = "current_thread")]
    async fn hung_github_call_ends_with_an_error() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let mut open = Vec::new();
            loop {
                let (sock, _) = listener.accept().await.unwrap();
                open.push(sock);
            }
        });
        let read = Duration::from_millis(500);
        let gh = Octo::with_base_uri("t", &format!("http://{addr}"), read).unwrap();
        let (events, _guard) = crate::kernel::telemetry::capture::start();
        let started = std::time::Instant::now();
        // (1 + 3 retries) × read timeout, plus a margin.
        let r = tokio::time::timeout(read * 4 + Duration::from_secs(10), gh.issue("o/r", 1))
            .await
            .expect("the call hung past its timeouts");
        assert!(r.is_err(), "{r:?}");
        println!("hung call ended after {:?}", started.elapsed());
        let events = events.lock().unwrap();
        assert!(
            events.iter().any(|e| e.level == tracing::Level::ERROR),
            "the closed span records the error: {events:?}"
        );
        held.abort();
    }

    /// ADR 0025: the client the issue POST goes through does not retry, so a create that does
    /// not answer is sent once. The same hung listener takes four read timeouts with retries.
    #[tokio::test(flavor = "current_thread")]
    async fn the_issue_client_sends_once() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = accepted.clone();
        let held = tokio::spawn(async move {
            let mut open = Vec::new();
            loop {
                let (sock, _) = listener.accept().await.unwrap();
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                open.push(sock);
            }
        });
        let app = GitHubApp {
            app_id: 12,
            private_key: super::super::app::tests::test_key(),
            installation_id: 34,
        };
        let base = format!("http://{addr}");
        let read = Duration::from_millis(400);
        let once = app_client_with(&app, Some(&base), read, false).unwrap();
        let r: std::result::Result<serde_json::Value, _> = once.get("/app", None::<&()>).await;
        assert!(r.is_err());
        let sent_once = accepted.load(std::sync::atomic::Ordering::SeqCst);
        let retried = app_client_with(&app, Some(&base), read, true).unwrap();
        let r: std::result::Result<serde_json::Value, _> = retried.get("/app", None::<&()>).await;
        assert!(r.is_err());
        let with_retries = accepted.load(std::sync::atomic::Ordering::SeqCst) - sent_once;
        println!("without retries {sent_once} attempt(s), with retries {with_retries}");
        assert_eq!(sent_once, 1, "the no-retry client sends once");
        assert!(
            with_retries > 1,
            "the default client retries: {with_retries}"
        );
        held.abort();
    }

    /// A stand-in for `GET /repos/o/r/issues/1/comments` with `pages` pages of 100 and
    /// GitHub's `Link` header. Returns its base URL and the pages it was asked for.
    async fn comment_pages(pages: u32) -> (String, std::sync::Arc<std::sync::Mutex<Vec<u32>>>) {
        use axum::extract::Query;
        use std::collections::HashMap;
        let asked: std::sync::Arc<std::sync::Mutex<Vec<u32>>> = Default::default();
        let log = asked.clone();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let me = base.clone();
        let router = axum::Router::new().route(
            "/repos/o/r/issues/1/comments",
            axum::routing::get(move |Query(q): Query<HashMap<String, String>>| {
                let log = log.clone();
                let me = me.clone();
                async move {
                    let page: u32 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
                    log.lock().unwrap().push(page);
                    let user = |n: &str| format!("https://x/{n}");
                    let items: Vec<serde_json::Value> = (1..=100u64)
                        .map(|i| {
                            let id = u64::from(page - 1) * 100 + i;
                            serde_json::json!({
                                "id": id, "node_id": "n", "url": user("c"), "html_url": user("c"),
                                "body": format!("comment {id}"), "author_association": "MEMBER",
                                "created_at": "2026-11-15T00:00:00Z",
                                "user": {"login": "m", "id": 1, "node_id": "u", "avatar_url": user("a"),
                                    "gravatar_id": "", "url": user("u"), "html_url": user("u"),
                                    "followers_url": user("f"), "following_url": user("f"),
                                    "gists_url": user("g"), "starred_url": user("s"),
                                    "subscriptions_url": user("s"), "organizations_url": user("o"),
                                    "repos_url": user("r"), "events_url": user("e"),
                                    "received_events_url": user("e"), "type": "User", "site_admin": false}
                            })
                        })
                        .collect();
                    let link = |p: u32, rel: &str| {
                        format!("<{me}/repos/o/r/issues/1/comments?per_page=100&page={p}>; rel=\"{rel}\"")
                    };
                    let mut links = Vec::new();
                    if page < pages {
                        links.push(link(page + 1, "next"));
                        links.push(link(pages, "last"));
                    }
                    let mut res = axum::response::IntoResponse::into_response(axum::Json(items));
                    if !links.is_empty() {
                        res.headers_mut()
                            .insert("link", links.join(", ").parse().unwrap());
                    }
                    res
                }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (base, asked)
    }

    /// Design 193 D6: a long issue costs at most six calls and yields the newest 500 comments.
    #[tokio::test(flavor = "current_thread")]
    async fn issue_comments_read_at_most_five_pages() {
        let (base, asked) = comment_pages(12).await;
        let gh = Octo::with_base_uri("t", &base, Duration::from_secs(5)).unwrap();
        let (events, _guard) = crate::kernel::telemetry::capture::start();
        let got = gh.issue_comments("o/r", 1).await.unwrap();
        assert_eq!(*asked.lock().unwrap(), [1, 8, 9, 10, 11, 12]);
        assert_eq!(got.len(), 500);
        assert_eq!((got[0].id, got[499].id), (701, 1200));
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.message == "issue comments cut" && e.level == tracing::Level::WARN)
        );

        let (base, asked) = comment_pages(3).await;
        let gh = Octo::with_base_uri("t", &base, Duration::from_secs(5)).unwrap();
        let got = gh.issue_comments("o/r", 1).await.unwrap();
        assert_eq!(*asked.lock().unwrap(), [1, 2, 3]);
        assert_eq!(got.len(), 300);
        println!("checked a 12-page and a 3-page issue");
    }

    /// A stand-in for `GET /repos/o/r/git/matching-refs/heads/issue/`. `answer(page)` says
    /// which ref numbers that page holds. Returns its base URL and the pages it was asked for.
    async fn matching_refs(
        answer: fn(u32) -> std::ops::Range<u32>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<u32>>>) {
        use axum::extract::Query;
        use std::collections::HashMap;
        let asked: std::sync::Arc<std::sync::Mutex<Vec<u32>>> = Default::default();
        let log = asked.clone();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = axum::Router::new().route(
            "/repos/o/r/git/matching-refs/heads/issue/",
            axum::routing::get(move |Query(q): Query<HashMap<String, String>>| {
                let log = log.clone();
                async move {
                    let page: u32 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
                    log.lock().unwrap().push(page);
                    let refs: Vec<serde_json::Value> = answer(page)
                        .map(|n| {
                            serde_json::json!({
                                "ref": format!("refs/heads/issue/{n}-x"),
                                "object": {"sha": format!("s{n}")}
                            })
                        })
                        .collect();
                    axum::Json(refs)
                }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (base, asked)
    }

    /// The loop that used the whole hourly budget (2026-11-16): GitHub answers
    /// `git/matching-refs` with every ref whatever page is asked, so with 100 refs or more
    /// "a full page means another" never ended. One listing is now one call there, and never
    /// more than `MAX_REF_PAGES` whatever the other side answers.
    #[tokio::test(flavor = "current_thread")]
    async fn a_branch_listing_ends_whatever_github_answers() {
        let list = |answer: fn(u32) -> std::ops::Range<u32>| async move {
            let (base, asked) = matching_refs(answer).await;
            let gh = Octo::with_base_uri("t", &base, Duration::from_secs(5)).unwrap();
            let heads = gh.branches("o/r", "issue/").await.unwrap();
            let asked = asked.lock().unwrap().clone();
            (heads, asked)
        };
        // As GitHub does: all 144, on every page.
        let (heads, asked) = list(|_| 0..144).await;
        assert_eq!((heads.len(), asked), (144, vec![1]));
        assert_eq!(
            (heads[0].name.as_str(), heads[0].sha.as_str()),
            ("issue/0-x", "s0")
        );
        // Exactly a page's worth, on every page: the second page brings nothing new.
        let (heads, asked) = list(|_| 0..100).await;
        assert_eq!((heads.len(), asked), (100, vec![1, 2]));
        // A side that does page: 250 refs in three pages.
        let (heads, asked) = list(|p| (p - 1) * 100..(p * 100).min(250)).await;
        assert_eq!((heads.len(), asked), (250, vec![1, 2, 3]));
        // A side whose pages never end: cut at the limit.
        let (heads, asked) = list(|p| (p - 1) * 100..p * 100).await;
        assert_eq!(asked.len(), MAX_REF_PAGES as usize);
        assert_eq!(heads.len(), MAX_REF_PAGES as usize * REFS_PER_PAGE);
        // Fewer than a page.
        let (heads, asked) = list(|_| 0..3).await;
        assert_eq!((heads.len(), asked), (3, vec![1]));
        println!("checked 5 branch listings");
    }

    #[test]
    fn open_issues_skips_pull_requests_and_pages() {
        // The shape GitHub answers with: a pull request carries a `pull_request` key.
        let page = serde_json::json!([
            {"number": 7, "title": "A bug", "html_url": "https://github.com/o/r/issues/7",
             "labels": [{"name": "bug"}, {"name": "p1"}], "updated_at": "2026-11-10T08:00:00Z"},
            {"number": 8, "title": "A pull request", "html_url": "https://github.com/o/r/pull/8",
             "labels": [], "updated_at": "2026-11-10T09:00:00Z", "pull_request": {"url": "x"}},
            {"number": 9, "title": "No labels", "html_url": "https://github.com/o/r/issues/9",
             "updated_at": "2026-11-10T10:00:00Z"}
        ]);
        let issues = issues_from_page(&page);
        assert_eq!(
            issues.iter().map(|i| i.number).collect::<Vec<_>>(),
            [7, 9],
            "the pull request is left out"
        );
        assert_eq!(issues[0].labels, ["bug", "p1"]);
        assert!(issues[1].labels.is_empty());
        assert_eq!(
            issues[0].updated_at.to_rfc3339(),
            "2026-11-10T08:00:00+00:00"
        );
        assert!(issues_from_page(&serde_json::json!({"message": "Not Found"})).is_empty());
        println!("checked 3 rows of one page");
    }

    /// ADR 0026: the body and the author's association come from the same listing page, at no
    /// extra call; a long body is cut at a character boundary.
    #[test]
    fn open_issues_carry_body_and_author_association() {
        let long = format!("{}é{}", "a".repeat(BODY_EXCERPT_BYTES - 1), "b".repeat(100));
        let page = serde_json::json!([
            {"number": 1, "title": "T", "html_url": "u", "author_association": "MEMBER",
             "user": {"login": "m0x6A", "type": "User"}, "body": "Map: seam web-server"},
            {"number": 2, "title": "T", "html_url": "u", "body": null},
            {"number": 3, "title": "T", "html_url": "u", "author_association": "NONE", "body": long},
            {"number": 4, "title": "T", "html_url": "u", "author_association": "NONE",
             "user": {"login": "rubato[bot]", "type": "Bot"}, "body": "Rubato-Request: r-1"}
        ]);
        let issues = issues_from_page(&page);
        assert_eq!(issues[0].body, "Map: seam web-server");
        assert_eq!(issues[0].author_association, "MEMBER");
        assert_eq!(issues[0].author, "m0x6A");
        // Issue #165: the Bot flag from `user.type`; no `user` reads as not a Bot.
        assert_eq!(
            issues.iter().map(|i| i.author_is_bot).collect::<Vec<_>>(),
            [false, false, false, true]
        );
        assert_eq!(issues[1].body, "");
        assert_eq!(issues[1].author_association, "");
        assert_eq!(
            issues[2].body.len(),
            BODY_EXCERPT_BYTES - 1,
            "cut before the é"
        );
        assert!(issues[2].body.chars().all(|c| c == 'a'));
    }

    /// The shapes GitHub answers, as in its REST docs: a fork's head repo differs from the
    /// base, a deleted head repo is null, pending reviews have no `submitted_at`.
    #[test]
    fn pull_detail_pages_parse() {
        use serde_json::json;
        let pull = |head_repo: serde_json::Value| {
            json!({"number": 5, "state": "open", "draft": false, "title": "T", "body": "B",
              "html_url": "https://github.com/o/r/pull/5", "mergeable": null,
              "user": {"login": "ann"}, "updated_at": "2026-11-15T10:00:00Z",
              "head": {"ref": "issue/7-x", "sha": "h1", "repo": head_repo},
              "base": {"ref": "main", "sha": "b1", "repo": {"full_name": "O/R"}}})
        };
        let p = pull_from_json(&pull(json!({"full_name": "o/r"}))).unwrap();
        assert_eq!(
            (p.info.number, p.branch.as_str(), p.base_sha.as_str()),
            (5, "issue/7-x", "b1")
        );
        assert!(p.info.open && !p.fork && p.mergeable.is_none());
        assert_eq!(p.info.base.as_deref(), Some("main"));
        assert!(
            pull_from_json(&pull(json!({"full_name": "x/r"})))
                .unwrap()
                .fork
        );
        assert!(pull_from_json(&pull(serde_json::Value::Null)).unwrap().fork);

        let reviews = reviews_from_page(&json!([
            {"user": {"login": "m0x6A", "type": "User"}, "author_association": "OWNER", "state": "APPROVED", "submitted_at": "2026-11-15T10:00:00Z"},
            {"user": {"login": "bot", "type": "Bot"}, "author_association": "NONE", "state": "COMMENTED", "submitted_at": "2026-11-15T10:01:00Z"},
            {"user": {"login": "m0x6A"}, "state": "PENDING"}
        ]));
        assert_eq!(reviews.len(), 2);
        assert!(reviews[1].author_is_bot);

        let files = files_from_page(&json!([{"filename": "a.rs", "additions": 3, "deletions": 1}]));
        assert_eq!(files[0].additions, 3);
        let runs = check_runs_from_page(&json!({"check_runs": [
            {"name": "ci", "status": "completed", "conclusion": "success", "html_url": "u"}]}));
        assert_eq!(runs[0].conclusion.as_deref(), Some("success"));

        use rubato_protocol::pulls::MergeMethod;
        let a = access_from_repo(&json!({"permissions": {"push": true},
            "allow_merge_commit": true, "allow_squash_merge": true, "allow_rebase_merge": false}));
        assert!(a.push);
        assert_eq!(a.methods, [MergeMethod::Merge, MergeMethod::Squash]);
        assert!(!access_from_repo(&json!({})).push);
        println!("checked 5 page shapes");
    }

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

    /// The `GITHUB_CALL` span attributes in `src` that lack the client kind or the peer: the
    /// line numbers. The service graph draws `server-github` from these two (plan 287).
    fn call_sites_lacking_kind_or_peer(src: &str) -> (usize, Vec<usize>) {
        use rubato_protocol::otel::semconv::{GITHUB_PEER, OTEL_KIND, PEER_SERVICE};
        let kind = format!("{OTEL_KIND} = \"client\"");
        let peer = format!("{PEER_SERVICE} = \"{GITHUB_PEER}\"");
        let sites: Vec<(usize, &str)> = src
            .lines()
            .enumerate()
            .filter(|(_, l)| {
                l.trim_start().starts_with("#[tracing::instrument(")
                    && l.contains("name = GITHUB_CALL")
            })
            .collect();
        let lacking = sites
            .iter()
            .filter(|(_, l)| !l.contains(&kind) || !l.contains(&peer))
            .map(|(i, _)| i + 1)
            .collect();
        (sites.len(), lacking)
    }

    #[test]
    fn every_github_call_span_is_a_client_span_that_names_its_peer() {
        let src = include_str!("api.rs");
        let (sites, lacking) = call_sites_lacking_kind_or_peer(src);
        assert!(
            sites >= 28,
            "found only {sites} call sites; the pattern is wrong"
        );
        assert_eq!(
            lacking,
            Vec::<usize>::new(),
            "api.rs lines without kind or peer"
        );
        // Every instrumented function in this file is a GitHub call: none escapes the pattern.
        let instrumented = src
            .lines()
            .filter(|l| l.trim_start().starts_with("#[tracing::instrument("))
            .count();
        assert_eq!(
            instrumented, sites,
            "an instrumented function that is not a GITHUB_CALL"
        );
        println!("checked {sites} GitHub call sites");
    }

    /// must fail: a site with the kind and no peer, and one with neither, are both reported.
    #[test]
    fn a_call_site_without_the_peer_or_the_kind_is_reported() {
        let src = concat!(
            "#[tracing::instrument(name = GITHUB_CALL, fields(otel.kind = \"client\", peer.service = \"github\"))]\n",
            "#[tracing::instrument(name = GITHUB_CALL, fields(otel.kind = \"client\"))]\n",
            "#[tracing::instrument(name = GITHUB_CALL, skip(self))]\n",
        );
        assert_eq!(call_sites_lacking_kind_or_peer(src), (3, vec![2, 3]));
    }
}
