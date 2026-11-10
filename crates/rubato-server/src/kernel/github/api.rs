//! The GitHub port (seam `server-github`) and its octocrab adapter. Every call is a
//! `rubato.github.call` span that records its error; the client has connect and read timeouts
//! so a hung call ends as an error (design D15).

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
        Ok(Self { client: b.build()? })
    }
}

#[async_trait]
impl GitHubApi for Octo {
    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "contents"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "branch"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "branches"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "issue"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "pulls"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "check-runs"))]
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

    #[tracing::instrument(name = GITHUB_CALL, skip(self), err, fields(rubato.github.endpoint = "rate_limit"))]
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
