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

/// One page of open pull requests per repo per tick (issue #49).
pub const PULLS_PER_PAGE: usize = 100;

#[async_trait]
pub trait GitHubApi: Send + Sync {
    /// File content at a ref, or None when absent.
    async fn file(&self, repo: &str, path: &str, git_ref: &str) -> Result<Option<String>>;
    /// Commit sha a branch points at.
    async fn branch_sha(&self, repo: &str, branch: &str) -> Result<Option<String>>;
    /// Branches starting with `prefix`, with their head commits.
    async fn branches(&self, repo: &str, prefix: &str) -> Result<Vec<BranchHead>>;
    /// The repo's open pull requests from its own branches (issue #49).
    async fn open_pulls(&self, repo: &str) -> Result<OpenPulls>;
    async fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueInfo>>;
    async fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<Comment>>;
    /// The newest PR whose head is `branch`.
    async fn pull_for_branch(&self, repo: &str, branch: &str) -> Result<Option<PullInfo>>;
    async fn checks(&self, repo: &str, sha: &str) -> Result<ChecksStatus>;
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
    /// Open an issue: its number and page. Sent once, with no retry: an issue POST is not
    /// idempotent. A 403 clears [`GitHubApi::can_create_issues`] until the next refresh.
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
            })
        })
        .collect()
}

fn split_repo(repo: &str) -> Result<(&str, &str)> {
    repo.split_once('/').context("repo must be owner/name")
}

// ----- octocrab ------------------------------------------------------------------------------

pub struct Octo {
    client: octocrab::Octocrab,
    /// Acting as a GitHub App installation, not with a personal token.
    installation: bool,
    /// As an installation only: the App itself (a JWT), to read the installation's permissions,
    /// and an installation client with retries off, for the issue POST (ADR 0025).
    as_app: Option<AsApp>,
    /// The installation's Issues permission is `write`, as last read.
    issues_write: std::sync::atomic::AtomicBool,
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
            installation: false,
            as_app: None,
            issues_write: Default::default(),
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
            installation: true,
            as_app: Some(AsApp {
                app: crab,
                no_retry,
                installation_id: app.installation_id,
            }),
            issues_write: Default::default(),
        })
    }
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
    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "contents"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "branch"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "branches"))]
    async fn branches(&self, repo: &str, prefix: &str) -> Result<Vec<BranchHead>> {
        // `git/matching-refs` filters by prefix on GitHub's side, so the pages count only the
        // matching refs, not every branch of the repo (ADR 0032).
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
        let mut heads = Vec::new();
        let mut pages = 0;
        for page in 1.. {
            let refs: Vec<Ref> = self
                .client
                .get(
                    format!("/repos/{repo}/git/matching-refs/heads/{prefix}"),
                    Some(&[("per_page", "100".to_owned()), ("page", page.to_string())]),
                )
                .await?;
            pages += 1;
            let full = refs.len() >= 100;
            heads.extend(refs.into_iter().filter_map(|r| {
                r.name.strip_prefix("refs/heads/").map(|n| BranchHead {
                    name: n.to_owned(),
                    sha: r.object.sha,
                })
            }));
            if !full {
                break;
            }
        }
        count_call("branches", pages);
        Ok(heads)
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "pulls"))]
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
                        },
                    )
                })
                .collect();
        Ok(OpenPulls { pulls, truncated })
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "issue"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "comments"))]
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
        count_call("comments", (all.len() as u64).div_ceil(100).max(1));
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "pulls"))]
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
        }))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "check-runs"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "rate_limit"))]
    async fn rate_limit(&self) -> Result<(u64, u64)> {
        count_call("rate_limit", 1);
        let v: serde_json::Value = self.client.get("/rate_limit", None::<&()>).await?;
        let core = v.pointer("/resources/core").cloned().unwrap_or_default();
        Ok((
            core.get("remaining").and_then(|x| x.as_u64()).unwrap_or(0),
            core.get("limit").and_then(|x| x.as_u64()).unwrap_or(0),
        ))
    }

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "issues"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "installation_repositories"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "create_ref"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self, content), err, fields(rubato.github.endpoint = "put_contents"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self, body), err, fields(rubato.github.endpoint = "create_pull"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "close_pull"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "installation"))]
    async fn refresh_issue_permission(&self) -> Result<bool> {
        count_call("installation", 1);
        let Some(a) = &self.as_app else {
            return Ok(false);
        };
        let route = format!("/app/installations/{}", a.installation_id);
        let v: serde_json::Value = a.app.get(route, None::<&()>).await?;
        let write = v.pointer("/permissions/issues").and_then(|p| p.as_str()) == Some("write");
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self, body), err, fields(rubato.github.endpoint = "create_issue"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "delete_ref"))]
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
             "user": {"login": "m0x6A"}, "body": "Map: seam web-server"},
            {"number": 2, "title": "T", "html_url": "u", "body": null},
            {"number": 3, "title": "T", "html_url": "u", "author_association": "NONE", "body": long}
        ]);
        let issues = issues_from_page(&page);
        assert_eq!(issues[0].body, "Map: seam web-server");
        assert_eq!(issues[0].author_association, "MEMBER");
        assert_eq!(issues[0].author, "m0x6A");
        assert_eq!(issues[1].body, "");
        assert_eq!(issues[1].author_association, "");
        assert_eq!(
            issues[2].body.len(),
            BODY_EXCERPT_BYTES - 1,
            "cut before the é"
        );
        assert!(issues[2].body.chars().all(|c| c == 'a'));
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
}
